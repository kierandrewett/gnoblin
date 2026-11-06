//! Three-dimensional placement transforms and their invertibility check.

use gnoblin_types::Vec3;

use super::error::{ReplyError, Subject};

/// Largest magnitude accepted for a translation or a size, in logical units.
///
/// Values beyond this are refused rather than clamped, so a layout cannot
/// silently move a window to a place the caller did not ask for.
pub const BOUND: f64 = 1_048_576.0;

/// Relative pivot size below which a matrix counts as singular.
const SINGULAR_RATIO: f64 = 1e-12;

/// A 4x4 matrix stored column-major, as OpenGL and WGSL read it.
pub type Matrix4 = [f64; 16];

/// The transform a placement applies to its window in 3D space.
///
/// `Trs` is the form Lua layouts normally write: translate, then rotate about
/// the axes in degrees (applied X, then Y, then Z), then scale. `Matrix` takes a
/// raw 4x4 for layouts that already hold one. Both forms are checked the same
/// way, so a matrix that cannot be inverted is refused like a zero scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transform3 {
    /// Translation, Euler rotation in degrees and scale.
    Trs {
        /// Where the window's origin sits, in logical units.
        position: Vec3,
        /// Rotation about the X, Y and Z axes, in degrees.
        rotation_degrees: Vec3,
        /// Per-axis scale; zero on any axis makes the transform singular.
        scale: Vec3,
    },
    /// A raw column-major 4x4 matrix.
    Matrix(Matrix4),
}

impl Transform3 {
    /// No movement, no rotation, unit scale.
    pub const IDENTITY: Self = Self::Trs {
        position: Vec3::new(0.0, 0.0, 0.0),
        rotation_degrees: Vec3::new(0.0, 0.0, 0.0),
        scale: Vec3::new(1.0, 1.0, 1.0),
    };

    /// A flat placement at `(x, y)` with no rotation and unit scale.
    pub const fn at(x: f64, y: f64) -> Self {
        Self::Trs {
            position: Vec3::new(x, y, 0.0),
            rotation_degrees: Vec3::new(0.0, 0.0, 0.0),
            scale: Vec3::new(1.0, 1.0, 1.0),
        }
    }

    /// The column-major matrix this transform applies, `T * Rz * Ry * Rx * S`.
    pub fn to_matrix(&self) -> Matrix4 {
        match *self {
            Self::Matrix(matrix) => matrix,
            Self::Trs {
                position,
                rotation_degrees,
                scale,
            } => {
                let translate = translation(position);
                let rotate = mul(
                    &rotation_z(rotation_degrees.z.to_radians()),
                    &mul(
                        &rotation_y(rotation_degrees.y.to_radians()),
                        &rotation_x(rotation_degrees.x.to_radians()),
                    ),
                );
                let scaled = mul(&rotate, &diagonal(scale));
                mul(&translate, &scaled)
            }
        }
    }

    /// Checks finiteness, translation bounds and invertibility.
    pub fn check(&self, subject: Subject) -> Result<(), ReplyError> {
        match *self {
            Self::Trs {
                position,
                rotation_degrees,
                scale,
            } => {
                finite(subject, "position", position)?;
                finite(subject, "rotation", rotation_degrees)?;
                finite(subject, "scale", scale)?;
                within_bound(subject, "position", position)?;
            }
            Self::Matrix(matrix) => {
                if matrix.iter().any(|value| !value.is_finite()) {
                    return Err(ReplyError::NonFinite {
                        subject,
                        field: "matrix",
                    });
                }
                within_bound(
                    subject,
                    "matrix translation",
                    Vec3::new(matrix[12], matrix[13], matrix[14]),
                )?;
            }
        }
        if invertible(&self.to_matrix()) {
            Ok(())
        } else {
            Err(ReplyError::NotInvertible(subject))
        }
    }
}

fn finite(subject: Subject, field: &'static str, value: Vec3) -> Result<(), ReplyError> {
    if value.x.is_finite() && value.y.is_finite() && value.z.is_finite() {
        Ok(())
    } else {
        Err(ReplyError::NonFinite { subject, field })
    }
}

fn within_bound(subject: Subject, field: &'static str, value: Vec3) -> Result<(), ReplyError> {
    if value.x.abs() <= BOUND && value.y.abs() <= BOUND && value.z.abs() <= BOUND {
        Ok(())
    } else {
        Err(ReplyError::OutOfBounds { subject, field })
    }
}

/// Gaussian elimination with partial pivoting, relative to the largest entry.
fn invertible(matrix: &Matrix4) -> bool {
    let mut rows = [[0.0_f64; 4]; 4];
    let mut largest = 0.0_f64;
    for (row, cells) in rows.iter_mut().enumerate() {
        for (col, cell) in cells.iter_mut().enumerate() {
            *cell = matrix[col * 4 + row];
            largest = largest.max(cell.abs());
        }
    }
    if largest == 0.0 {
        return false;
    }
    let floor = largest * SINGULAR_RATIO;
    for col in 0..4 {
        let mut pivot = col;
        for row in col + 1..4 {
            if rows[row][col].abs() > rows[pivot][col].abs() {
                pivot = row;
            }
        }
        if rows[pivot][col].abs() <= floor {
            return false;
        }
        rows.swap(col, pivot);
        let pivot_row = rows[col];
        for row in rows.iter_mut().skip(col + 1) {
            let factor = row[col] / pivot_row[col];
            for (cell, pivot_cell) in row[col..].iter_mut().zip(&pivot_row[col..]) {
                *cell -= factor * pivot_cell;
            }
        }
    }
    true
}

fn identity() -> Matrix4 {
    let mut matrix = [0.0; 16];
    matrix[0] = 1.0;
    matrix[5] = 1.0;
    matrix[10] = 1.0;
    matrix[15] = 1.0;
    matrix
}

fn translation(offset: Vec3) -> Matrix4 {
    let mut matrix = identity();
    matrix[12] = offset.x;
    matrix[13] = offset.y;
    matrix[14] = offset.z;
    matrix
}

fn diagonal(scale: Vec3) -> Matrix4 {
    let mut matrix = identity();
    matrix[0] = scale.x;
    matrix[5] = scale.y;
    matrix[10] = scale.z;
    matrix
}

fn rotation_x(radians: f64) -> Matrix4 {
    let (sin, cos) = radians.sin_cos();
    let mut matrix = identity();
    matrix[5] = cos;
    matrix[6] = sin;
    matrix[9] = -sin;
    matrix[10] = cos;
    matrix
}

fn rotation_y(radians: f64) -> Matrix4 {
    let (sin, cos) = radians.sin_cos();
    let mut matrix = identity();
    matrix[0] = cos;
    matrix[2] = -sin;
    matrix[8] = sin;
    matrix[10] = cos;
    matrix
}

fn rotation_z(radians: f64) -> Matrix4 {
    let (sin, cos) = radians.sin_cos();
    let mut matrix = identity();
    matrix[0] = cos;
    matrix[1] = sin;
    matrix[4] = -sin;
    matrix[5] = cos;
    matrix
}

/// Column-major product `a * b`.
fn mul(a: &Matrix4, b: &Matrix4) -> Matrix4 {
    let mut out = [0.0; 16];
    for col in 0..4 {
        for row in 0..4 {
            out[col * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[col * 4 + k]).sum();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_matrix_is_identity() {
        let matrix = Transform3::IDENTITY.to_matrix();
        assert_eq!(matrix, identity());
    }

    #[test]
    fn zero_scale_axis_is_not_invertible() {
        let flat = Transform3::Trs {
            position: Vec3::new(0.0, 0.0, 0.0),
            rotation_degrees: Vec3::new(0.0, 0.0, 0.0),
            scale: Vec3::new(1.0, 0.0, 1.0),
        };
        assert_eq!(
            flat.check(Subject::Camera),
            Err(ReplyError::NotInvertible(Subject::Camera))
        );
    }

    #[test]
    fn singular_raw_matrix_is_refused() {
        let zeros = Transform3::Matrix([0.0; 16]);
        assert_eq!(
            zeros.check(Subject::Camera),
            Err(ReplyError::NotInvertible(Subject::Camera))
        );
    }
}
