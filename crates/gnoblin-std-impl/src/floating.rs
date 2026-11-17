//! Where a floating window first goes: free-space, centred and cascading placement.
//!
//! This is `data/js/gnoblin-std/layouts/floating.ts` in Rust, line for line, so the same windows land on the same rectangles in
//! both libraries (`tests/parity.rs` holds them to it). The functions are pure: a rectangle, the area, the rectangles that are
//! taken, the cascade step.

use gnoblin_core::Rect;
use gnoblin_std_abi as abi;

/// The grid the free-space scan steps over, in logical pixels.
const SCAN_STEP: f64 = 16.0;
/// The most grid spots the scan looks at before it falls back to a cascade.
const MAX_SCAN_STEPS: usize = 20_000;
/// The most steps a cascade takes before it settles.
const MAX_CASCADE_STEPS: usize = 32;
/// The size a window opens at before it has committed one.
pub(crate) const DEFAULT_WIDTH: f64 = 800.0;
/// The size a window opens at before it has committed one.
pub(crate) const DEFAULT_HEIGHT: f64 = 600.0;
/// The cascade step when a layout spec leaves it at zero.
pub(crate) const DEFAULT_CASCADE_STEP: f64 = 32.0;

/// How a new window is placed, one per `STD_PLACE_*` constant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlaceMode {
    /// The first spot in the area that overlaps nothing.
    FreeSpace,
    /// Centred.
    Center,
    /// A cascade from the area's top left.
    Cascade,
}

impl PlaceMode {
    /// The mode for a `STD_PLACE_*` constant.
    pub(crate) fn from_abi(placement: u32) -> Option<PlaceMode> {
        match placement {
            abi::STD_PLACE_SMART => Some(PlaceMode::FreeSpace),
            abi::STD_PLACE_CENTER => Some(PlaceMode::Center),
            abi::STD_PLACE_CASCADE => Some(PlaceMode::Cascade),
            _ => None,
        }
    }
}

fn intersects(a: &Rect, b: &Rect) -> bool {
    // An empty rectangle overlaps nothing.
    a.width > 0.0
        && a.height > 0.0
        && b.width > 0.0
        && b.height > 0.0
        && a.x < b.x + b.width
        && b.x < a.x + a.width
        && a.y < b.y + b.height
        && b.y < a.y + a.height
}

fn hits(rect: &Rect, occupied: &[Rect]) -> bool {
    occupied.iter().any(|other| intersects(rect, other))
}

/// Moves `r` the least distance inside `area`. A rectangle at least as large as the area goes to its origin.
fn clamp_inside(r: &Rect, area: &Rect) -> Rect {
    let x = if r.width >= area.width {
        area.x
    } else {
        r.x.max(area.x).min(area.x + area.width - r.width)
    };
    let y = if r.height >= area.height {
        area.y
    } else {
        r.y.max(area.y).min(area.y + area.height - r.height)
    };
    Rect::new(x, y, r.width, r.height)
}

/// Moves `frame` back toward the area's origin, one `step` at a time, until no window already has its top-left corner. `place_rect`
/// pushes a cascade that has run out of room back inside the area, which pins every later window to the area's far corner: the
/// second and the third of three large windows would land exactly on each other. A frame that cannot move any further stays.
pub(crate) fn apart_from(frame: &Rect, occupied: &[Rect], area: &Rect, step: f64) -> Rect {
    let mut c = *frame;
    let mut i = 0;
    while i < MAX_CASCADE_STEPS && occupied.iter().any(|other| other.x == c.x && other.y == c.y) {
        let moved = clamp_inside(&Rect::new(c.x - step, c.y - step, c.width, c.height), area);
        if moved.x == c.x && moved.y == c.y {
            break;
        }
        c = moved;
        i += 1;
    }
    c
}

/// Steps diagonally from `origin` until the window overlaps nothing, or the bound is hit.
fn cascade(origin: &Rect, area: &Rect, occupied: &[Rect], step: f64) -> Rect {
    let mut c = clamp_inside(origin, area);
    for i in 1..=MAX_CASCADE_STEPS {
        if !hits(&c, occupied) {
            break;
        }
        let d = step * i as f64;
        c = clamp_inside(
            &Rect::new(origin.x + d, origin.y + d, origin.width, origin.height),
            area,
        );
    }
    c
}

/// The first grid spot in the area that overlaps nothing; cascades from the top left when none fits.
fn free_space(width: f64, height: f64, area: &Rect, occupied: &[Rect], step: f64) -> Rect {
    let mut steps = 0;
    let mut y = area.y;
    while y + height <= area.y + area.height && steps < MAX_SCAN_STEPS {
        let mut x = area.x;
        while x + width <= area.x + area.width && steps < MAX_SCAN_STEPS {
            let c = Rect::new(x, y, width, height);
            if !hits(&c, occupied) {
                return c;
            }
            steps += 1;
            x += SCAN_STEP;
        }
        y += SCAN_STEP;
    }
    cascade(&Rect::new(area.x, area.y, width, height), area, occupied, step)
}

/// Centred; cascades from the centre only when that spot is exactly taken.
fn centred(width: f64, height: f64, area: &Rect, occupied: &[Rect], step: f64) -> Rect {
    let c = clamp_inside(
        &Rect::new(
            area.x + ((area.width - width).max(0.0) / 2.0).floor(),
            area.y + ((area.height - height).max(0.0) / 2.0).floor(),
            width,
            height,
        ),
        area,
    );
    let taken = occupied
        .iter()
        .any(|o| o.x == c.x && o.y == c.y && o.width == c.width && o.height == c.height);
    if taken { cascade(&c, area, occupied, step) } else { c }
}

/// The rectangle for a new window of `width` by `height` in `area`, avoiding `occupied` where possible. A size below 1 on an
/// axis is raised to 1, so the window covers something.
pub(crate) fn place_rect(
    mode: PlaceMode,
    width: f64,
    height: f64,
    area: &Rect,
    occupied: &[Rect],
    cascade_step: f64,
) -> Rect {
    let w = width.max(1.0);
    let h = height.max(1.0);
    match mode {
        PlaceMode::FreeSpace => free_space(w, h, area, occupied, cascade_step),
        PlaceMode::Center => centred(w, h, area, occupied, cascade_step),
        PlaceMode::Cascade => cascade(&Rect::new(area.x, area.y, w, h), area, occupied, cascade_step),
    }
}

/// The frame inset of a window: how much room its frame takes round the content on each side.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Inset {
    pub(crate) top: f64,
    pub(crate) right: f64,
    pub(crate) bottom: f64,
    pub(crate) left: f64,
}

/// `rect` grown by `inset` on every side: the frame round a content rectangle.
pub(crate) fn grow(rect: &Rect, inset: &Inset) -> Rect {
    Rect::new(
        rect.x - inset.left,
        rect.y - inset.top,
        rect.width + inset.left + inset.right,
        rect.height + inset.top + inset.bottom,
    )
}

/// `rect` shrunk by `inset` on every side: the content inside a frame.
pub(crate) fn shrink(rect: &Rect, inset: &Inset) -> Rect {
    Rect::new(
        rect.x + inset.left,
        rect.y + inset.top,
        rect.width - inset.left - inset.right,
        rect.height - inset.top - inset.bottom,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(0.0, 0.0, 1000.0, 800.0);

    #[test]
    fn a_window_goes_to_the_first_free_grid_spot() {
        let taken = [Rect::new(0.0, 0.0, 400.0, 300.0)];
        let spot = place_rect(PlaceMode::FreeSpace, 300.0, 200.0, &AREA, &taken, 32.0);
        assert_eq!((spot.x, spot.y), (400.0, 0.0));
    }

    #[test]
    fn with_no_room_free_space_cascades_and_settles_inside_the_area() {
        // The same case the TypeScript library answers: a window of 800 by 600 against one at 10,20 of 300 by 200.
        let taken = [Rect::new(10.0, 20.0, 300.0, 200.0)];
        let spot = place_rect(PlaceMode::FreeSpace, 800.0, 600.0, &AREA, &taken, 32.0);
        assert_eq!((spot.x, spot.y, spot.width, spot.height), (200.0, 200.0, 800.0, 600.0));
    }

    #[test]
    fn centred_placement_cascades_only_from_an_exactly_taken_spot() {
        let spot = place_rect(PlaceMode::Center, 400.0, 200.0, &AREA, &[], 32.0);
        assert_eq!((spot.x, spot.y), (300.0, 300.0));
        let taken = [spot];
        let next = place_rect(PlaceMode::Center, 400.0, 200.0, &AREA, &taken, 32.0);
        // 300 + 32 * 7 = 524 is the first step that clears the rectangle that is exactly there (it ends at y = 500).
        assert_eq!((next.x, next.y), (524.0, 524.0));
    }

    #[test]
    fn a_cascade_steps_from_the_top_left_until_nothing_is_hit() {
        let taken = [Rect::new(0.0, 0.0, 100.0, 100.0)];
        let spot = place_rect(PlaceMode::Cascade, 100.0, 100.0, &AREA, &taken, 32.0);
        assert_eq!((spot.x, spot.y), (128.0, 128.0));
    }

    #[test]
    fn a_size_below_one_is_raised_to_one() {
        let spot = place_rect(PlaceMode::Center, 0.0, -5.0, &AREA, &[], 32.0);
        assert_eq!((spot.width, spot.height), (1.0, 1.0));
    }

    #[test]
    fn a_window_larger_than_the_area_goes_to_its_origin() {
        let spot = place_rect(PlaceMode::Center, 2000.0, 2000.0, &AREA, &[], 32.0);
        assert_eq!((spot.x, spot.y), (0.0, 0.0));
    }

    #[test]
    fn apart_from_moves_a_frame_off_a_shared_top_left_corner() {
        let taken = [Rect::new(200.0, 200.0, 800.0, 600.0)];
        let moved = apart_from(&Rect::new(200.0, 200.0, 800.0, 600.0), &taken, &AREA, 32.0);
        assert_eq!((moved.x, moved.y), (168.0, 168.0));
    }

    #[test]
    fn apart_from_stops_at_the_origin() {
        let taken = [Rect::new(0.0, 0.0, 800.0, 600.0)];
        let moved = apart_from(&Rect::new(0.0, 0.0, 800.0, 600.0), &taken, &AREA, 32.0);
        assert_eq!((moved.x, moved.y), (0.0, 0.0));
    }

    #[test]
    fn an_empty_rectangle_overlaps_nothing() {
        assert!(!intersects(
            &Rect::new(0.0, 0.0, 0.0, 10.0),
            &Rect::new(0.0, 0.0, 10.0, 10.0)
        ));
    }

    #[test]
    fn grow_and_shrink_are_inverses() {
        let inset = Inset {
            top: 30.0,
            right: 1.0,
            bottom: 2.0,
            left: 3.0,
        };
        let content = Rect::new(10.0, 10.0, 100.0, 50.0);
        assert_eq!(shrink(&grow(&content, &inset), &inset), content);
    }
}
