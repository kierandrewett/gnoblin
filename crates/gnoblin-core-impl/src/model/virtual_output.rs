//! `virtual_output`: the description of an output with no display behind it.
//!
//! The description is checked when it is made, and what is wrong with it is named: the field, and what it must be. Handing it to the
//! compositor (`output_collection.add`) is refused with `unsupported` until the compositor serves virtual outputs.

use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Fail, Res, Val};

/// What a virtual output is.
pub(super) struct VirtualOutput {
    name: String,
    width: u64,
    height: u64,
    scale: f64,
    /// The attachment as an enum value, when the output has one.
    attachment: Option<Val>,
}

/// A size of a virtual output: a whole number of pixels above zero.
fn side(value: Option<Val>, which: &str) -> Res<u64> {
    let wrong = || {
        Fail::type_error(format!(
            "VirtualOutput: {which} must be a whole number of pixels above 0"
        ))
    };
    let value = value.ok_or_else(wrong)?;
    match value.uint() {
        Ok(pixels) if pixels >= 1 => Ok(pixels),
        _ => Err(wrong()),
    }
}

/// Reads the description from the properties the constructor was given.
fn describe(cx: &Cx<'_>, inits: &mut gnoblin_object::server::Inits) -> Res<VirtualOutput> {
    let name = match inits.take("name") {
        Some(Val::Text(name)) if !name.is_empty() => name,
        _ => return Err(Fail::type_error("VirtualOutput: name must be a non-empty string")),
    };
    let width = side(inits.take("width"), "width")?;
    let height = side(inits.take("height"), "height")?;
    let scale = match inits.take("scale") {
        None | Some(Val::None) => 1.0,
        Some(value) => match value.double() {
            Ok(scale) if scale.is_finite() && scale > 0.0 => scale,
            _ => return Err(Fail::type_error("VirtualOutput: scale must be a number above 0")),
        },
    };
    let attachment = match inits.take("attachment") {
        None | Some(Val::None) => None,
        Some(value @ Val::Enum { .. }) => Some(value),
        Some(_) => return Err(Fail::type_error("VirtualOutput: attachment must be \"viewer\"")),
    };
    let _ = cx;
    Ok(VirtualOutput {
        name,
        width,
        height,
        scale,
        attachment,
    })
}

/// What `output_collection.add` says: no compositor serves a virtual output yet.
pub(super) fn add(args: &[Val]) -> Res<Val> {
    args[0].object()?;
    Err(Fail::unsupported(
        "gnoblin.outputs.add: the compositor does not serve virtual outputs",
    ))
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<VirtualOutput>("virtual_output")
        .ctor(|cx, inits| describe(cx, inits))
        .get("name", |state, _| Ok(Val::Text(state.name.clone())))
        .get("width", |state, _| Ok(Val::Uint(state.width)))
        .get("height", |state, _| Ok(Val::Uint(state.height)))
        .get("scale", |state, _| Ok(Val::Double(state.scale)))
        .get("attachment", |state, _| {
            Ok(state.attachment.clone().unwrap_or(Val::None))
        });
}
