//! Reading and writing the typed property records (`GnoblinStdOption`) the table carries options, arguments and answers in.
//!
//! Every value is checked against the declaration of its kind, so a layout never sees a value of the wrong type or range.
#![allow(unsafe_code)]

use gnoblin_core_abi::GnoblinStr;
use gnoblin_std_abi::layouts::{ActionDesc, LayoutKindDesc, OptionDesc, ValueDesc, ValueShape};
use gnoblin_std_abi::{
    GnoblinStdOption, STD_VALUE_BOOL, STD_VALUE_ENUM, STD_VALUE_NODE, STD_VALUE_NONE, STD_VALUE_NUMBER,
};

use super::{OptionSet, Val, range_text, within};
use crate::error::{Fail, Res};
use crate::table::read_ext;

/// `text` as a string.
///
/// # Safety
///
/// `text` must be empty or point at `len` readable bytes.
pub(crate) unsafe fn read_text(text: &GnoblinStr) -> Res<&str> {
    if text.ptr.is_null() || text.len == 0 {
        return Ok("");
    }
    // SAFETY: the caller guarantees `len` readable bytes at `ptr`.
    let bytes = unsafe { std::slice::from_raw_parts(text.ptr, text.len) };
    std::str::from_utf8(bytes).map_err(|_| Fail::type_error("text must be UTF-8"))
}

fn describe(ty: &ValueShape) -> &'static str {
    match ty {
        ValueShape::Bool => "a boolean",
        ValueShape::Number => "a number",
        ValueShape::Int => "a whole number",
        ValueShape::Enum(_) => "a member of its enumeration",
        ValueShape::Node => "a window",
    }
}

/// Checks `raw` against the declaration `desc` of `owner.name`.
pub(crate) fn decode_value(owner: &str, name: &str, desc: &ValueDesc, raw: &GnoblinStdOption) -> Res<Val> {
    if raw.kind == STD_VALUE_NONE {
        return if desc.optional {
            Ok(Val::None)
        } else {
            Err(Fail::type_error(format!("{owner}.{name} is required")))
        };
    }
    let wrong = || {
        Fail::type_error(format!(
            "{owner}.{name} must be {}{}",
            describe(&desc.ty),
            if desc.optional { " or absent" } else { "" }
        ))
    };
    match (desc.ty, raw.kind) {
        (ValueShape::Bool, STD_VALUE_BOOL) => Ok(Val::Bool(raw.flag != 0)),
        (ValueShape::Number | ValueShape::Int, STD_VALUE_NUMBER) => {
            let number = raw.number;
            let whole = !matches!(desc.ty, ValueShape::Int) || number.fract() == 0.0;
            if number.is_finite() && whole && within(desc, number) {
                Ok(Val::Num(number))
            } else {
                Err(Fail::type_error(format!(
                    "{owner}.{name} must be {}, got {number}",
                    range_text(desc)
                )))
            }
        }
        (ValueShape::Enum(members), STD_VALUE_ENUM) => {
            if members.iter().any(|member| f64::from(member.value) == raw.number) {
                Ok(Val::Num(raw.number))
            } else {
                let names: Vec<&str> = members.iter().map(|member| member.name).collect();
                Err(Fail::type_error(format!(
                    "{owner}.{name} must be one of {}, got {}",
                    names.join(", "),
                    raw.number
                )))
            }
        }
        (ValueShape::Node, STD_VALUE_NODE) if raw.node != 0 => Ok(Val::Node(raw.node)),
        _ => Err(wrong()),
    }
}

/// The record for an answer.
pub(crate) fn encode_value(value: &Val, desc: Option<&ValueDesc>) -> GnoblinStdOption {
    let mut out = GnoblinStdOption {
        size: size_of::<GnoblinStdOption>() as u32,
        kind: STD_VALUE_NONE,
        flag: 0,
        reserved: 0,
        name: GnoblinStr {
            ptr: core::ptr::null(),
            len: 0,
        },
        text: GnoblinStr {
            ptr: core::ptr::null(),
            len: 0,
        },
        number: 0.0,
        node: 0,
    };
    match value {
        Val::None => {}
        Val::Bool(flag) => {
            out.kind = STD_VALUE_BOOL;
            out.flag = u32::from(*flag);
        }
        Val::Num(number) => {
            out.kind = if matches!(desc.map(|desc| desc.ty), Some(ValueShape::Enum(_))) {
                STD_VALUE_ENUM
            } else {
                STD_VALUE_NUMBER
            };
            out.number = *number;
        }
        Val::Node(node) => {
            out.kind = STD_VALUE_NODE;
            out.node = *node;
        }
    }
    out
}

/// Checks one named record against `kind`'s options.
fn decode_option(kind: &LayoutKindDesc, raw: &GnoblinStdOption) -> Res<(&'static OptionDesc, Val)> {
    // SAFETY: the caller of the table function guarantees the record's name points at `len` readable bytes.
    let name = unsafe { read_text(&raw.name) }?;
    let desc = kind
        .option(name)
        .ok_or_else(|| Fail::no_such(format!("the {} layout has no option `{name}`", kind.name)))?;
    let value = decode_value(kind.name, name, &desc.value, raw)?;
    Ok((desc, value))
}

/// The options of a kind from the records a shell passed: the defaults, with those given set.
///
/// # Safety
///
/// `raw` must be null (with `len` zero) or point at `len` records whose first field is their size, each readable for that size.
pub(crate) unsafe fn read_options(
    kind: &'static LayoutKindDesc,
    raw: *const GnoblinStdOption,
    len: usize,
) -> Res<OptionSet> {
    let mut options = OptionSet::defaults(kind);
    if raw.is_null() || len == 0 {
        return Ok(options);
    }
    for at in 0..len {
        // SAFETY: the caller guarantees `len` records at `raw`.
        let record = unsafe { read_ext(raw.add(at), "layout option") }?;
        let (desc, value) = decode_option(kind, &record)?;
        options.put(desc.name, value);
    }
    Ok(options)
}

/// One option of a kind, from a record.
///
/// # Safety
///
/// As for [`read_options`], for one record.
pub(crate) unsafe fn read_one(
    kind: &'static LayoutKindDesc,
    raw: *const GnoblinStdOption,
) -> Res<(&'static OptionDesc, Val)> {
    // SAFETY: forwarded from the caller.
    let record = unsafe { read_ext(raw, "layout option") }?;
    decode_option(kind, &record)
}

/// The arguments of an action in the order the table declares them; an argument not given is absent, and a required one is an error.
///
/// # Safety
///
/// As for [`read_options`].
pub(crate) unsafe fn read_args(
    kind: &LayoutKindDesc,
    action: &ActionDesc,
    raw: *const GnoblinStdOption,
    len: usize,
) -> Res<Vec<Val>> {
    let mut given: Vec<Option<Val>> = vec![None; action.args.len()];
    if !raw.is_null() {
        for at in 0..len {
            // SAFETY: the caller guarantees `len` records at `raw`.
            let record = unsafe { read_ext(raw.add(at), "action argument") }?;
            // SAFETY: the record's name is readable, as above.
            let name = unsafe { read_text(&record.name) }?;
            let Some(index) = action.args.iter().position(|arg| arg.name == name) else {
                return Err(Fail::no_such(format!(
                    "the action {}.{} has no argument `{name}`",
                    kind.name, action.name
                )));
            };
            let owner = format!("{}.{}", kind.name, action.name);
            given[index] = Some(decode_value(&owner, name, &action.args[index].value, &record)?);
        }
    }
    let mut args = Vec::with_capacity(action.args.len());
    for (desc, value) in action.args.iter().zip(given) {
        match value {
            Some(value) => args.push(value),
            None if desc.value.optional => args.push(Val::None),
            None => {
                return Err(Fail::type_error(format!(
                    "the action {}.{} needs the argument `{}`",
                    kind.name, action.name, desc.name
                )));
            }
        }
    }
    Ok(args)
}
