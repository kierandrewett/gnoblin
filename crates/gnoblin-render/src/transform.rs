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
//!   transform: the type would have to know the bounds. [`transform_rect`] and
//!   [`inverse_transform_point`] compose the two, and rendering and hit testing
//!   both go through them so they cannot disagree.
//!
//! Only the 2D subset is accepted ([`Transform::check_2d`]); a depth value is
//! refused by the scene when it is written, so these helpers never see one.

use gnoblin_types::{PointF, RectF, Transform, Vec3};

use crate::color::Color;

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

    /// Whether the transform lies inside the accepted 2D subset.
    #[must_use]
    pub fn is_2d(&self) -> bool {
        self.base.is_2d()
    }

    /// Refuse the first transform member that needs depth support.
    pub fn check_2d(&self) -> Result<(), gnoblin_types::Unsupported3d> {
        self.base.check_2d()
    }
}

impl Default for VisualTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// The four corners of a rectangle, in its own space.
fn corners(rect: RectF) -> [PointF; 4] {
    let right = rect.right();
    let bottom = rect.bottom();
    [
        PointF::new(rect.x, rect.y),
        PointF::new(right, rect.y),
        PointF::new(right, bottom),
        PointF::new(rect.x, bottom),
    ]
}

/// Map a point through a transform about `pivot`.
fn map_point(point: PointF, pivot: Vec3, transform: &Transform) -> PointF {
    debug_assert!(
        transform.is_2d(),
        "the scene refuses a depth transform before it gets here"
    );
    let local = PointF::new(point.x - pivot.x, point.y - pivot.y);
    match transform.apply_point(local) {
        Ok(mapped) => PointF::new(mapped.x + pivot.x, mapped.y + pivot.y),
        Err(_) => point,
    }
}

/// Map a rectangle from a node's local space into its parent's space.
///
/// Scale and rotation are applied about `pivot`, which is the node's own bounds
/// centre, and the translation after, matching a 2D TRS matrix.
#[must_use]
pub(crate) fn transform_rect(rect: RectF, pivot: Vec3, transform: &Transform) -> RectF {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for corner in corners(rect) {
        let mapped = map_point(corner, pivot, transform);
        min_x = min_x.min(mapped.x);
        min_y = min_y.min(mapped.y);
        max_x = max_x.max(mapped.x);
        max_y = max_y.max(mapped.y);
    }
    RectF::new(min_x, min_y, max_x - min_x, max_y - min_y)
}

/// Map a point from a node's parent space back into its local space.
///
/// Returns `None` for a zero scale, where the forward map has no inverse and no
/// local point can be named, or a transform this build refuses.
#[must_use]
pub(crate) fn inverse_transform_point(point: PointF, pivot: Vec3, transform: &Transform) -> Option<PointF> {
    transform.check_2d().ok()?;
    if transform.scale.x == 0.0 || transform.scale.y == 0.0 {
        return None;
    }
    let translated_x = point.x - transform.offset.x - pivot.x;
    let translated_y = point.y - transform.offset.y - pivot.y;
    let (sin, cos) = (-transform.rotation_degrees.z).to_radians().sin_cos();
    let rotated_x = translated_x * cos - translated_y * sin;
    let rotated_y = translated_x * sin + translated_y * cos;
    Some(PointF::new(
        pivot.x + rotated_x / transform.scale.x,
        pivot.y + rotated_y / transform.scale.y,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_round_trips_through_the_pivot_transform_and_its_inverse() {
        let transform = Transform::from_2d(Vec3::new(30.0, -12.0, 0.0), 25.0, Vec3::new(1.5, 0.75, 1.0));
        let pivot = Vec3::new(50.0, 40.0, 0.0);
        let local = PointF::new(13.0, 71.0);
        let parent = map_point(local, pivot, &transform);
        let back = inverse_transform_point(parent, pivot, &transform).expect("invertible");
        assert!((back.x - local.x).abs() < 1e-9);
        assert!((back.y - local.y).abs() < 1e-9);
    }

    #[test]
    fn a_scaled_parent_projects_its_child_relative_to_its_own_centre() {
        let parent_transform = Transform::scaled(2.0, 2.0);
        let parent_bounds = RectF::new(0.0, 0.0, 400.0, 300.0);
        let child = RectF::new(200.0, 150.0, 100.0, 100.0);
        let projected = transform_rect(child, parent_bounds.centre(), &parent_transform);
        assert_eq!(projected, RectF::new(200.0, 150.0, 200.0, 200.0));
    }

    #[test]
    fn a_zero_scale_has_no_inverse() {
        let transform = Transform::scaled(0.0, 1.0);
        assert_eq!(
            inverse_transform_point(PointF::new(1.0, 1.0), Vec3::ZERO, &transform),
            None
        );
    }

    #[test]
    fn a_depth_transform_has_no_inverse_and_is_not_silently_mapped() {
        let transform = Transform {
            depth: 1.0,
            ..Transform::IDENTITY
        };
        assert_eq!(
            inverse_transform_point(PointF::new(1.0, 1.0), Vec3::ZERO, &transform),
            None
        );
    }
}
