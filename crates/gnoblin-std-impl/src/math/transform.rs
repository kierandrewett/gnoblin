//! A pose: a position, a rotation matrix and one scale. `data/js/gnoblin-core/math/transform.ts`, in the same arithmetic.
//!
//! Rotations are Euler angles in radians about X, then Y, then Z: the matrix is `Rz * Ry * Rx`, row-major.

use super::vector::{Flat, Vec3};

/// A row-major 3x3 matrix.
type Mat3 = [f64; 9];

/// Multiplies two row-major 3x3 matrices.
fn product(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [0.0; 9];
    for row in 0..3 {
        for col in 0..3 {
            out[row * 3 + col] = a[row * 3] * b[col] + a[row * 3 + 1] * b[3 + col] + a[row * 3 + 2] * b[6 + col];
        }
    }
    out
}

/// The rotation matrix of Euler angles in radians about X, Y and Z.
pub(crate) fn matrix_of(euler: [f64; 3]) -> Mat3 {
    let [a, b, c] = euler;
    let (sa, ca, sb, cb, sc, cc) = (a.sin(), a.cos(), b.sin(), b.cos(), c.sin(), c.cos());
    [
        cc * cb,
        cc * sb * sa - sc * ca,
        cc * sb * ca + sc * sa,
        sc * cb,
        sc * sb * sa + cc * ca,
        sc * sb * ca - cc * sa,
        -sb,
        cb * sa,
        cb * ca,
    ]
}

/// The Euler angles of a rotation matrix, with zero about Z at a quarter turn about Y.
pub(crate) fn euler_of(m: &Mat3) -> [f64; 3] {
    let b = (-m[6]).clamp(-1.0, 1.0).asin();
    if m[6].abs() < 1.0 - 1e-9 {
        return [m[7].atan2(m[8]), b, m[3].atan2(m[0])];
    }
    if m[6] < 0.0 {
        [m[1].atan2(m[2]), b, 0.0]
    } else {
        [(-m[1]).atan2(-m[2]), b, 0.0]
    }
}

/// A position, a rotation and a uniform scale that can transform points and directions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Pose {
    pub(crate) position: Vec3,
    pub(crate) scale: f64,
    matrix: Mat3,
}

impl Pose {
    /// A pose from a position, Euler angles and a uniform scale.
    pub(crate) fn new(position: Vec3, rotation: [f64; 3], scale: f64) -> Pose {
        Pose {
            position,
            scale,
            matrix: matrix_of(rotation),
        }
    }

    pub(crate) fn identity() -> Pose {
        Pose::new(Vec3::default(), [0.0; 3], 1.0)
    }

    /// The rotation as Euler angles.
    pub(crate) fn rotation(&self) -> [f64; 3] {
        euler_of(&self.matrix)
    }

    /// The pose that places `other` inside this one.
    pub(crate) fn multiply(&self, other: &Pose) -> Pose {
        Pose {
            position: self.transform_point(other.position),
            scale: self.scale * other.scale,
            matrix: product(&self.matrix, &other.matrix),
        }
    }

    pub(crate) fn inverse(&self) -> Pose {
        let m = &self.matrix;
        let transposed = [m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8]];
        let scale = 1.0 / self.scale;
        let mut back = Pose {
            position: Vec3::default(),
            scale,
            matrix: transposed,
        };
        back.position = back.transform_direction(self.position).scale(-scale);
        back
    }

    /// Applies the rotation to a direction, without translating or scaling it.
    pub(crate) fn transform_direction(&self, direction: Vec3) -> Vec3 {
        let m = &self.matrix;
        let Vec3 { x, y, z } = direction;
        Vec3::new(
            m[0] * x + m[1] * y + m[2] * z,
            m[3] * x + m[4] * y + m[5] * z,
            m[6] * x + m[7] * y + m[8] * z,
        )
    }

    pub(crate) fn transform_point(&self, point: Vec3) -> Vec3 {
        self.position.add(self.transform_direction(point.scale(self.scale)))
    }

    /// The plane through the position, facing along local +Z.
    pub(crate) fn plane(&self) -> Flat {
        Flat {
            point: self.position,
            normal: self.transform_direction(Vec3::new(0.0, 0.0, 1.0)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-9
    }

    #[test]
    fn a_pose_composed_with_its_inverse_is_the_identity() {
        let pose = Pose::new(Vec3::new(1.0, 2.0, 3.0), [0.3, -0.4, 0.5], 2.0);
        let round = pose.multiply(&pose.inverse());
        assert!(near(round.position, Vec3::default()));
        assert!((round.scale - 1.0).abs() < 1e-12);
        assert!(near(Vec3::of(&round.rotation()), Vec3::default()));
    }

    #[test]
    fn euler_angles_of_a_matrix_turn_back_into_it() {
        for rotation in [
            [0.1, 0.2, 0.3],
            [3.0, 2.0, 1.0],
            [0.0, std::f64::consts::FRAC_PI_2, 0.0],
            [-0.7, 1.2, -2.5],
        ] {
            let matrix = matrix_of(rotation);
            let again = matrix_of(euler_of(&matrix));
            for (a, b) in matrix.iter().zip(again.iter()) {
                assert!((a - b).abs() < 1e-9, "{rotation:?}");
            }
        }
    }
}
