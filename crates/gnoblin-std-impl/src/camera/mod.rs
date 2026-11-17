//! Cameras: what a `Camera3D` and a flat `Camera2D` take.
//!
//! `data/js/gnoblin-std/scene/camera.ts` and `display/camera2d.ts` are the reference. A `Camera3D` is a tree node of class `camera`
//! with a projection and clipping values; its orthographic height (`size`) has no tree property, so the library keeps it. A `Camera2D`
//! is a plain node used flat: its position is `x` and `y`, its zoom is the node's scale (`[zoom, zoom, 1]`) and its roll is the
//! rotation about the view axis (`[0, 0, roll]`), so a transition can animate all three.

use crate::animation::staging::Prop;
use crate::error::{Fail, Res};
use crate::scene::{Kind, write};
use crate::state::State;

/// How a camera maps its scene into an image.
const PROJECTIONS: [&str; 3] = ["orthographic", "perspective", "stereo"];

/// A number that is finite, as `finite()` of the TypeScript scene checks it.
pub(crate) fn finite(label: &str, key: &str, value: f64) -> Res<f64> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(Fail::type_error(format!("{label}.{key}: must be a finite number")))
    }
}

/// A number in a camera's property.
fn number(prop: &Prop, label: &str, key: &str) -> Res<f64> {
    match prop {
        Prop::Number(value) => finite(label, key, *value),
        _ => Err(Fail::type_error(format!("{label}.{key}: must be a finite number"))),
    }
}

/// A number above zero.
fn positive(prop: &Prop, label: &str, key: &str) -> Res<f64> {
    let value = number(prop, label, key)?;
    if value > 0.0 {
        Ok(value)
    } else {
        Err(Fail::type_error(format!("{label}.{key}: must be above 0")))
    }
}

/// A write the camera kinds take of their own, or `None` for a key every node takes.
pub(crate) fn set(st: &mut State, node: u64, kind: Kind, label: &str, key: &str, prop: &Prop) -> Option<Res<()>> {
    match (kind, key) {
        (Kind::Camera3D, "projection") => Some(match prop {
            Prop::Text(text) if PROJECTIONS.contains(&text.as_str()) => write(st, node, label, key, prop.clone()),
            _ => Err(Fail::type_error(format!(
                "{label}.projection: must be \"orthographic\", \"perspective\" or \"stereo\""
            ))),
        }),
        (Kind::Camera3D, "metres" | "near" | "far" | "fov" | "ipd") => {
            Some(number(prop, label, key).and_then(|value| write(st, node, label, key, Prop::Number(value))))
        }
        (Kind::Camera3D, "size") => Some(positive(prop, label, key).map(|value| {
            st.scene.sizes.insert(node, value);
        })),
        (Kind::Camera2D, "zoom") => Some(
            positive(prop, label, key)
                .and_then(|value| write(st, node, label, "scale", Prop::Triple([value, value, 1.0]))),
        ),
        (Kind::Camera2D, "roll") => Some(
            number(prop, label, key)
                .and_then(|value| write(st, node, label, "rotation", Prop::Triple([0.0, 0.0, value]))),
        ),
        _ => None,
    }
}

/// What a camera reports of itself that the tree does not hold.
pub(crate) fn get(st: &State, node: u64, kind: Kind, key: &str) -> Option<Prop> {
    match (kind, key) {
        (Kind::Camera3D, "size") => Some(Prop::Number(st.scene.sizes.get(&node).copied().unwrap_or(1000.0))),
        (Kind::Camera2D, "zoom") => match st.transition_get(node, "scale") {
            Some(Prop::Triple([zoom, _, _])) => Some(Prop::Number(zoom)),
            _ => None,
        },
        (Kind::Camera2D, "roll") => match st.transition_get(node, "rotation") {
            Some(Prop::Triple([_, _, roll])) => Some(Prop::Number(roll)),
            _ => None,
        },
        _ => None,
    }
}
