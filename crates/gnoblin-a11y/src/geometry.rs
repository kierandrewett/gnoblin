//! The magnifier's view geometry.
//!
//! [`Point`], [`Size`], [`Rect`] and [`Scale`] are `gnoblin-types`' shared
//! primitives, re-exported here so a consumer of this crate names one path for
//! them. What lives in this module is only the magnifier's own arithmetic: the
//! window it takes out of an output, which is exact integer math at any scale.
//!
//! # This is the projection, not the scene
//!
//! [`Point`] and [`Rect`] describe the layout plane: the projected bounds of an
//! output and the projected anchor of a caret, a focus ring or the pointer. A
//! magnifier view is a two-dimensional area of one output's surface by
//! definition, so nothing here can express a third axis and nothing here has to.
//! `gnoblin-render` projects three-dimensional scene bounds onto this plane and
//! hands this crate the projection, exactly as it hands it an output's bounds
//! today.

/// Points, sizes, rectangles and scale: the shared geometry types, re-exported from `gnoblin-types`.
pub use gnoblin_types::{Point, Rect, Scale, Size};

/// The area of `bounds` a magnifier at `scale` shows.
///
/// The window is `scale`'s logical size of the output, centred on `point` and
/// moved the smallest distance that keeps it inside. The renderer paints this
/// source rectangle over the whole output.
#[must_use]
pub fn magnified(bounds: Rect, point: Point, scale: Scale) -> Rect {
    centred(bounds, point, scale.logical_size(bounds.size()))
}

/// A `size`-sized window into `bounds`, centred on `point` and kept inside.
///
/// This is the whole magnifier: the visible area is a window into one output, so
/// it can never describe anything outside it, not even by one pixel at a corner.
#[must_use]
pub fn centred(bounds: Rect, point: Point, size: Size) -> Rect {
    let width = i64::from(size.width).clamp(1, i64::from(bounds.width).max(1));
    let height = i64::from(size.height).clamp(1, i64::from(bounds.height).max(1));
    Rect::new(
        clamp_origin(
            i64::from(bounds.x),
            i64::from(bounds.width),
            width,
            i64::from(point.x) - width / 2,
        ),
        clamp_origin(
            i64::from(bounds.y),
            i64::from(bounds.height),
            height,
            i64::from(point.y) - height / 2,
        ),
        to_u32(width),
        to_u32(height),
    )
}

/// The origin of a `len`-long span placed at `wanted` inside an `extent`-long
/// span starting at `start`, moved the smallest distance that keeps it in.
fn clamp_origin(start: i64, extent: i64, len: i64, wanted: i64) -> i32 {
    if extent <= 0 {
        return to_i32(start);
    }
    let len = len.clamp(1, extent);
    let last = start.saturating_add(extent.saturating_sub(len));
    to_i32(wanted.clamp(start, last.max(start)))
}

fn to_i32(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(if value.is_negative() { i32::MIN } else { i32::MAX })
}

fn to_u32(value: i64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
