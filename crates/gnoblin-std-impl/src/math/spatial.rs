//! Positions, rotations, scales and transforms checked and turned into the three numbers the tree stores:
//! `data/js/gnoblin-core/math/spatial.ts`, with the same words for what is refused.
//!
//! A value of the wrong kind never gets here: the object protocol refuses a text where a vector is declared. What is checked is
//! what the type cannot say: a number is finite, a list has three numbers, a quaternion has a length.

use super::vector::Point3;
use crate::error::{Fail, Res};

/// A number that is finite, or a refusal naming `target` and what the number is.
pub(crate) fn finite(target: &str, value: f64, what: &str) -> Res<f64> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(Fail::type_error(format!("{target}: {what} must be a finite number")))
    }
}

/// Three finite numbers from a list of three.
pub(crate) fn vector_from(target: &str, parts: &[f64]) -> Res<[f64; 3]> {
    if let [x, y, z] = parts {
        return Ok([
            finite(target, *x, "every component")?,
            finite(target, *y, "every component")?,
            finite(target, *z, "every component")?,
        ]);
    }
    Err(Fail::type_error(format!(
        "{target}: must be three numbers or {{ x, y, z }}"
    )))
}

/// Three finite numbers from a vector.
pub(crate) fn vector_checked(target: &str, vector: Point3) -> Res<[f64; 3]> {
    Ok([
        finite(target, vector.x, "x")?,
        finite(target, vector.y, "y")?,
        finite(target, vector.z, "z")?,
    ])
}

/// The angles about X, Y and Z, in radians, of a quaternion, for the order the tree uses. A quaternion that is not of unit length is
/// normalised first.
pub(crate) fn euler_of_quaternion(x: f64, y: f64, z: f64, w: f64) -> Res<[f64; 3]> {
    let length = (x * x + y * y + z * z + w * w).sqrt();
    if length == 0.0 {
        return Err(Fail::type_error("a quaternion of length 0 is not a rotation"));
    }
    let (x, y, z, w) = (x / length, y / length, z / length, w / length);
    let pitch = (2.0 * (w * y - z * x)).clamp(-1.0, 1.0).asin();
    Ok([
        (2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y)),
        pitch,
        (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z)),
    ])
}

fn not_angles(target: &str) -> Fail {
    Fail::type_error(format!(
        "{target}: must be three angles, {{ x, y, z }} or a quaternion {{ x, y, z, w }}"
    ))
}

/// Three angles in radians from a list of three.
pub(crate) fn rotation_from_list(target: &str, parts: &[f64]) -> Res<[f64; 3]> {
    vector_from(target, parts).map_err(|_| not_angles(target))
}

/// Three angles in radians from a vector.
pub(crate) fn rotation_from_angles(target: &str, angles: Point3) -> Res<[f64; 3]> {
    vector_checked(target, angles).map_err(|_| not_angles(target))
}

/// Three angles in radians from a quaternion.
pub(crate) fn rotation_from_quaternion(target: &str, [x, y, z, w]: [f64; 4]) -> Res<[f64; 3]> {
    let x = finite(target, x, "x")?;
    let y = finite(target, y, "y")?;
    let z = finite(target, z, "z")?;
    let w = finite(target, w, "w")?;
    euler_of_quaternion(x, y, z, w).map_err(|fail| Fail::new(fail.code, format!("{target}: {}", fail.message)))
}

fn not_scale(target: &str) -> Fail {
    Fail::type_error(format!("{target}: must be a number, three numbers or {{ x, y, z }}"))
}

/// The scale along each axis from one number for every axis.
pub(crate) fn scale_from_number(target: &str, value: f64) -> Res<[f64; 3]> {
    let uniform = finite(target, value, "a uniform scale")?;
    Ok([uniform; 3])
}

/// The scale along each axis from a list of three.
pub(crate) fn scale_from_list(target: &str, parts: &[f64]) -> Res<[f64; 3]> {
    vector_from(target, parts).map_err(|_| not_scale(target))
}

/// The scale along each axis from a vector.
pub(crate) fn scale_from_vector(target: &str, value: Point3) -> Res<[f64; 3]> {
    vector_checked(target, value).map_err(|_| not_scale(target))
}

/// A transform with each part turned into the three numbers the tree stores, or none where nothing was given.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ParsedTransform {
    pub(crate) position: Option<[f64; 3]>,
    pub(crate) rotation: Option<[f64; 3]>,
    pub(crate) scale: Option<[f64; 3]>,
}

/// What a script may give as a transform, each part left out when not given.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Given {
    pub(crate) position: Option<Point3>,
    pub(crate) rotation: Option<Point3>,
    pub(crate) orientation: Option<[f64; 4]>,
    pub(crate) scale: Option<Point3>,
    pub(crate) uniform_scale: Option<f64>,
}

/// Checks the parts of a transform, in the order position, rotation, scale.
pub(crate) fn transform_from(target: &str, given: &Given) -> Res<ParsedTransform> {
    if given.rotation.is_some() && given.orientation.is_some() {
        return Err(Fail::type_error(format!(
            "{target}.orientation: a transform has one rotation, `rotation` or `orientation`"
        )));
    }
    if given.scale.is_some() && given.uniform_scale.is_some() {
        return Err(Fail::type_error(format!(
            "{target}.uniform_scale: a transform has one scale, `scale` or `uniform_scale`"
        )));
    }
    let position = given
        .position
        .map(|value| vector_checked(&format!("{target}.position"), value))
        .transpose()?;
    let place = format!("{target}.rotation");
    let rotation = match (given.rotation, given.orientation) {
        (Some(angles), _) => Some(rotation_from_angles(&place, angles)?),
        (None, Some(quaternion)) => Some(rotation_from_quaternion(&place, quaternion)?),
        (None, None) => None,
    };
    let place = format!("{target}.scale");
    let scale = match (given.scale, given.uniform_scale) {
        (Some(axes), _) => Some(scale_from_vector(&place, axes)?),
        (None, Some(uniform)) => Some(scale_from_number(&place, uniform)?),
        (None, None) => None,
    };
    Ok(ParsedTransform {
        position,
        rotation,
        scale,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quarter_turn_quaternion_is_a_quarter_turn_about_its_axis() {
        let half = std::f64::consts::FRAC_PI_4;
        let [a, b, c] = euler_of_quaternion(half.sin(), 0.0, 0.0, half.cos()).expect("a rotation");
        assert!((a - std::f64::consts::FRAC_PI_2).abs() < 1e-12 && b.abs() < 1e-12 && c.abs() < 1e-12);
        assert!(euler_of_quaternion(0.0, 0.0, 0.0, 0.0).is_err());
    }

    #[test]
    fn a_refusal_names_the_place_it_was_given() {
        let fail = vector_from("Node#7.position", &[1.0, f64::NAN, 3.0]).expect_err("not finite");
        assert_eq!(fail.message, "Node#7.position: every component must be a finite number");
        let fail = rotation_from_list("Node#7.rotation", &[1.0, 2.0]).expect_err("two numbers");
        assert_eq!(
            fail.message,
            "Node#7.rotation: must be three angles, { x, y, z } or a quaternion { x, y, z, w }"
        );
    }

    #[test]
    fn a_part_left_out_stays_none() {
        let given = Given {
            scale: None,
            uniform_scale: Some(2.0),
            position: Some(Point3::new(1.0, 2.0, 3.0)),
            ..Given::default()
        };
        let parsed = transform_from("t", &given).expect("a transform");
        assert_eq!(parsed.position, Some([1.0, 2.0, 3.0]));
        assert_eq!(parsed.rotation, None);
        assert_eq!(parsed.scale, Some([2.0, 2.0, 2.0]));
    }
}
