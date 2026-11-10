//! The placement of a floating window when no shell publishes one.
//!
//! Until a shell publishes a position, a new window cascades from `(32, 32)` of its work area, moving
//! 32 px further for each window already visible on that output and workspace. The positions wrap after
//! 32 steps, and every position is clamped so the window sits inside the work area. The function is pure
//! so the window manager and the compositor can both call it without depending on each other.

use crate::{Rect, Size};

/// Diagonal step, in logical pixels, of the cascade.
const CASCADE_OFFSET: i32 = 32;

/// Number of distinct cascade positions before the cascade wraps around.
const CASCADE_STEPS: usize = 32;

/// The top-left corner of a window of `window_size` placed as the `index`-th visible window.
///
/// `index` is the number of windows already visible on the output and workspace. The window starts at the
/// work area's origin plus `(32, 32)` and steps 32 px per index, wrapping after 32 steps. The rectangle is
/// then moved as little as possible to sit inside `work_area`; a window larger than the work area is aligned
/// to its origin.
#[must_use]
pub fn cascade_position(index: usize, work_area: Rect, window_size: Size) -> (i32, i32) {
    let step = i32::try_from(index % CASCADE_STEPS).unwrap_or(0) + 1;
    let offset = CASCADE_OFFSET.saturating_mul(step);
    let geometry = Rect::from_size(window_size)
        .translate(work_area.x.saturating_add(offset), work_area.y.saturating_add(offset))
        .clamp_inside(work_area);
    (geometry.x, geometry.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(0, 0, 1920, 1080);
    const WINDOW: Size = Size::new(600, 400);

    // The three positions the window manager's failure-mode tests expect without a shell.
    #[test]
    fn a_first_window_without_a_shell_cascades_from_32_32() {
        assert_eq!(cascade_position(0, AREA, WINDOW), (32, 32));
    }

    #[test]
    fn three_windows_without_a_shell_get_distinct_cascading_positions() {
        let positions: Vec<(i32, i32)> = (0..3).map(|index| cascade_position(index, AREA, WINDOW)).collect();
        assert_eq!(positions, vec![(32, 32), (64, 64), (96, 96)]);
    }

    #[test]
    fn the_cascade_wraps_after_32_steps() {
        let area = Rect::new(0, 0, 4000, 4000);
        assert_eq!(cascade_position(30, area, WINDOW), (992, 992));
        assert_eq!(cascade_position(31, area, WINDOW), (1024, 1024));
        assert_eq!(cascade_position(32, area, WINDOW), (32, 32));
        assert_eq!(cascade_position(33, area, WINDOW), (64, 64));
    }

    #[test]
    fn the_cascade_starts_from_the_work_area_origin() {
        let area = Rect::new(100, 50, 1000, 800);
        assert_eq!(cascade_position(0, area, WINDOW), (132, 82));
        assert_eq!(cascade_position(1, area, WINDOW), (164, 114));
    }

    #[test]
    fn a_position_past_the_work_area_is_clamped_inside_it() {
        // Index 30 offsets the window by 992, past the 800 px wide area: it is pinned to the right and bottom edges.
        let area = Rect::new(0, 0, 800, 600);
        assert_eq!(cascade_position(30, area, WINDOW), (200, 200));
        assert_eq!(cascade_position(0, area, WINDOW), (32, 32));
    }

    #[test]
    fn a_window_larger_than_the_work_area_is_aligned_to_its_origin() {
        let area = Rect::new(10, 20, 500, 300);
        assert_eq!(cascade_position(0, area, WINDOW), (10, 20));
    }
}
