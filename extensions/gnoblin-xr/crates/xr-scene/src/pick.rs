//! Ray picking against the planned layers.
//!
//! [`pick`] answers which layer a ray lands on, and where in the View's pixels.
//! Layers are tested in reverse submission order: the last one drawn is the one
//! the user sees, so it takes the ray first. Projection layers are never picked.
//!
//! The ray and the layer poses must be in the same frame. A quad in
//! [`Space::View`](crate::plan::Space::View) is head-relative, so a caller picking
//! against it passes a ray expressed in the head's frame.

use crate::math::{Pose, conjugate, rotate};
use crate::plan::{Layer, Placement};
use crate::tree::NodeId;

/// A ray: an origin and a direction. The direction need not be normalised.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: [f64; 3],
    pub direction: [f64; 3],
}

impl Ray {
    /// The ray a pose points along: from its position, down its own `-Z` axis (the OpenXR forward direction, for a
    /// controller's aim pose and for the head alike).
    pub fn from_pose(pose: &Pose) -> Self {
        Self {
            origin: pose.position,
            direction: rotate(pose.orientation, [0.0, 0.0, -1.0]),
        }
    }
}

/// Where a ray struck a quad.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// The View node the layer draws.
    pub id: NodeId,
    /// The window node the View shows: where pointer input over this hit goes.
    pub source: NodeId,
    /// The pixel column within the View, from `0` at the left to `extent[0]`.
    pub u: f64,
    /// The pixel row within the View, from `0` at the top to `extent[1]`.
    pub v: f64,
    /// The hit point in the same frame as the ray.
    pub point: [f64; 3],
}

/// The last quad drawn that the ray strikes, if any.
///
/// A quad is one-sided: its front is the +Z side in its own frame, so a ray
/// must come from the front. A quad behind the ray origin is not hit. Layers
/// with `input` false are skipped, so the layer under them can be hit.
pub fn pick(layers: &[Layer], ray: &Ray) -> Option<Hit> {
    layers.iter().rev().find_map(|layer| match layer {
        Layer::Quad(placement) if placement.input => hit_quad(placement, ray),
        Layer::Quad(_) | Layer::Projection(_) => None,
    })
}

fn hit_quad(p: &Placement, ray: &Ray) -> Option<Hit> {
    let hit = hit_plane(p, ray)?;
    let [width, height] = p.size;
    let inside = (-width * 0.5..=width * 0.5).contains(&hit.x) && (-height * 0.5..=height * 0.5).contains(&hit.y);
    inside.then(|| hit.into_hit(p))
}

/// The quad `view` names, intersected as an unbounded plane: the pixel the ray lands on even when that is outside the
/// View's rectangle, so `u` and `v` can be negative or past the extent.
///
/// This is the coordinate a drag keeps sending while the pointer is held outside the window it grabbed. Unlike
/// [`pick`], it ignores the layer's `input` flag and every other layer: the caller already chose the quad. A ray
/// parallel to the plane, pointing away from it, or reaching it from behind has no answer, and neither does a `view`
/// that is not a quad in `layers`.
pub fn project(layers: &[Layer], view: NodeId, ray: &Ray) -> Option<Hit> {
    layers.iter().find_map(|layer| match layer {
        Layer::Quad(placement) if placement.id == view => hit_plane(placement, ray).map(|hit| hit.into_hit(placement)),
        Layer::Quad(_) | Layer::Projection(_) => None,
    })
}

/// Where a ray crosses a quad's plane, in the quad's own frame, and in the ray's frame.
struct PlaneHit {
    x: f64,
    y: f64,
    point: [f64; 3],
}

impl PlaneHit {
    fn into_hit(self, p: &Placement) -> Hit {
        let [width, height] = p.size;
        // Pixels run from the top-left corner: u grows right, v grows down.
        let u = (self.x + width * 0.5) / width * p.extent[0];
        let v = (height * 0.5 - self.y) / height * p.extent[1];
        let point = self.point;
        Hit {
            id: p.id,
            source: p.source,
            u,
            v,
            point,
        }
    }
}

fn hit_plane(p: &Placement, ray: &Ray) -> Option<PlaneHit> {
    // Express the ray in the quad's frame, where the quad is the plane z = 0.
    let inverse = conjugate(p.pose.orientation);
    let origin = rotate(inverse, sub(ray.origin, p.pose.position));
    let direction = rotate(inverse, ray.direction);

    // The ray must travel towards the front face. A parallel ray has no hit, and
    // a NaN direction or distance has none either.
    if direction[2].is_nan() || direction[2] >= 0.0 {
        return None;
    }
    let t = -origin[2] / direction[2];
    if t.is_nan() || t <= 0.0 {
        return None;
    }
    Some(PlaneHit {
        x: origin[0] + t * direction[0],
        y: origin[1] + t * direction[1],
        point: [
            ray.origin[0] + t * ray.direction[0],
            ray.origin[1] + t * ray.direction[1],
            ray.origin[2] + t * ray.direction[2],
        ],
    })
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
