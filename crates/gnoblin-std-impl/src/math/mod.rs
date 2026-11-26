//! The arithmetic of a scene: vectors, rays, planes, poses, and the checks that turn what a shell gives into the numbers the tree
//! stores. This is `data/js/gnoblin-core/math/*` in Rust, held to the same numbers (`scene_math` of the oracle's fixtures), and the
//! classes of `math.toml` over it:
//!
//! - `vector`: points, directions, rays and planes;
//! - `transform`: a pose, and the Euler angles `Rz * Ry * Rx` it keeps as a matrix;
//! - `spatial`: the checks of positions, rotations, scales and transforms, and a quaternion as angles;
//! - `classes`: `geometry`, the functions, and the records they take and give.

mod classes;
pub(crate) mod spatial;
pub(crate) mod transform;
pub(crate) mod vector;

use gnoblin_object::server::Builder;

pub(crate) use classes::{given_of, pose_of, pose_val, triple_val, vec3_of, vec3_val};

/// Implements the classes of the module.
pub(crate) fn register(builder: &mut Builder) {
    classes::register(builder);
}
