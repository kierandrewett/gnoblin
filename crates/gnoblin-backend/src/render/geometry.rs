//! The one rule every renderer converts a frame's logical rectangles to pixels with.

use smithay::utils::{Physical, Rectangle};

use gnoblin_types::{Rect, Scale};

/// A logical rectangle in physical pixels.
///
/// Both edges are converted and rounded rather than the origin and the extent, so
/// two rectangles that share an edge still share it after scaling: rounding the
/// extent on its own would leave a one-pixel gap that nothing ever repaints. Every
/// renderer uses it, so a frame's damage lands on the same pixels whichever one is
/// installed.
pub(crate) fn physical_rect(rect: Rect, scale: Scale) -> Rectangle<i32, Physical> {
    if scale.is_identity() {
        return Rectangle::new((rect.x, rect.y).into(), (rect.width as i32, rect.height as i32).into());
    }
    let ratio = scale.as_f64();
    let left = (f64::from(rect.x) * ratio).round() as i32;
    let top = (f64::from(rect.y) * ratio).round() as i32;
    let right = (rect.right() as f64 * ratio).round() as i32;
    let bottom = (rect.bottom() as f64 * ratio).round() as i32;
    Rectangle::new(
        (left, top).into(),
        ((right - left).max(0), (bottom - top).max(0)).into(),
    )
}
