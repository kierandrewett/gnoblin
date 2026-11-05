//! Placement of new floating windows.
//!
//! `window_management.new_window_placement` offers three modes. All three are
//! deterministic and bounded: free-space scanning walks a fixed grid inside the
//! work area, and every fallback cascades a fixed number of steps. A full work
//! area therefore still produces a rectangle, which is what stops placement from
//! failing to terminate or from leaving a window off screen.
//!
//! Only normal application toplevels use this module. Dialogs and modals are
//! placed with their parent by the window manager.

use gnoblin_types::{Rect, Size};

use crate::geometry::clamp_to_i32;

/// Where a new normal window opens.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PlacementMode {
    /// The first free area of the work area, scanning from the top-left, cascading
    /// when no area fits. This is the compositor default.
    #[default]
    FreeSpace,
    /// Centred on the work area, cascading from the centre when that spot is
    /// already exactly covered.
    Center,
    /// Always steered diagonally from the top-left, past the windows already on
    /// the workspace.
    Cascade,
}

/// Grid step of the free-space scan, in logical pixels.
pub(crate) const SCAN_STEP: i32 = 16;

/// Diagonal step of a cascade, in logical pixels.
pub(crate) const CASCADE_STEP: i32 = 32;

/// Upper bound on free-space candidates examined for one window.
pub(crate) const MAX_SCAN_STEPS: i64 = 20_000;

/// Upper bound on cascade steps for one window.
pub(crate) const MAX_CASCADE_STEPS: i32 = 32;

/// The rectangle for a new window of `size`, avoiding `occupied` where possible.
///
/// A size with an empty axis is raised to one pixel, because a window that covers
/// nothing can never receive input and would fall out of every placement scan.
#[must_use]
pub fn place(mode: PlacementMode, size: Size, work_area: Rect, occupied: &[Rect]) -> Rect {
    let size = size.at_least_one();
    match mode {
        PlacementMode::FreeSpace => free_space(size, work_area, occupied),
        PlacementMode::Center => centered(size, work_area, occupied),
        PlacementMode::Cascade => cascade(
            Rect::from_size(size).translate(work_area.x, work_area.y),
            work_area,
            occupied,
        ),
    }
}

/// Scans the work area on a grid for the first spot that overlaps nothing.
fn free_space(size: Size, work_area: Rect, occupied: &[Rect]) -> Rect {
    let mut steps = 0_i64;
    let mut y = work_area.y;
    while i64::from(y) + i64::from(size.height) <= work_area.bottom() && steps < MAX_SCAN_STEPS {
        let mut x = work_area.x;
        while i64::from(x) + i64::from(size.width) <= work_area.right() && steps < MAX_SCAN_STEPS {
            let candidate = Rect::new(x, y, size.width, size.height);
            if !occupied.iter().any(|other| other.intersects(candidate)) {
                return candidate;
            }
            x = x.saturating_add(SCAN_STEP);
            steps += 1;
        }
        y = y.saturating_add(SCAN_STEP);
    }
    // No free area, or a window larger than the work area: cascade from the top-left.
    cascade(
        Rect::from_size(size).translate(work_area.x, work_area.y),
        work_area,
        occupied,
    )
}

/// Centres the window, cascading from the centre only when the spot is exactly taken.
fn centered(size: Size, work_area: Rect, occupied: &[Rect]) -> Rect {
    // The offset saturates at zero because a window wider than the work area is
    // aligned to the origin below, not centred off the left edge.
    let offset_x = i64::from(work_area.width.saturating_sub(size.width) / 2);
    let offset_y = i64::from(work_area.height.saturating_sub(size.height) / 2);
    let centered = Rect::new(
        clamp_to_i32(i64::from(work_area.x) + offset_x),
        clamp_to_i32(i64::from(work_area.y) + offset_y),
        size.width,
        size.height,
    )
    .clamp_inside(work_area);
    if occupied.contains(&centered) {
        return cascade(centered, work_area, occupied);
    }
    centered
}

/// Steps diagonally from `origin` until the window overlaps nothing, or the bound is hit.
fn cascade(origin: Rect, work_area: Rect, occupied: &[Rect]) -> Rect {
    let mut candidate = origin.clamp_inside(work_area);
    for step in 1..=MAX_CASCADE_STEPS {
        if !occupied.iter().any(|other| other.intersects(candidate)) {
            break;
        }
        let offset = CASCADE_STEP.saturating_mul(step);
        candidate = origin.translate(offset, offset).clamp_inside(work_area);
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK_AREA: Rect = Rect::new(0, 0, 1920, 1080);

    #[test]
    fn the_first_window_opens_at_the_work_area_origin() {
        let size = Size::new(400, 300);
        assert_eq!(
            place(PlacementMode::FreeSpace, size, WORK_AREA, &[]),
            Rect::new(0, 0, 400, 300)
        );
    }

    #[test]
    fn free_space_avoids_existing_windows() {
        let size = Size::new(400, 300);
        let occupied = [Rect::new(0, 0, 400, 300)];
        let placed = place(PlacementMode::FreeSpace, size, WORK_AREA, &occupied);
        assert!(!placed.intersects(occupied[0]));
        assert!(placed.is_inside(WORK_AREA));
    }

    #[test]
    fn a_full_work_area_still_places_the_window() {
        // A single window covering the whole work area leaves no free spot at all.
        let size = Size::new(400, 300);
        let occupied = [WORK_AREA];
        let placed = place(PlacementMode::FreeSpace, size, WORK_AREA, &occupied);
        assert!(placed.is_inside(WORK_AREA));
        assert_eq!(placed.size(), size);
    }

    #[test]
    fn a_window_larger_than_the_work_area_is_aligned_to_it() {
        let size = Size::new(2560, 1440);
        let placed = place(PlacementMode::FreeSpace, size, WORK_AREA, &[]);
        assert_eq!(placed, Rect::new(0, 0, 2560, 1440));
    }

    #[test]
    fn center_centres_on_the_work_area() {
        let size = Size::new(400, 300);
        let placed = place(PlacementMode::Center, size, WORK_AREA, &[]);
        assert_eq!(placed, Rect::new(760, 390, 400, 300));
    }

    #[test]
    fn center_cascades_when_its_spot_is_taken_exactly() {
        let size = Size::new(400, 300);
        let occupied = [Rect::new(760, 390, 400, 300)];
        let placed = place(PlacementMode::Center, size, WORK_AREA, &occupied);
        assert_ne!(placed, occupied[0]);
        assert!(!placed.intersects(occupied[0]));
    }

    #[test]
    fn center_stays_put_when_only_partially_covered() {
        let size = Size::new(400, 300);
        let occupied = [Rect::new(760, 390, 800, 600)];
        assert_eq!(
            place(PlacementMode::Center, size, WORK_AREA, &occupied),
            Rect::new(760, 390, 400, 300)
        );
    }

    #[test]
    fn cascade_steps_past_the_windows_already_placed() {
        let size = Size::new(400, 300);
        let occupied = [Rect::new(0, 0, 400, 300), Rect::new(32, 32, 400, 300)];
        let placed = place(PlacementMode::Cascade, size, WORK_AREA, &occupied);
        // Eleven diagonal steps clear the second window vertically: the overlap in
        // x still exists, but no rows are shared.
        assert_eq!(placed, Rect::new(352, 352, 400, 300));
        assert!(!occupied.iter().any(|other| other.intersects(placed)));
    }

    #[test]
    fn cascade_wraps_inside_the_work_area() {
        let size = Size::new(400, 300);
        let occupied: Vec<Rect> = (0..64)
            .map(|step| Rect::new(0, 0, 400, 300).translate(step, step))
            .collect();
        let placed = place(PlacementMode::Cascade, size, WORK_AREA, &occupied);
        assert!(placed.is_inside(WORK_AREA));
        assert_eq!(placed.size(), size);
    }

    #[test]
    fn placement_is_deterministic_and_offset_aware() {
        let work_area = Rect::new(64, 32, 1856, 1000);
        let size = Size::new(400, 300);
        let first = place(PlacementMode::FreeSpace, size, work_area, &[]);
        assert_eq!(first, Rect::new(64, 32, 400, 300));
        let second = place(PlacementMode::FreeSpace, size, work_area, &[first]);
        assert!(!second.intersects(first));
        assert!(second.is_inside(work_area));
        assert_eq!(second, place(PlacementMode::FreeSpace, size, work_area, &[first]));
    }

    #[test]
    fn a_zero_area_never_yields_an_empty_rectangle() {
        let placed = place(PlacementMode::FreeSpace, Size::new(0, 0), Rect::new(0, 0, 0, 0), &[]);
        assert_eq!((placed.width, placed.height), (1, 1));
    }
}
