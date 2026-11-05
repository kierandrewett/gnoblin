//! Homogeneous 4x4 matrices for node transforms and perspective spaces.
//!
//! Every node maps its own coordinates into its parent's space with an affine
//! matrix built from its [`Transform`] about its bounds centre. A 3D space adds
//! a projective matrix between its children and its own transform. Composing
//! the chain from the root gives one matrix per node that maps the node's local
//! plane `(x, y, 0, 1)` to homogeneous output coordinates `(X, Y, Z, W)`, where
//! the screen position is `X / W, Y / W` and the depth is `Z / W`.
//!
//! Matrices are row-major and `f64`. Rotations by a multiple of a quarter turn
//! use exact sine and cosine values, so a right-angle 2D rotation keeps whole
//! pixel edges; a `sin(90°)` of `6e-17` would otherwise move a `floor` or `ceil`.

use gnoblin_types::{Transform, Vec3};

/// A row-major 4x4 matrix; `m[row][column]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4 {
    /// The matrix entries, row by row.
    pub m: [[f64; 4]; 4],
}

impl Mat4 {
    /// The identity matrix.
    pub const IDENTITY: Self = Self {
        m: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };

    /// `self · rhs`: applying the result is applying `rhs` first, then `self`.
    #[must_use]
    pub fn mul(&self, rhs: &Self) -> Self {
        let mut out = [[0.0; 4]; 4];
        for (row, out_row) in out.iter_mut().enumerate() {
            for (column, cell) in out_row.iter_mut().enumerate() {
                *cell = (0..4).map(|k| self.m[row][k] * rhs.m[k][column]).sum();
            }
        }
        Self { m: out }
    }

    /// A translation by `(x, y, z)`.
    #[must_use]
    pub const fn translation(x: f64, y: f64, z: f64) -> Self {
        Self {
            m: [
                [1.0, 0.0, 0.0, x],
                [0.0, 1.0, 0.0, y],
                [0.0, 0.0, 1.0, z],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// A non-uniform scale by `(x, y, z)`.
    #[must_use]
    pub const fn scale(x: f64, y: f64, z: f64) -> Self {
        Self {
            m: [
                [x, 0.0, 0.0, 0.0],
                [0.0, y, 0.0, 0.0],
                [0.0, 0.0, z, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// A rotation about the x axis, in degrees.
    #[must_use]
    pub fn rotation_x(degrees: f64) -> Self {
        let (sin, cos) = sin_cos_degrees(degrees);
        Self {
            m: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, cos, -sin, 0.0],
                [0.0, sin, cos, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// A rotation about the y axis, in degrees.
    #[must_use]
    pub fn rotation_y(degrees: f64) -> Self {
        let (sin, cos) = sin_cos_degrees(degrees);
        Self {
            m: [
                [cos, 0.0, sin, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [-sin, 0.0, cos, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// A rotation about the z axis, in degrees; positive turns x towards y.
    #[must_use]
    pub fn rotation_z(degrees: f64) -> Self {
        let (sin, cos) = sin_cos_degrees(degrees);
        Self {
            m: [
                [cos, -sin, 0.0, 0.0],
                [sin, cos, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// The matrix that places a node's local plane in its parent's space.
    ///
    /// The order is the published one, applied to a point: scale, then rotate
    /// about x, then y, then z, then translate by the offset and the depth. The
    /// whole thing is conjugated by the pivot, so scale and rotation happen about
    /// the pivot. The depth adds to the offset's z: a node's plane sits at
    /// `offset.z + depth` in its parent's space.
    #[must_use]
    pub fn node_local(transform: &Transform, pivot: Vec3) -> Self {
        let rotation = Self::rotation_z(transform.rotation_degrees.z)
            .mul(&Self::rotation_y(transform.rotation_degrees.y))
            .mul(&Self::rotation_x(transform.rotation_degrees.x));
        let scale = Self::scale(transform.scale.x, transform.scale.y, transform.scale.z);
        let place = Self::translation(
            transform.offset.x,
            transform.offset.y,
            transform.offset.z + transform.depth,
        );
        Self::translation(pivot.x, pivot.y, pivot.z)
            .mul(&place)
            .mul(&rotation)
            .mul(&scale)
            .mul(&Self::translation(-pivot.x, -pivot.y, -pivot.z))
    }

    /// The projective matrix of a perspective space, applied to its content.
    ///
    /// A content point `(x, y, z)` is projected onto the plane `z = 0` seen from
    /// an eye `distance` units in front of it on the z axis, centred on
    /// `(centre_x, centre_y)`:
    ///
    /// - `X = distance * x - centre_x * z`, `Y = distance * y - centre_y * z`,
    /// - `W = distance - z`, the eye distance of the point,
    /// - `Z / W` maps the eye distances `near..far` onto `1..-1`, so a nearer
    ///   point has the larger depth. Content at `z > 0` sits nearer the eye, the
    ///   same way a larger `z` is nearer in every space.
    #[must_use]
    pub fn perspective(centre_x: f64, centre_y: f64, distance: f64, near: f64, far: f64) -> Self {
        let a = -(far + near) / (far - near);
        let b = -2.0 * far * near / (far - near);
        Self {
            m: [
                [distance, 0.0, -centre_x, 0.0],
                [0.0, distance, -centre_y, 0.0],
                [0.0, 0.0, a, b - a * distance],
                [0.0, 0.0, -1.0, distance],
            ],
        }
    }

    /// Apply the matrix to a homogeneous point.
    #[must_use]
    pub fn apply(&self, point: [f64; 4]) -> [f64; 4] {
        let mut out = [0.0; 4];
        for (row, cell) in out.iter_mut().enumerate() {
            *cell = (0..4).map(|k| self.m[row][k] * point[k]).sum();
        }
        out
    }

    /// Whether every entry is finite.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.m.iter().flatten().all(|value| value.is_finite())
    }
}

/// Sine and cosine of an angle in degrees, exact at every quarter turn.
#[must_use]
pub fn sin_cos_degrees(degrees: f64) -> (f64, f64) {
    let quarters = (degrees / 90.0).rem_euclid(4.0);
    if quarters == 0.0 {
        (0.0, 1.0)
    } else if quarters == 1.0 {
        (1.0, 0.0)
    } else if quarters == 2.0 {
        (0.0, -1.0)
    } else if quarters == 3.0 {
        (-1.0, 0.0)
    } else {
        degrees.to_radians().sin_cos()
    }
}

/// The eye distance that makes a space of `height` span `fov_degrees` vertically.
#[must_use]
pub fn fit_distance(height: f64, fov_degrees: f64) -> f64 {
    (height / 2.0) / (fov_degrees.to_radians() / 2.0).tan()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_types::Vec3;

    fn apply_xy(matrix: &Mat4, x: f64, y: f64) -> (f64, f64) {
        let [px, py, _, w] = matrix.apply([x, y, 0.0, 1.0]);
        (px / w, py / w)
    }

    #[test]
    fn a_right_angle_rotation_is_exact() {
        assert_eq!(sin_cos_degrees(90.0), (1.0, 0.0));
        assert_eq!(sin_cos_degrees(-90.0), (-1.0, 0.0));
        assert_eq!(sin_cos_degrees(450.0), (1.0, 0.0));
        assert_eq!(apply_xy(&Mat4::rotation_z(90.0), 3.0, 0.0), (0.0, 3.0));
    }

    #[test]
    fn the_node_matrix_matches_a_two_dimensional_trs_about_its_pivot() {
        let transform = Transform {
            offset: Vec3::new(30.0, -12.0, 0.0),
            rotation_degrees: Vec3::new(0.0, 0.0, 25.0),
            scale: Vec3::new(1.5, 0.75, 1.0),
            ..Transform::IDENTITY
        };
        let pivot = Vec3::new(50.0, 40.0, 0.0);
        let matrix = Mat4::node_local(&transform, pivot);
        // Scale, then rotate about the pivot, then translate by the offset.
        let (x, y) = apply_xy(&matrix, 13.0, 71.0);
        let local = (13.0 - 50.0, 71.0 - 40.0);
        let scaled = (local.0 * 1.5, local.1 * 0.75);
        let (sin, cos) = 25.0_f64.to_radians().sin_cos();
        let expected = (
            scaled.0 * cos - scaled.1 * sin + 50.0 + 30.0,
            scaled.0 * sin + scaled.1 * cos + 40.0 - 12.0,
        );
        assert!((x - expected.0).abs() < 1e-9, "{x} vs {}", expected.0);
        assert!((y - expected.1).abs() < 1e-9, "{y} vs {}", expected.1);
    }

    #[test]
    fn a_depth_offset_moves_the_plane_along_z_and_leaves_flat_points_alone() {
        let flat = Mat4::node_local(&Transform::IDENTITY, Vec3::ZERO);
        assert_eq!(flat, Mat4::IDENTITY);
        let raised = Transform {
            offset: Vec3::new(0.0, 0.0, 5.0),
            depth: 2.0,
            ..Transform::IDENTITY
        };
        let [_, _, z, w] = Mat4::node_local(&raised, Vec3::ZERO).apply([1.0, 1.0, 0.0, 1.0]);
        assert_eq!((z, w), (7.0, 1.0));
    }

    #[test]
    fn a_perspective_space_keeps_its_centre_and_enlarges_nearer_content() {
        let distance = 100.0;
        let projection = Mat4::perspective(0.0, 0.0, distance, 1.0, 1000.0);
        // At z = 0 the content is on the screen plane: unchanged.
        assert_eq!(apply_xy(&projection, 40.0, -20.0), (40.0, -20.0));
        // Halfway to the eye, the same content is twice the size.
        let [x, y, _, w] = projection.apply([40.0, -20.0, 50.0, 1.0]);
        assert_eq!((x / w, y / w), (80.0, -40.0));
        assert!((w - 50.0).abs() < 1e-12);
    }

    #[test]
    fn a_nearer_point_has_a_smaller_depth() {
        let projection = Mat4::perspective(0.0, 0.0, 100.0, 1.0, 1000.0);
        let depth = |z: f64| {
            let [_, _, zc, w] = projection.apply([0.0, 0.0, z, 1.0]);
            zc / w
        };
        assert!(depth(50.0) < depth(0.0));
        assert!(depth(0.0) < depth(-50.0));
        // The near and far planes land on the edges of the clip range.
        assert!((depth(100.0 - 1.0) - -1.0).abs() < 1e-9);
        assert!((depth(100.0 - 1000.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_fit_distance_spans_the_field_of_view_at_the_screen_plane() {
        let distance = fit_distance(600.0, 90.0);
        // A 90 degree field spans a half-height of `distance` at the screen plane.
        assert!((distance - 300.0).abs() < 1e-9);
    }
}
