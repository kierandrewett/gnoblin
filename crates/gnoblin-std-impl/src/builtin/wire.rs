//! Reading and writing the typed property records (`GnoblinStdOption`) the table carries options, arguments and answers in.
//!
//! Every value is checked against the declaration of its kind, so a layout never sees a value of the wrong type or range.
#![allow(unsafe_code)]

use gnoblin_core_abi::GnoblinStr;
use gnoblin_std_abi::layouts::{ActionDesc, LayoutKindDesc, OptionDesc, ValueDesc, ValueShape};
use gnoblin_std_abi::{
    GnoblinStdOption, STD_VALUE_BOOL, STD_VALUE_ENUM, STD_VALUE_NODE, STD_VALUE_NONE, STD_VALUE_NUMBER, STD_VALUE_TEXT,
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
        ValueShape::Text => "text",
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
        (ValueShape::Text, STD_VALUE_TEXT) => {
            // SAFETY: the caller of the table function guarantees the record's text is readable.
            Ok(Val::Text(unsafe { read_text(&raw.text) }?.to_owned()))
        }
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
        // No action answers text: an option that is text is read at the call that sets it.
        Val::Text(_) => {}
    }
    out
}

/// Checks one named record against the options `descs` of `owner`.
fn decode_option(
    owner: &'static str,
    descs: &'static [OptionDesc],
    raw: &GnoblinStdOption,
) -> Res<(&'static OptionDesc, Val)> {
    // SAFETY: the caller of the table function guarantees the record's name points at `len` readable bytes.
    let name = unsafe { read_text(&raw.name) }?;
    let desc = descs
        .iter()
        .find(|desc| desc.name == name)
        .ok_or_else(|| Fail::no_such(format!("{owner} has no option `{name}`")))?;
    let value = decode_value(owner, name, &desc.value, raw)?;
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
    // SAFETY: forwarded from the caller.
    unsafe { read_set(kind.name, kind.options, raw, len) }
}

/// The options `descs` of `owner` from the records a shell passed: the defaults, with those given set.
///
/// # Safety
///
/// As for [`read_options`].
pub(crate) unsafe fn read_set(
    owner: &'static str,
    descs: &'static [OptionDesc],
    raw: *const GnoblinStdOption,
    len: usize,
) -> Res<OptionSet> {
    let mut options = OptionSet::of(owner, descs);
    if raw.is_null() || len == 0 {
        return Ok(options);
    }
    for at in 0..len {
        // SAFETY: the caller guarantees `len` records at `raw`.
        let record = unsafe { read_ext(raw.add(at), "layout option") }?;
        let (desc, value) = decode_option(owner, descs, &record)?;
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
    decode_option(kind.name, kind.options, &record)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::encode_value;
    use gnoblin_core_abi as abi;
    use gnoblin_std_abi::layouts::{BINDING_KINDS, LAYOUT_KINDS};
    use gnoblin_std_abi::{STD_BUILTIN_ARC, STD_BUILTIN_DWINDLE, STD_BUILTIN_GRID};

    fn record(kind: u32, number: f64, name: &str) -> GnoblinStdOption {
        let mut raw = encode_value(&Val::Num(number), None);
        raw.kind = kind;
        raw.name = GnoblinStr {
            ptr: name.as_ptr(),
            len: name.len(),
        };
        raw
    }

    fn kind(id: u32) -> &'static LayoutKindDesc {
        LAYOUT_KINDS
            .iter()
            .find(|kind| kind.id == id)
            .expect("the kind is declared")
    }

    fn read(id: u32, records: &[GnoblinStdOption]) -> Res<OptionSet> {
        // SAFETY: the records are live locals and their names borrow literals.
        unsafe { read_options(kind(id), records.as_ptr(), records.len()) }
    }

    #[test]
    fn what_is_not_given_has_the_declared_default() {
        let set = read(STD_BUILTIN_DWINDLE, &[]).expect("the defaults");
        assert_eq!(set.num("ratio"), 0.5);
        assert!(set.boolean("smart_resizing"));
        assert_eq!(set.maybe("gap"), None);
    }

    #[test]
    fn an_option_the_table_does_not_name_is_refused_with_no_such_member() {
        let error = read(STD_BUILTIN_ARC, &[record(STD_VALUE_NUMBER, 1.0, "ratio")]).expect_err("no such option");
        assert_eq!(error.code, abi::E_NO_SUCH_MEMBER);
        assert!(error.message.contains("arc has no option `ratio`"), "{}", error.message);
    }

    #[test]
    fn a_value_of_the_wrong_type_or_out_of_range_is_refused_with_type() {
        for (id, raw) in [
            (STD_BUILTIN_ARC, record(STD_VALUE_NUMBER, 0.0, "radius")),
            (STD_BUILTIN_ARC, record(STD_VALUE_NUMBER, f64::NAN, "spacing")),
            (STD_BUILTIN_ARC, record(STD_VALUE_BOOL, 1.0, "spacing")),
            (STD_BUILTIN_GRID, record(STD_VALUE_NUMBER, 2.5, "columns")),
            (STD_BUILTIN_GRID, record(STD_VALUE_NUMBER, -1.0, "columns")),
            (STD_BUILTIN_GRID, record(STD_VALUE_ENUM, 7.0, "fill")),
            (STD_BUILTIN_GRID, record(STD_VALUE_NUMBER, 1.0, "fill")),
            (STD_BUILTIN_DWINDLE, record(STD_VALUE_NONE, 0.0, "ratio")),
        ] {
            let error = read(id, &[raw]).expect_err("a refused value");
            assert_eq!(error.code, abi::E_TYPE, "{}", error.message);
        }
    }

    #[test]
    fn an_optional_option_may_be_absent_and_a_member_of_an_enumeration_is_taken() {
        let set = read(
            STD_BUILTIN_GRID,
            &[record(STD_VALUE_NONE, 0.0, "gap"), record(STD_VALUE_ENUM, 1.0, "fill")],
        )
        .expect("both are allowed");
        assert_eq!(set.maybe("gap"), None);
        assert_eq!(set.member("fill"), 1);
    }

    #[test]
    fn every_declared_option_has_a_default_that_the_checks_accept() {
        for kind in LAYOUT_KINDS {
            let set = OptionSet::defaults(kind);
            for option in kind.options {
                if let Val::Num(number) = set.get(option.name) {
                    assert!(within(&option.value, *number), "{}.{}", kind.name, option.name);
                }
            }
        }
        for kind in BINDING_KINDS {
            assert!(
                matches!(OptionSet::of(kind.name, kind.options).get("trigger"), Val::Text(text) if !text.is_empty())
            );
        }
    }

    #[test]
    fn an_answer_is_written_with_the_kind_of_its_declaration() {
        static MEMBERS: [gnoblin_std_abi::layouts::EnumMember; 1] = [gnoblin_std_abi::layouts::EnumMember {
            name: "a",
            value: 0,
            doc: "",
        }];
        let desc = ValueDesc {
            ty: ValueShape::Enum(&MEMBERS),
            optional: false,
            min: gnoblin_std_abi::layouts::LowerBound::Free,
        };
        assert_eq!(encode_value(&Val::Num(3.0), Some(&desc)).kind, STD_VALUE_ENUM);
        assert_eq!(encode_value(&Val::Num(3.0), None).kind, STD_VALUE_NUMBER);
        assert_eq!(encode_value(&Val::Bool(true), None).flag, 1);
        assert_eq!(encode_value(&Val::None, None).kind, STD_VALUE_NONE);
    }
}
