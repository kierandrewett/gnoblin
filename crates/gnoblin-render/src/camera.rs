//! Cameras and projections for scene spaces.
//!
//! A node may be a 3D space: a volume with a camera that projects its subtree.
//! An orthographic camera is the identity projection a flat scene already uses,
//! so the 2D path is the orthographic path rather than a special case. A
//! perspective camera places the space's content in front of an eye on the
//! space's own axis, centred on the space's bounds, with the eye distance chosen
//! so that the content at depth zero fills the space's height exactly.
//!
//! A camera is validated when the node is created. Only a field of view outside
//! `(0, 180)` degrees, planes that are not `0 < near < far`, or a perspective
//! space with no height is refused; every value that can be drawn is accepted.

use std::fmt;

use gnoblin_types::RectF;

use crate::matrix::{Mat4, fit_distance};

/// Why a camera cannot be honoured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CameraError {
    /// The vertical field of view is not finite, or outside `(0, 180)` degrees.
    FieldOfView {
        /// The rejected value, in degrees.
        degrees: f32,
    },
    /// The planes are not finite, or are not `0 < near < far`.
    Planes {
        /// The rejected near plane.
        near: f32,
        /// The rejected far plane.
        far: f32,
    },
    /// A perspective space needs a positive height to fit its field of view.
    NoHeight,
}

impl fmt::Display for CameraError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FieldOfView { degrees } => {
                write!(
                    formatter,
                    "perspective field of view {degrees} is outside (0, 180) degrees"
                )
            }
            Self::Planes { near, far } => {
                write!(formatter, "perspective planes {near}..{far} are not 0 < near < far")
            }
            Self::NoHeight => write!(formatter, "a perspective space needs a positive height"),
        }
    }
}

impl std::error::Error for CameraError {}

/// How a space's camera maps its subtree onto the output plane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Projection {
    /// Parallel projection, where depth does not change a node's size on screen.
    Orthographic,
    /// Perspective projection, where a node's screen size falls with depth.
    Perspective {
        /// Vertical field of view in degrees, strictly between 0 and 180.
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
    /// The orthographic camera, and the identity of a flat scene.
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

    /// Refuse a camera whose values cannot be projected.
    ///
    /// `bounds` is the space's own rectangle, which a perspective camera centres
    /// on and fits to its height.
    pub fn validate(&self, bounds: RectF) -> Result<(), CameraError> {
        match self.projection {
            Projection::Orthographic => Ok(()),
            Projection::Perspective {
                vertical_field_of_view_degrees: degrees,
                near_plane: near,
                far_plane: far,
            } => {
                if !(degrees.is_finite() && degrees > 0.0 && degrees < 180.0) {
                    return Err(CameraError::FieldOfView { degrees });
                }
                if !(near.is_finite() && far.is_finite() && near > 0.0 && near < far) {
                    return Err(CameraError::Planes { near, far });
                }
                if !(bounds.height.is_finite() && bounds.height > 0.0) {
                    return Err(CameraError::NoHeight);
                }
                Ok(())
            }
        }
    }

    /// The projective matrix applied between a space's content and its own
    /// transform.
    ///
    /// The content plane is the space's local plane, so a child's depth `z` is
    /// measured from the space's plane, and the eye sits on the z axis at the
    /// distance that fits the space's height. An orthographic camera is the
    /// identity. The camera must already be valid for `bounds`.
    #[must_use]
    pub fn content_matrix(&self, bounds: RectF) -> Mat4 {
        match self.projection {
            Projection::Orthographic => Mat4::IDENTITY,
            Projection::Perspective {
                vertical_field_of_view_degrees: degrees,
                near_plane: near,
                far_plane: far,
            } => {
                let centre_x = bounds.x + bounds.width / 2.0;
                let centre_y = bounds.y + bounds.height / 2.0;
                let distance = fit_distance(bounds.height, f64::from(degrees));
                Mat4::perspective(centre_x, centre_y, distance, f64::from(near), f64::from(far))
            }
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

    fn bounds() -> RectF {
        RectF::new(0.0, 0.0, 800.0, 600.0)
    }

    #[test]
    fn a_valid_perspective_camera_is_accepted() {
        assert_eq!(Camera::perspective(60.0, 0.1, 100.0).validate(bounds()), Ok(()));
        assert_eq!(Camera::default().validate(bounds()), Ok(()));
    }

    #[test]
    fn a_field_of_view_outside_the_open_interval_is_refused() {
        for degrees in [0.0, -10.0, 180.0, 240.0, f32::NAN, f32::INFINITY] {
            let camera = Camera::perspective(degrees, 0.1, 100.0);
            assert!(matches!(
                camera.validate(bounds()),
                Err(CameraError::FieldOfView { .. })
            ));
        }
    }

    #[test]
    fn planes_must_satisfy_zero_below_near_below_far() {
        for (near, far) in [
            (0.0, 10.0),
            (-1.0, 10.0),
            (10.0, 10.0),
            (20.0, 10.0),
            (1.0, f32::INFINITY),
        ] {
            let camera = Camera::perspective(60.0, near, far);
            assert!(matches!(camera.validate(bounds()), Err(CameraError::Planes { .. })));
        }
    }

    #[test]
    fn a_perspective_space_needs_a_height() {
        let flat = RectF::new(0.0, 0.0, 800.0, 0.0);
        assert_eq!(
            Camera::perspective(60.0, 0.1, 100.0).validate(flat),
            Err(CameraError::NoHeight)
        );
    }

    #[test]
    fn the_orthographic_content_matrix_is_the_identity() {
        assert_eq!(Camera::ORTHOGRAPHIC.content_matrix(bounds()), Mat4::IDENTITY);
    }
}
