//! The head pose as the pose of the stereo camera that follows it.
//!
//! The runtime measures the head in tracking space: metres, y up, a unit quaternion. A stereo camera states its eyes
//! from its own position and rotation in node units with the scene's conventions (see [`crate::rig`]). This module
//! is the inverse of the first half of [`crate::rig::eyes`]: it turns a measured head pose into the position and
//! rotation to write on the camera so that the eyes the planner states from that camera are the head's own eyes. It
//! decides nothing about where a camera sits in a room: it converts a measurement.
//!
//! # The conventions
//!
//! - The camera's position is the head's position in the camera's units (`metres` metres each), with y mirrored:
//!   `(x, y, z)` metres is `(x, -y, z) / metres`.
//! - The camera's rotation is the head's orientation as Euler angles in degrees, about X, then Y, then Z, with the
//!   mirror in y reversing the turn about x and about z: a head turn `(a, b, c)` is the camera rotation
//!   `(-a, b, -c)`.
//! - Near a quarter turn of yaw the three angles are not unique. The rotation they state is still exact: the extra
//!   degree of freedom goes into the turn about z.

use crate::math::Pose;

/// The largest coordinate, in node units, a camera pose may carry. A pose past it is refused: it is not a head in a
/// room, and the compositor stores positions in single precision.
pub const MAX_COORDINATE: f64 = 1.0e7;

/// How far from 1 the length of an orientation may be and still be taken for a rotation. The runtime reports unit
/// quaternions; this catches the zero quaternion and a quaternion that is not one.
const NORM_TOLERANCE: f64 = 1.0e-3;

/// The position and rotation to write on a stereo camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraPose {
    /// `x`, `y` and `z` in the camera's units.
    pub position: [f64; 3],
    /// The rotation in degrees about X, then Y, then Z.
    pub rotation_degrees: [f64; 3],
}

/// The camera pose for the head pose `head`, for a camera whose unit is `metres` metres, or `None` when `head` or
/// `metres` cannot be used: a value that is not finite, a `metres` that is not positive, a position past
/// [`MAX_COORDINATE`] in the camera's units, or an orientation whose length is not 1.
#[must_use]
pub fn camera_pose(head: &Pose, metres: f64) -> Option<CameraPose> {
    if !(metres.is_finite() && metres > 0.0) {
        return None;
    }
    if !head.position.iter().chain(&head.orientation).all(|c| c.is_finite()) {
        return None;
    }
    let [x, y, z, w] = head.orientation;
    let norm = (x * x + y * y + z * z + w * w).sqrt();
    if (norm - 1.0).abs() > NORM_TOLERANCE {
        return None;
    }
    let position = [
        head.position[0] / metres,
        -head.position[1] / metres,
        head.position[2] / metres,
    ];
    if !position.iter().all(|c| c.is_finite() && c.abs() <= MAX_COORDINATE) {
        return None;
    }
    let [a, b, c] = euler_degrees_of([x / norm, y / norm, z / norm, w / norm]);
    Some(CameraPose {
        position,
        rotation_degrees: [-a, b, -c],
    })
}

/// The angles in degrees about X, then Y, then Z that [`crate::math::euler_degrees`] turns into the unit quaternion
/// `q`: the rotation matrix is `Rz * Ry * Rx`.
fn euler_degrees_of(q: [f64; 4]) -> [f64; 3] {
    let [x, y, z, w] = q;
    let r00 = 1.0 - 2.0 * (y * y + z * z);
    let r01 = 2.0 * (x * y - z * w);
    let r10 = 2.0 * (x * y + z * w);
    let r11 = 1.0 - 2.0 * (x * x + z * z);
    let r20 = 2.0 * (x * z - y * w);
    let r21 = 2.0 * (y * z + x * w);
    let r22 = 1.0 - 2.0 * (x * x + y * y);
    let b = (-r20).clamp(-1.0, 1.0).asin();
    let (a, c) = if r20.abs() < 1.0 - 1.0e-12 {
        (r21.atan2(r22), r10.atan2(r00))
    } else {
        // The turns about x and z share an axis: put it all in z.
        (0.0, (-r01).atan2(r11))
    };
    [a.to_degrees(), b.to_degrees(), c.to_degrees()]
}
