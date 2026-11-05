//! Points, rectangles and the integer zoom factor.
//!
//! Every magnifier rectangle is integer layout coordinates. The C tree computes
//! magnifier geometry in floats; nothing here needs a fraction of a pixel, and
//! integers keep a model test able to compare two runs exactly.
//!
//! # This is the projection, not the scene
//!
//! [`Point`] and [`Rect`] describe the layout plane: the projected bounds of an
//! output and the projected anchor of a caret, a focus ring or the pointer. A
//! magnifier view is a two-dimensional area of one output's surface by
//! definition, so nothing here can express a third axis and nothing here has to.
//!
//! Depth arrives later without changing a policy: `gnoblin-render` projects a
//! three-dimensional scene bound onto the layout plane and hands this crate the
//! projection, exactly as it hands it an output's bounds today. If the projected
//! anchor needs a depth of its own, [`Point`] gains a `z` and [`Rect`] gains a
//! depth, both defaulting to zero, and [`Rect::contains`], [`Rect::within`] and
//! [`Rect::magnified`] keep their meaning because they already work on the
//! projected rectangle. Nothing in this module or in
//! [`crate::MagnifierConfig`] names a type or a field that forbids that
//! addition.

/// A point in the compositor's logical layout coordinates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Point {
    /// Distance from the layout origin, positive rightwards.
    pub x: i32,
    /// Distance from the layout origin, positive downwards.
    pub y: i32,
}

impl Point {
    /// A point at these layout coordinates.
    #[must_use]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// A width and height in logical pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Size {
    /// Width in logical pixels.
    pub width: u32,
    /// Height in logical pixels.
    pub height: u32,
}

impl Size {
    /// A size in logical pixels.
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

/// An axis-aligned rectangle in layout coordinates.
///
/// The origin is inside the rectangle and the far edges are not, so two
/// rectangles that share an edge never overlap: a point on the boundary belongs
/// to the neighbour, which is what makes "the pointer is on that output" a
/// well-defined question.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width in logical pixels.
    pub width: u32,
    /// Height in logical pixels.
    pub height: u32,
}

impl Rect {
    /// A rectangle at these layout coordinates.
    #[must_use]
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    /// The size of the rectangle.
    #[must_use]
    pub const fn size(self) -> Size {
        Size::new(self.width, self.height)
    }

    /// Whether `point` lies inside.
    #[must_use]
    pub fn contains(self, point: Point) -> bool {
        let x = i64::from(point.x);
        let y = i64::from(point.y);
        x >= i64::from(self.x)
            && x < i64::from(self.x) + i64::from(self.width)
            && y >= i64::from(self.y)
            && y < i64::from(self.y) + i64::from(self.height)
    }

    /// Whether this rectangle lies entirely inside `other`.
    ///
    /// This is the magnifier's guarantee expressed as a predicate: a view whose
    /// source is not `within` its output's bounds is a bug, and both the
    /// scenario tests and the model test assert exactly that.
    #[must_use]
    pub fn within(self, other: Rect) -> bool {
        self.x >= other.x
            && self.y >= other.y
            && i64::from(self.x) + i64::from(self.width) <= i64::from(other.x) + i64::from(other.width)
            && i64::from(self.y) + i64::from(self.height) <= i64::from(other.y) + i64::from(other.height)
    }

    /// The centre point, where a magnifier with no follow target starts.
    #[must_use]
    pub fn centre(self) -> Point {
        Point::new(
            to_i32(i64::from(self.x) + i64::from(self.width) / 2),
            to_i32(i64::from(self.y) + i64::from(self.height) / 2),
        )
    }

    /// `point` moved the smallest distance that puts it inside this rectangle.
    ///
    /// A locked session uses this to keep a follow target it must not honour
    /// from describing another output's area.
    #[must_use]
    pub fn clamp(self, point: Point) -> Point {
        Point::new(
            clamp_origin(i64::from(self.x), i64::from(self.width), 1, i64::from(point.x)),
            clamp_origin(i64::from(self.y), i64::from(self.height), 1, i64::from(point.y)),
        )
    }

    /// A `size`-sized rectangle centred on `point`, moved the smallest distance
    /// that keeps it inside this rectangle.
    ///
    /// This is the whole magnifier: the visible area is a window into one
    /// output, so it can never describe anything outside it, not even by one
    /// pixel at a corner.
    #[must_use]
    pub fn centred(self, point: Point, size: Size) -> Rect {
        let width = i64::from(size.width).clamp(1, i64::from(self.width).max(1));
        let height = i64::from(size.height).clamp(1, i64::from(self.height).max(1));
        Rect::new(
            clamp_origin(
                i64::from(self.x),
                i64::from(self.width),
                width,
                i64::from(point.x) - width / 2,
            ),
            clamp_origin(
                i64::from(self.y),
                i64::from(self.height),
                height,
                i64::from(point.y) - height / 2,
            ),
            to_u32(width),
            to_u32(height),
        )
    }

    /// The area of this rectangle that a magnifier at `zoom` shows.
    ///
    /// Both magnifier modes use this source rectangle; the mode only decides
    /// whether the renderer paints it over the whole output or in a lens at its
    /// own position.
    #[must_use]
    pub fn magnified(self, point: Point, zoom: Zoom) -> Rect {
        self.centred(point, Size::new(scale(self.width, zoom), scale(self.height, zoom)))
    }
}

/// A magnifier zoom factor in thousandths, so every rectangle stays integer.
///
/// A float factor would make the view rectangle depend on rounding mode, and a
/// parity run could then differ from the reference by a pixel without anything
/// being wrong. Thousandths resolve to a whole pixel at any realistic output
/// size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Zoom(u32);

impl Zoom {
    /// One to one: the magnifier shows exactly the output.
    pub const X1: Self = Self(1000);
    /// The usual doubling, and this crate's default.
    pub const X2: Self = Self(2000);
    /// The smallest factor the compositor accepts.
    pub const MIN: Self = Self(1000);
    /// The largest factor the compositor accepts: 32 times.
    pub const MAX: Self = Self(32_000);

    /// A factor from thousandths, clamped into [`Zoom::MIN`]..=[`Zoom::MAX`].
    #[must_use]
    pub fn new(per_mille: u32) -> Self {
        Self(per_mille.clamp(Self::MIN.0, Self::MAX.0))
    }

    /// The factor in thousandths.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl Default for Zoom {
    fn default() -> Self {
        Self::X2
    }
}

/// The area of an `extent`-sized span a magnifier at `zoom` shows: never larger
/// than the output and never zero, so a view always covers at least one pixel.
fn scale(extent: u32, zoom: Zoom) -> u32 {
    let per_mille = u64::from(zoom.get());
    let scaled = (u64::from(extent) * 1000).div_ceil(per_mille);
    let scaled = u32::try_from(scaled).unwrap_or(u32::MAX);
    scaled.clamp(1, extent.max(1))
}

/// The origin of a `len`-long span placed at `wanted` inside a `extent`-long
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
