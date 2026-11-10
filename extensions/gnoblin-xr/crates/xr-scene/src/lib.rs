//! The pure translation from the Gnoblin node tree to OpenXR layers.
//!
//! `gnoblin-xr` mirrors the node graph from the owner socket. This crate is the
//! part that needs no runtime, no socket and no GPU: it decides which `View`
//! nodes under the node named `room` become layers, where they sit, and in what
//! order. The rules are in `design/node-api-refactor.md`, section 9.
//!
//! - [`tree`]: the mirrored node model ([`TreeView`], [`TreeNode`], [`Class`]).
//! - [`plan`]: [`plan`] turns the `room` subtree into a [`Plan`] of [`Layer`]s.
//! - [`pick`]: [`pick`] finds the layer and the pixel a ray lands on.
//! - [`math`]: [`Pose`] and the quaternion arithmetic both of them use.
//!
//! Everything here is deterministic: the same tree, head pose and options give
//! the same [`Plan`] bit for bit. Nothing iterates a hash map.

pub mod math;
pub mod pick;
pub mod plan;
pub mod tree;

pub use math::{HeadPose, IDENTITY, Pose};
pub use pick::{Hit, Ray, pick};
pub use plan::{Layer, Options, Placement, Plan, SkipReason, Skipped, Space, plan};
pub use tree::{CameraProjection, Class, NodeId, TreeNode, TreeView};
