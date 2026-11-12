//! The eyes of a stereo camera, as the compositor draws them and the headset is told about them.
//!
//! A View that names a stereo camera holds two pictures side by side. Gnoblin draws them from the camera's pose, its
//! eye separation and its frustum (`gnoblin-render-software`, `stereo.rs`), and this module states the same eyes as
//! OpenXR poses and fields of view, so the projection layer declares exactly what was drawn. Nothing is measured:
//! the eyes come from the tree, so the same tree gives the same eyes.
//!
//! # The conventions
//!
//! - The camera's own position and rotation are its pose in the source world, in node units, as the compositor reads
//!   them. The world is the scene's: x grows right, y grows down, z grows towards the viewer, and the camera looks
//!   down negative z. OpenXR's y grows up, so a position `(x, y, z)` is `(x, -y, z)` and a rotation `(a, b, c)` about
//!   the three axes is `(-a, b, -c)`: a mirror in y reverses the turn about x and about z.
//! - The left eye sits half the eye separation to the camera's left and the right eye half to its right, along the
//!   camera's own x axis. The separation is `ipd` metres, which is `ipd / metres` node units; `metres` is the
//!   camera's own scale, and also converts the camera's position to metres.
//! - The frustum is the left eye's. The right eye's is its mirror: `left` and `right` swapped and negated.
//! - A picture is `width` by `height` with the left eye in its left `floor(width / 2)` columns, so an eye's aspect
//!   ratio is that over the height.

use crate::math::{Pose, euler_degrees, rotate};
use crate::plan::{Eye, Fov};
use crate::tree::Rig;

/// The two eyes of a camera at `position` (node units) turned by `rotation_degrees`, drawing a picture of `picture`
/// node units, or `None` when the camera cannot be drawn: a `metres` that is not finite and positive, an `ipd` that is
/// not finite and zero or more, a frustum that is not one, or a pose that is not finite. The compositor draws a blank
/// picture for the same cameras.
#[must_use]
pub fn eyes(
    position: [f64; 3],
    rotation_degrees: [f64; 3],
    metres: f64,
    rig: &Rig,
    picture: [f64; 2],
) -> Option<[Eye; 2]> {
    let finite = position.iter().chain(&rotation_degrees).all(|c| c.is_finite());
    if !(finite && metres.is_finite() && metres > 0.0 && rig.ipd.is_finite() && rig.ipd >= 0.0) {
        return None;
    }
    let left = frustum(rig, picture)?;
    let right = Fov {
        left: -left.right,
        right: -left.left,
        up: left.up,
        down: left.down,
    };

    // The camera's own x axis in the world, and half the separation along it, in node units.
    let half = rig.ipd / metres / 2.0;
    let offset = rotate(euler_degrees(rotation_degrees), [half, 0.0, 0.0]);
    let orientation = euler_degrees([-rotation_degrees[0], rotation_degrees[1], -rotation_degrees[2]]);
    let eye = |sign: f64, fov: Fov| Eye {
        pose: Pose {
            position: [
                (position[0] + sign * offset[0]) * metres,
                -(position[1] + sign * offset[1]) * metres,
                (position[2] + sign * offset[2]) * metres,
            ],
            orientation,
        },
        fov,
    };
    Some([eye(-1.0, left), eye(1.0, right)])
}

/// The left eye's frustum in radians, or `None` when it is not one: an angle that is not finite or reaches 90
/// degrees, `left` not below `right`, `down` not below `up`, or, with no angles, a vertical field of view outside
/// `(0, 180)` degrees or a picture that gives the eye no area.
fn frustum(rig: &Rig, picture: [f64; 2]) -> Option<Fov> {
    let limit = std::f64::consts::FRAC_PI_2;
    let [left, right, up, down] = if rig.angles_degrees.iter().any(|angle| *angle != 0.0) {
        rig.angles_degrees.map(f64::to_radians)
    } else {
        let aspect = (picture[0] / 2.0).floor() / picture[1];
        if !(rig.fov_degrees > 0.0 && rig.fov_degrees < 180.0 && aspect.is_finite() && aspect > 0.0) {
            return None;
        }
        let half_vertical = (rig.fov_degrees.to_radians() / 2.0).tan();
        let horizontal = (aspect * half_vertical).atan();
        let vertical = half_vertical.atan();
        [-horizontal, horizontal, vertical, -vertical]
    };
    let usable = [left, right, up, down]
        .iter()
        .all(|angle| angle.is_finite() && angle.abs() < limit)
        && left < right
        && down < up;
    #[allow(clippy::cast_possible_truncation)]
    usable.then_some(Fov {
        left: left as f32,
        right: right as f32,
        up: up as f32,
        down: down as f32,
    })
}
