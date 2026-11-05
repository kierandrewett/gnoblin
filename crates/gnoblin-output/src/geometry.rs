//! Logical rectangles and physical sizes.
//!
//! A rectangle is an output's box projected onto the desktop plane. The session
//! places monitors in two axes today and depth belongs to the renderer
//! (design/rust-compositor.md, "3D-ready, 2D today"), so nothing here forecloses a
//! third axis: [`Rect`] is built through [`Rect::new`] and read through
//! [`Rect::size`], [`Rect::overlaps`] and [`Rect::touches`], which are the only
//! places that would learn about depth. Adding it means a `depth` field on [`Rect`]
//! that defaults to zero, a constructor that takes it, and depth-aware overlap and
//! adjacency; every call site in this crate already goes through those four
//! functions, so none of them would have to be restructured.

/// A width and height in whole logical pixels, each at least one.
///
/// Extents are unsigned because a logical monitor cannot be narrower than the
/// pixel it renders, and keeping them unsigned is what makes "logical sizes are
/// integral" a property of the type instead of a rule someone has to remember.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct Size {
    /// Horizontal extent.
    pub width: u32,
    /// Vertical extent.
    pub height: u32,
}

impl Size {
    /// A size, clamped to at least one pixel in each direction.
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Self {
            width: if width == 0 { 1 } else { width },
            height: if height == 0 { 1 } else { height },
        }
    }

    /// The same extent with the axes exchanged.
    ///
    /// A quarter turn presents the mode's height horizontally, and starting from
    /// the mode's size is the only place that decision is made.
    #[must_use]
    pub const fn swapped(self) -> Self {
        Self {
            width: self.height,
            height: self.width,
        }
    }
}

impl Default for Size {
    fn default() -> Self {
        Self::new(1, 1)
    }
}

/// A logical monitor rectangle in global logical pixel coordinates.
///
/// The origin is signed because an output placed to the left of or above the
/// primary has a negative coordinate, which the layout engine produces from a
/// relative placement without clamping it to the origin.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Horizontal extent, at least one.
    pub width: u32,
    /// Vertical extent, at least one.
    pub height: u32,
}

impl Rect {
    /// A rectangle, clamped to at least one pixel in each direction.
    #[must_use]
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width: if width == 0 { 1 } else { width },
            height: if height == 0 { 1 } else { height },
        }
    }

    /// The extent, without the origin.
    #[must_use]
    pub const fn size(self) -> Size {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    /// The right edge, exclusive.
    fn right(self) -> i64 {
        i64::from(self.x) + i64::from(self.width)
    }

    /// The bottom edge, exclusive.
    fn bottom(self) -> i64 {
        i64::from(self.y) + i64::from(self.height)
    }

    /// Whether the two rectangles share at least one pixel.
    ///
    /// Interior overlap only: touching along an edge is not an overlap, because
    /// that is exactly how two monitors in a layout are arranged.
    #[must_use]
    pub fn overlaps(self, other: Self) -> bool {
        i64::from(self.x) < other.right()
            && i64::from(other.x) < self.right()
            && i64::from(self.y) < other.bottom()
            && i64::from(other.y) < self.bottom()
    }

    /// Whether the two rectangles share an edge over a non-zero span.
    ///
    /// This is the connection the layout requires: a pointer must be able to
    /// cross between two monitors at some point, which a corner-to-corner touch
    /// does not allow.
    #[must_use]
    pub fn touches(self, other: Self) -> bool {
        let horizontal_span = i64::from(self.x) < other.right() && i64::from(other.x) < self.right();
        let vertical_span = i64::from(self.y) < other.bottom() && i64::from(other.y) < self.bottom();
        let vertical_edge = self.right() == i64::from(other.x) || other.right() == i64::from(self.x);
        let horizontal_edge = self.bottom() == i64::from(other.y) || other.bottom() == i64::from(self.y);
        (vertical_edge && vertical_span) || (horizontal_edge && horizontal_span)
    }
}

/// The panel's physical extent, in millimetres.
///
/// A zero axis means the display did not report one, which `EDID` allows and
/// projectors commonly do, so zero is preserved instead of being turned into a
/// plausible-looking number.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash, Default)]
pub struct PhysicalSize {
    /// Horizontal extent in millimetres, or zero when unknown.
    pub width_mm: u32,
    /// Vertical extent in millimetres, or zero when unknown.
    pub height_mm: u32,
}

impl PhysicalSize {
    /// A physical extent in millimetres, zero meaning unknown.
    #[must_use]
    pub const fn new(width_mm: u32, height_mm: u32) -> Self {
        Self { width_mm, height_mm }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_extent_is_never_stored() {
        assert_eq!(Size::new(0, 0), Size::new(1, 1));
        assert_eq!(Rect::new(-10, 5, 0, 0), Rect::new(-10, 5, 1, 1));
    }

    #[test]
    fn side_by_side_monitors_touch_but_do_not_overlap() {
        let left = Rect::new(0, 0, 1920, 1080);
        let right = Rect::new(1920, 0, 1920, 1080);
        assert!(!left.overlaps(right));
        assert!(left.touches(right));
        assert!(right.touches(left));
    }

    #[test]
    fn a_corner_touch_is_not_a_connection() {
        let top_left = Rect::new(0, 0, 100, 100);
        let bottom_right = Rect::new(100, 100, 100, 100);
        assert!(!top_left.overlaps(bottom_right));
        assert!(!top_left.touches(bottom_right));
    }

    #[test]
    fn an_offset_pair_overlaps_and_a_separated_pair_does_not_touch() {
        let first = Rect::new(0, 0, 100, 100);
        assert!(first.overlaps(Rect::new(99, 0, 100, 100)));
        assert!(!first.touches(Rect::new(101, 0, 100, 100)));
    }

    #[test]
    fn a_quarter_turn_exchanges_the_axes() {
        assert_eq!(Size::new(1920, 1080).swapped(), Size::new(1080, 1920));
    }
}
