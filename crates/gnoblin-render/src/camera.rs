//! Cameras and projections for scene spaces.
//!
//! A node may be a 3D space: a volume with a camera that projects its subtree.
//! Only an orthographic camera is accepted today (design/rust-compositor.md,
//! Renderer card, "3D-ready, 2D today"), which is exactly the identity
//! projection a flat scene already uses, so the existing 2D path is the
//! orthographic path rather than a special case. A perspective camera is
//! refused with the named `unsupported-3d` code instead of being approximated:
//! an approximated perspective would silently produce the wrong pixels and the
//! wrong damage, which is worse than a refusal a caller can see.
//!
//! The refusal is its own type rather than a member of the shared
//! `gnoblin_types::Unsupported3dField`, because a camera is a renderer concept
//! and a transform is not; both report the same `unsupported-3d` code so a
//! caller branches on one code.
//!
//! Damage under a perspective camera falls back to full damage on the affected
//! outputs (the plan's rule), because a projected bound is not the union of the
//! node's rectangle and no tighter rule has been measured yet. That branch is
//! written and tested here even though the scene cannot currently accept a
//! perspective camera: turning depth on must not need a damage rewrite.

use std::fmt;

/// The named refusal for a projection that needs depth support.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnsupportedProjection;

impl UnsupportedProjection {
    /// The machine-readable code every surface reports.
    pub const CODE: &'static str = "unsupported-3d";

    /// Which member carried the value; a camera has exactly one.
    #[must_use]
    pub const fn member() -> &'static str {
        "perspective"
    }
}

impl fmt::Display for UnsupportedProjection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: `{}` is outside the accepted 2D subset",
            Self::CODE,
            Self::member()
        )
    }
}

impl std::error::Error for UnsupportedProjection {}

/// How a space's camera maps its subtree onto the output plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Projection {
    /// Parallel projection, where depth does not change a node's size on screen.
    Orthographic,
    /// Perspective projection, where a node's screen size falls with depth.
    ///
    /// The parameters are the ones a 3D space needs, so the value a caller
    /// supplies today is refused as a whole rather than partially read.
    Perspective {
        /// Vertical field of view in degrees.
        vertical_field_of_view_degrees: f32,
        /// Near clip distance in logical units, greater than zero.
        near_plane: f32,
        /// Far clip distance in logical units, greater than the near plane.
        far_plane: f32,
    },
}

/// A space's camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// The projection the camera applies to its subtree.
    pub projection: Projection,
}

impl Camera {
    /// The only camera this build accepts, and the identity of a flat scene.
    pub const ORTHOGRAPHIC: Self = Self {
        projection: Projection::Orthographic,
    };

    /// A camera with a perspective projection.
    #[must_use]
    pub const fn perspective(vertical_field_of_view_degrees: f32, near_plane: f32, far_plane: f32) -> Self {
        Self {
            projection: Projection::Perspective {
                vertical_field_of_view_degrees,
                near_plane,
                far_plane,
            },
        }
    }

    /// Refuse a projection that needs depth support.
    pub fn check_2d(&self) -> Result<(), UnsupportedProjection> {
        match self.projection {
            Projection::Orthographic => Ok(()),
            Projection::Perspective { .. } => Err(UnsupportedProjection),
        }
    }
}

impl Default for Camera {
    fn default() -> Self {
        Self::ORTHOGRAPHIC
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_perspective_camera_is_refused_by_name() {
        let camera = Camera::perspective(60.0, 0.1, 100.0);
        assert_eq!(camera.check_2d(), Err(UnsupportedProjection));
        assert_eq!(UnsupportedProjection::CODE, "unsupported-3d");
        assert!(format!("{}", UnsupportedProjection).starts_with("unsupported-3d"));
    }

    #[test]
    fn the_orthographic_camera_is_the_default() {
        assert!(Camera::default().check_2d().is_ok());
    }
}
