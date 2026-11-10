//! Poses, and the transform arithmetic the planner and the picker share.
//!
//! Orientations are unit quaternions stored as `[x, y, z, w]`. The arithmetic is
//! plain `f64` in a fixed order of operations, so the same input always gives the
//! same bits, on every run.

/// A position in metres and an orientation quaternion `[x, y, z, w]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub position: [f64; 3],
    pub orientation: [f64; 4],
}

/// The head's pose in tracking space. It has the shape of any other [`Pose`].
///
/// [`plan`](crate::plan::plan) accepts it so the call matches the runtime's
/// per-frame call, but it does not read it: world layers and head-locked layers
/// are both independent of the head pose.
pub type HeadPose = Pose;

/// The identity orientation `[0, 0, 0, 1]`.
pub const IDENTITY: [f64; 4] = [0.0, 0.0, 0.0, 1.0];

/// A transform: a translation in the parent's space, a rotation, and a uniform
/// scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Transform {
    pub position: [f64; 3],
    pub rotation: [f64; 4],
    pub scale: f64,
}

impl Transform {
    pub const IDENTITY: Self = Self {
        position: [0.0; 3],
        rotation: IDENTITY,
        scale: 1.0,
    };

    /// `self` with `child` placed inside it: the child's transform expressed in
    /// the space of `self`'s parent.
    pub fn compose(self, child: Self) -> Self {
        let offset = rotate(self.rotation, child.position.map(|c| c * self.scale));
        Self {
            position: [
                self.position[0] + offset[0],
                self.position[1] + offset[1],
                self.position[2] + offset[2],
            ],
            rotation: quat_mul(self.rotation, child.rotation),
            scale: self.scale * child.scale,
        }
    }

    pub fn is_finite(&self) -> bool {
        self.position.iter().all(|c| c.is_finite())
            && self.rotation.iter().all(|c| c.is_finite())
            && self.scale.is_finite()
    }
}

/// The Hamilton product `a * b` of two `[x, y, z, w]` quaternions.
pub(crate) fn quat_mul(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

/// The inverse of a unit quaternion: its conjugate.
pub(crate) fn conjugate(q: [f64; 4]) -> [f64; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

/// Rotates `v` by the unit quaternion `q`.
pub(crate) fn rotate(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    let [qx, qy, qz, qw] = q;
    let u = [qx, qy, qz];
    let t = cross(u, v).map(|c| 2.0 * c);
    let c = cross(u, t);
    [
        v[0] + qw * t[0] + c[0],
        v[1] + qw * t[1] + c[1],
        v[2] + qw * t[2] + c[2],
    ]
}

/// The quaternion for Euler angles in degrees, applied about X, then Y, then Z.
pub(crate) fn euler_degrees(degrees: [f64; 3]) -> [f64; 4] {
    let [x, y, z] = degrees.map(|d| d.to_radians() * 0.5);
    let qx = [x.sin(), 0.0, 0.0, x.cos()];
    let qy = [0.0, y.sin(), 0.0, y.cos()];
    let qz = [0.0, 0.0, z.sin(), z.cos()];
    quat_mul(quat_mul(qz, qy), qx)
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
