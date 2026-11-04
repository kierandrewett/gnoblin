//! Rectangles in logical global coordinates.
//!
//! The window manager compares, clamps and splits rectangles and nothing else, so
//! it carries its own integer geometry instead of borrowing a compositor type.
//! Coordinates are logical pixels in the session's global space; scaling belongs
//! to the renderer. Every constructor clamps a non-positive width or height to one
//! pixel, so a degenerate client request cannot produce a rectangle that
//! intersects nothing and disappears from every placement scan.

/// A width and height in logical pixels, each at least one.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Size {
    /// Horizontal extent.
    pub width: i32,
    /// Vertical extent.
    pub height: i32,
}

impl Size {
    /// A size, clamped to at least one pixel in each direction.
    #[must_use]
    pub const fn new(width: i32, height: i32) -> Self {
        Self {
            width: if width < 1 { 1 } else { width },
            height: if height < 1 { 1 } else { height },
        }
    }
}

impl Default for Size {
    fn default() -> Self {
        Self::new(1, 1)
    }
}

/// An axis-aligned rectangle, half-open on the right and bottom edges.
///
/// There is deliberately no `Default`: a zero-size rectangle is never valid, and
/// the only way to build one should be a constructor that clamps it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Horizontal extent, at least one.
    pub width: i32,
    /// Vertical extent, at least one.
    pub height: i32,
}

impl Rect {
    /// A rectangle, clamped to at least one pixel in each direction.
    #[must_use]
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width: if width < 1 { 1 } else { width },
            height: if height < 1 { 1 } else { height },
        }
    }

    /// A rectangle at the origin with the given size.
    #[must_use]
    pub const fn from_size(size: Size) -> Self {
        Self::new(0, 0, size.width, size.height)
    }

    /// The first column past the right edge.
    #[must_use]
    pub const fn right(&self) -> i32 {
        self.x.saturating_add(self.width)
    }

    /// The first row past the bottom edge.
    #[must_use]
    pub const fn bottom(&self) -> i32 {
        self.y.saturating_add(self.height)
    }

    /// The size of the rectangle.
    #[must_use]
    pub const fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    /// Whether the two rectangles share more than a zero-area edge.
    ///
    /// Touching edges do not count as overlapping, so a window placed exactly
    /// beside another one is free space rather than a collision.
    #[must_use]
    pub fn intersects(&self, other: &Self) -> bool {
        self.x < other.right() && other.x < self.right() && self.y < other.bottom() && other.y < self.bottom()
    }

    /// The same rectangle moved by the given offset.
    #[must_use]
    pub const fn translate(&self, dx: i32, dy: i32) -> Self {
        Self::new(
            self.x.saturating_add(dx),
            self.y.saturating_add(dy),
            self.width,
            self.height,
        )
    }

    /// The rectangle moved as little as possible to sit inside `bounds`.
    ///
    /// A rectangle larger than the bounds is aligned to the bounds' origin and
    /// keeps its size, because shrinking a client's window is the compositor's
    /// decision to send as a configure, not a geometry side effect.
    #[must_use]
    pub fn clamp_inside(&self, bounds: &Self) -> Self {
        let x = if self.width >= bounds.width {
            bounds.x
        } else {
            self.x.max(bounds.x).min(bounds.right().saturating_sub(self.width))
        };
        let y = if self.height >= bounds.height {
            bounds.y
        } else {
            self.y.max(bounds.y).min(bounds.bottom().saturating_sub(self.height))
        };
        Self::new(x, y, self.width, self.height)
    }

    /// Whether the whole of `self` lies inside `bounds`.
    #[must_use]
    pub fn is_inside(&self, bounds: &Self) -> bool {
        self.x >= bounds.x && self.y >= bounds.y && self.right() <= bounds.right() && self.bottom() <= bounds.bottom()
    }
}

/// Space reserved by panels and bars, counted inwards from each output edge.
///
/// The work area is the output rectangle minus these insets. They are part of the
/// output model rather than a separate layer list because placement, maximise and
/// snapping only ever need the resulting rectangle.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Insets {
    /// Reserved above the work area.
    pub top: i32,
    /// Reserved to the right of the work area.
    pub right: i32,
    /// Reserved below the work area.
    pub bottom: i32,
    /// Reserved to the left of the work area.
    pub left: i32,
}

impl Insets {
    /// The same inset on every edge.
    #[must_use]
    pub const fn uniform(value: i32) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }

    /// No reservation.
    pub const ZERO: Self = Self::uniform(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_degenerate_size_becomes_one_pixel() {
        let rect = Rect::new(10, 10, 0, -40);
        assert_eq!((rect.width, rect.height), (1, 1));
        assert_eq!(Size::new(-5, 0), Size::new(1, 1));
    }

    #[test]
    fn touching_edges_do_not_intersect() {
        let left = Rect::new(0, 0, 100, 100);
        let right = Rect::new(100, 0, 100, 100);
        assert!(!left.intersects(&right));
        assert!(left.intersects(&Rect::new(99, 0, 100, 100)));
        assert!(!left.intersects(&Rect::new(0, 100, 100, 100)));
    }

    #[test]
    fn clamping_moves_inside_but_keeps_size() {
        let bounds = Rect::new(0, 0, 1920, 1080);
        let clamped = Rect::new(1900, -50, 400, 300).clamp_inside(&bounds);
        assert_eq!(clamped, Rect::new(1520, 0, 400, 300));
        assert!(clamped.is_inside(&bounds));
    }

    #[test]
    fn clamping_an_oversized_window_aligns_it_to_the_origin() {
        let bounds = Rect::new(100, 50, 800, 600);
        let clamped = Rect::new(400, 400, 900, 700).clamp_inside(&bounds);
        assert_eq!(clamped, Rect::new(100, 50, 900, 700));
    }

    #[test]
    fn clamping_an_inside_rectangle_changes_nothing() {
        let bounds = Rect::new(0, 0, 1920, 1080);
        let rect = Rect::new(100, 100, 200, 200);
        assert_eq!(rect.clamp_inside(&bounds), rect);
    }

    #[test]
    fn extremes_saturate_instead_of_panicking() {
        let rect = Rect::new(i32::MAX - 5, i32::MIN, 10, 10);
        assert_eq!(rect.right(), i32::MAX);
        assert_eq!(rect.translate(i32::MAX, i32::MIN).width, 10);
    }
}
