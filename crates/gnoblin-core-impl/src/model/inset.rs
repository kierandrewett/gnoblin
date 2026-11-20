//! `insets`: the room a window's frame takes on each side of its content, and the arithmetic on it.
//!
//! The functions are pure: they read two records and answer one, and no desktop is involved.

use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Res, Val};

use super::values::{fields_of, record, rect};

/// The four sides of an inset record, in the order the catalog declares them: top, right, bottom, left.
type Sides = [f64; 4];

/// An inset from its record.
fn inset(cx: &Cx<'_>, value: &Val) -> Res<Sides> {
    let mut sides = [0.0; 4];
    for (index, (_, field)) in fields_of(cx, "frame_inset", value)?.into_iter().enumerate().take(4) {
        sides[index] = if field.is_none() { 0.0 } else { field.double()? };
    }
    Ok(sides)
}

/// A rectangle from its record: x, y, width, height.
fn rectangle(cx: &Cx<'_>, value: &Val) -> Res<[f64; 4]> {
    let mut sides = [0.0; 4];
    for (index, (_, field)) in fields_of(cx, "rect", value)?.into_iter().enumerate().take(4) {
        sides[index] = if field.is_none() { 0.0 } else { field.double()? };
    }
    Ok(sides)
}

/// An inset as a record.
fn inset_value(cx: &Cx<'_>, [top, right, bottom, left]: Sides) -> Res<Val> {
    record(
        cx,
        "frame_inset",
        vec![
            ("top", Val::Double(top)),
            ("right", Val::Double(right)),
            ("bottom", Val::Double(bottom)),
            ("left", Val::Double(left)),
        ],
    )
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<()>("insets")
        .ctor(|_, _| Ok(()))
        .function("none", |cx, _| inset_value(cx, [0.0; 4]))
        .function("is_none", |cx, args| Ok(Val::Bool(inset(cx, &args[0])? == [0.0; 4])))
        .function("same", |cx, args| {
            Ok(Val::Bool(inset(cx, &args[0])? == inset(cx, &args[1])?))
        })
        .function("grow", |cx, args| {
            let [x, y, width, height] = rectangle(cx, &args[0])?;
            let [top, right, bottom, left] = inset(cx, &args[1])?;
            rect(cx, x - left, y - top, width + left + right, height + top + bottom)
        })
        .function("shrink", |cx, args| {
            let [x, y, width, height] = rectangle(cx, &args[0])?;
            let [top, right, bottom, left] = inset(cx, &args[1])?;
            rect(
                cx,
                x + left,
                y + top,
                (width - left - right).max(0.0),
                (height - top - bottom).max(0.0),
            )
        });
}
