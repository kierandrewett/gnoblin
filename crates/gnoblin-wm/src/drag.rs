//! Work-area constraints for interactive move and resize.
//!
//! The input layer turns a pointer gesture into a candidate rectangle; this module
//! keeps that rectangle reachable. `window_management.constrain_drag_to_work_area`
//! chooses between keeping the whole window inside the work area and keeping a
//! strip of at least [`MIN_VISIBLE_STRIP`] pixels on screen, which is Mutter's
//! own rule for a window dragged mostly off the monitor.
//!
//! The same function is used for placement, snapping and output changes, so a
//! window cannot end up in a position that a drag would have refused to produce.

use crate::geometry::Rect;

/// Height and width, in logical pixels, that must stay inside the work area when
/// the drag constraint only requires a visible strip.
pub const MIN_VISIBLE_STRIP: i32 = 75;

/// How far a moved or resized window may leave its work area.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DragConstraint {
    /// The whole window stays inside the work area. This is the default.
    #[default]
    WorkArea,
    /// A strip of at least [`MIN_VISIBLE_STRIP`] pixels stays inside the work area
    /// in each direction, so a window can be pushed partly off screen but never
    /// become unreachable.
    MinimumStrip,
}

/// Moves `rect` into the position the constraint allows, keeping its size.
#[must_use]
pub fn constrain(rect: Rect, work_area: Rect, constraint: DragConstraint) -> Rect {
    match constraint {
        DragConstraint::WorkArea => rect.clamp_inside(&work_area),
        DragConstraint::MinimumStrip => strip(rect, work_area),
    }
}

/// Clamps so that a strip of each axis still overlaps the work area.
fn strip(rect: Rect, work_area: Rect) -> Rect {
    let visible_x = MIN_VISIBLE_STRIP.min(rect.width);
    let visible_y = MIN_VISIBLE_STRIP.min(rect.height);
    // The window may hang off an edge by all of its extent except the strip. Using
    // max/min instead of clamp keeps an oversized window inside instead of panicking
    // when the range would be empty.
    let min_x = work_area.x.saturating_sub(rect.width.saturating_sub(visible_x));
    let max_x = work_area.right().saturating_sub(visible_x);
    let min_y = work_area.y.saturating_sub(rect.height.saturating_sub(visible_y));
    let max_y = work_area.bottom().saturating_sub(visible_y);
    Rect::new(
        rect.x.max(min_x).min(max_x),
        rect.y.max(min_y).min(max_y),
        rect.width,
        rect.height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK_AREA: Rect = Rect::new(0, 0, 1920, 1080);

    #[test]
    fn the_work_area_constraint_keeps_everything_inside() {
        let dragged = Rect::new(-500, 900, 400, 300);
        let constrained = constrain(dragged, WORK_AREA, DragConstraint::WorkArea);
        assert_eq!(constrained, Rect::new(0, 780, 400, 300));
        assert!(constrained.is_inside(&WORK_AREA));
    }

    #[test]
    fn the_work_area_constraint_moves_an_oversized_window_to_the_origin() {
        let dragged = Rect::new(500, 500, 2600, 1500);
        assert_eq!(
            constrain(dragged, WORK_AREA, DragConstraint::WorkArea),
            Rect::new(0, 0, 2600, 1500)
        );
    }

    #[test]
    fn the_strip_constraint_allows_the_window_off_the_left_edge() {
        let dragged = Rect::new(-1000, 0, 400, 300);
        let constrained = constrain(dragged, WORK_AREA, DragConstraint::MinimumStrip);
        // Only the 75 pixel strip is left on screen.
        assert_eq!(constrained.x, -325);
        assert_eq!(constrained.right(), MIN_VISIBLE_STRIP);
    }

    #[test]
    fn the_strip_constraint_stops_at_the_far_edge() {
        let dragged = Rect::new(5000, 5000, 400, 300);
        let constrained = constrain(dragged, WORK_AREA, DragConstraint::MinimumStrip);
        assert_eq!(constrained.x, WORK_AREA.right() - MIN_VISIBLE_STRIP);
        assert_eq!(constrained.y, WORK_AREA.bottom() - MIN_VISIBLE_STRIP);
    }

    #[test]
    fn the_strip_constraint_leaves_an_inside_window_alone() {
        let rect = Rect::new(200, 200, 400, 300);
        assert_eq!(constrain(rect, WORK_AREA, DragConstraint::MinimumStrip), rect);
    }

    #[test]
    fn a_small_window_keeps_all_of_itself_visible() {
        // The strip is the smaller of the window extent and the constant.
        let dragged = Rect::new(-500, -500, 40, 20);
        let constrained = constrain(dragged, WORK_AREA, DragConstraint::MinimumStrip);
        assert_eq!(constrained, Rect::new(0, 0, 40, 20));
    }

    #[test]
    fn an_oversized_window_under_the_strip_constraint_stays_reachable() {
        let dragged = Rect::new(-5000, -5000, 3000, 2000);
        let constrained = constrain(dragged, WORK_AREA, DragConstraint::MinimumStrip);
        assert!(constrained.intersects(&WORK_AREA));
        assert_eq!(constrained.size(), dragged.size());
    }
}
