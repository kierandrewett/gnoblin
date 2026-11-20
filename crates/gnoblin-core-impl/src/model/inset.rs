//! `insets`: the room a window's frame takes on each side of its content, and the arithmetic on it.
//!
//! The functions are pure: they read two records and answer one, and no desktop is involved.

use gnoblin_object::server::{Builder, Cx};
use gnoblin_object::{Res, Val};

use super::values::{fields_of, record, rect};

/// The four sides of an inset record, in the order the catalog declares them: top, right, bottom, left.
#[derive(Clone, Copy, PartialEq)]
struct Inset {
    top: f64,
    right: f64,
    bottom: f64,
    left: f64,
}

/// An inset from its record.
fn inset(cx: &Cx<'_>, value: &Val) -> Res<Inset> {
    let mut sides = [0.0; 4];
    for (index, (_, field)) in fields_of(cx, "frame_inset", value)?.into_iter().enumerate().take(4) {
        sides[index] = if field.is_none() { 0.0 } else { field.double()? };
    }
    Ok(Inset {
        top: sides[0],
        right: sides[1],
        bottom: sides[2],
        left: sides[3],
    })
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
fn inset_value(cx: &Cx<'_>, inset: Inset) -> Res<Val> {
    record(
        cx,
        "frame_inset",
        vec![
            ("top", Val::Double(inset.top)),
            ("right", Val::Double(inset.right)),
            ("bottom", Val::Double(inset.bottom)),
            ("left", Val::Double(inset.left)),
        ],
    )
}

pub(super) fn register(builder: &mut Builder) {
    let _ = builder
        .class::<()>("insets")
        .ctor(|_, _| Ok(()))
        .function("none", |cx, _| {
            inset_value(
                cx,
                Inset {
                    top: 0.0,
                    right: 0.0,
                    bottom: 0.0,
                    left: 0.0,
                },
            )
        })
        .function("is_none", |cx, args| {
            let inset = inset(cx, &args[0])?;
            Ok(Val::Bool(
                inset.top == 0.0 && inset.right == 0.0 && inset.bottom == 0.0 && inset.left == 0.0,
            ))
        })
        .function("same", |cx, args| {
            Ok(Val::Bool(inset(cx, &args[0])? == inset(cx, &args[1])?))
        })
        .function("grow", |cx, args| {
            let [x, y, width, height] = rectangle(cx, &args[0])?;
            let inset = inset(cx, &args[1])?;
            rect(
                cx,
                x - inset.left,
                y - inset.top,
                width + inset.left + inset.right,
                height + inset.top + inset.bottom,
            )
        })
        .function("shrink", |cx, args| {
            let [x, y, width, height] = rectangle(cx, &args[0])?;
            let inset = inset(cx, &args[1])?;
            rect(
                cx,
                x + inset.left,
                y + inset.top,
                (width - inset.left - inset.right).max(0.0),
                (height - inset.top - inset.bottom).max(0.0),
            )
        });
}
