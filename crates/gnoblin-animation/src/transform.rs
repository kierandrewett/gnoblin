//! The scene's visual transform and the pivot-relative projection helpers.
//!
//! The 3D-ready transform itself lives in `gnoblin-types`, because every crate
//! that names a position or a rotation must use one type. This module adds the
//! two things only the renderer needs:
//!
//! - [`VisualTransform`] pairs that transform with the visual properties the
//!   scene owns (opacity, tint and corner radius), so a node has one value the
//!   renderer reads instead of a parallel property table.
//! - Scale and rotation happen about the node's own bounds centre, which is the
//!   `origin = center` pivot the animation guide documents. That pivot is
//!   relative to the node's own rectangle, so it cannot live in the shared
//!   transform: the type would have to know the bounds. The renderer's
//!   `Mat4::node_local` composes the two, and rendering and hit testing both go
//!   through it so they cannot disagree.
//!
//! Every member is valid in 3D. The scene builds one 4x4 matrix per node from
//! these values and refuses only a result that is not finite
//! or not invertible.

use gnoblin_types::{Color, Transform};

/// A node's transform plus the visual properties the scene layers on top.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisualTransform {
    /// The shared three-axis transform and draw depth.
    pub base: Transform,
    /// Group opacity, 0 to 1.
    pub opacity: f32,
    /// Multiplicative colour, white by default.
    pub tint: Color,
    /// Corner radius in logical pixels.
    pub corner_radius: f32,
}

impl VisualTransform {
    /// The identity transform: no offset, no rotation, unit scale, opaque.
    pub const IDENTITY: Self = Self {
        base: Transform::IDENTITY,
        opacity: 1.0,
        tint: Color::WHITE,
        corner_radius: 0.0,
    };

    /// Whether every value is finite.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.base.is_finite() && self.opacity.is_finite() && self.tint.is_finite() && self.corner_radius.is_finite()
    }
}

impl Default for VisualTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}
