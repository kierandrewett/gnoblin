//! Chooses where an open menu goes, from its anchor and the work area.
//!
//! The rule, in order:
//!
//! 1. Open below the anchor, with its left edge on the anchor's left edge.
//! 2. If that does not fit in the work area, flip above the anchor.
//! 3. Clamp horizontally into the work area.
//! 4. Clamp vertically into the work area.
//!
//! Step 2 flips even when the menu fits on neither side. Step 4 then keeps it
//! on screen. A menu larger than the work area is pinned to the work area's
//! top-left corner, so its first items stay visible. The work area is already
//! the output minus exclusive zones, so the menu never covers a panel.

use gnoblin_types::{Point, Rect, Size};

/// Where a menu opens, and whether it flipped above its anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    /// The top-left corner of the menu, in the same space as the anchor.
    pub origin: Point,
    /// True when the menu opened above the anchor rather than below it.
    pub flipped: bool,
}

/// Places a menu of `menu_size` for `anchor` inside `work_area`.
///
/// `anchor` is the button rectangle for a pointer invocation, or a point's
/// degenerate rectangle for a keyboard one. A zero-height anchor sits on its
/// own edge, so the menu opens right at that point.
///
/// Edges are computed in 64 bits, so an anchor near `i32::MAX` does not wrap.
#[must_use]
pub fn place(anchor: Rect, menu_size: Size, work_area: Rect) -> Placement {
    let menu_width = i64::from(menu_size.width);
    let menu_height = i64::from(menu_size.height);
    let anchor_bottom = i64::from(anchor.y) + i64::from(anchor.height);

    let below = anchor_bottom;
    let fits_below = below + menu_height <= work_area.bottom();
    let (top, flipped) = if fits_below {
        (below, false)
    } else {
        (i64::from(anchor.y) - menu_height, true)
    };

    let left = clamp_axis(
        i64::from(anchor.x),
        menu_width,
        i64::from(work_area.x),
        work_area.right(),
    );
    let top = clamp_axis(top, menu_height, i64::from(work_area.y), work_area.bottom());

    Placement {
        origin: Point::new(narrow(left), narrow(top)),
        flipped,
    }
}

/// Clamps a menu's start so the menu lies in `[lo, hi]` along one axis.
///
/// If the menu is longer than the span, the start is pinned to `lo`, so the
/// leading edge stays visible.
fn clamp_axis(start: i64, extent: i64, lo: i64, hi: i64) -> i64 {
    let latest = hi - extent;
    if latest < lo {
        return lo;
    }
    start.clamp(lo, latest)
}

/// Narrows a clamped coordinate back to `i32`.
///
/// The clamp keeps the value between the work area's edges, which are `i32`
/// values, so the value fits. The saturating fallback is only for the case the
/// argument rules out, where a menu's far edge would exceed `i32::MAX`.
fn narrow(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX })
}
