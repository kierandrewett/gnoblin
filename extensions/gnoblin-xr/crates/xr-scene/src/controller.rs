//! A controller's aim pose as the pose of its `controller` node.
//!
//! The runtime measures a controller in tracking space: metres, y up, a unit quaternion. A `controller` node states the
//! pose the way a View states its own: `x`, `y` and `z` in the room's node units and `rotation` as Euler angles, in the
//! placement frame `crate::pick` casts rays in. So a binding in the shell can move a View with the pose of the hand as it
//! reads it, with no conversion beyond the units. Unlike the head camera's pose, which is mirrored in y to suit a
//! camera's own conventions ([`crate::head`]), a controller's is not.

use crate::head::node_units;
use crate::math::Pose;

/// The position and rotation to write on a `controller` node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedPose {
    /// `x`, `y` and `z` in node units.
    pub position: [f64; 3],
    /// The rotation in degrees about X, then Y, then Z.
    pub rotation_degrees: [f64; 3],
}

/// The node pose for the aim pose `aim`, for a room whose unit is `metres` metres ([`crate::plan::placement_metres`]),
/// or `None` when `aim` or `metres` cannot be used: a value that is not finite, a `metres` that is not positive, a
/// position past the largest coordinate the compositor stores, or an orientation whose length is not 1. Near a quarter
/// turn of yaw the three angles are not unique; the rotation they state is still exact.
#[must_use]
pub fn placed_pose(aim: &Pose, metres: f64) -> Option<PlacedPose> {
    let (position, rotation_degrees) = node_units(aim, metres)?;
    Some(PlacedPose {
        position,
        rotation_degrees,
    })
}
