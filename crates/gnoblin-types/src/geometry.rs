//! Points, sizes, rectangles and insets, in logical pixels and in continuous
//! space.
//!
//! See the crate documentation for why there are two spaces. The short version:
//! layout talks in whole logical pixels ([`Point`], [`Size`], [`Rect`]), a
//! transform and a camera talk in [`f64`] ([`PointF`], [`RectF`], [`Vec3`]), and
//! the crossing between them is explicit.

/// A width and height in logical pixels.
///
/// Extents are unsigned: a negative width cannot be laid out, and an unsigned
/// extent makes "a logical size is a whole, non-negative number of pixels" a
/// property of the type. Zero is allowed and means an empty extent — see
/// [`Size::is_empty`] and [`Size::at_least_one`], because a rectangle that
/// covers at least one pixel is a different statement from one that covers
/// none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Size {
    /// Horizontal extent.
    pub width: u32,
    /// Vertical extent.
    pub height: u32,
}

impl Size {
    /// The empty extent.
    pub const ZERO: Self = Self::new(0, 0);

    /// A size from its extents, exactly as given.
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Whether either extent is zero, so the size covers no pixel.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// The same size with every zero extent raised to one pixel.
    ///
    /// Callers that own the policy (window placement, damage) use this instead
    /// of a constructor that clamps silently, so the decision to round up is
    /// visible at the call site.
    #[must_use]
    pub const fn at_least_one(self) -> Self {
        Self::new(
            if self.width == 0 { 1 } else { self.width },
            if self.height == 0 { 1 } else { self.height },
        )
    }

    /// The same extents with the axes exchanged.
    ///
    /// A quarter-turn output transform presents the mode's height horizontally,
    /// and starting from the mode's size is the only place that decision is
    /// made.
    #[must_use]
    pub const fn swapped(self) -> Self {
        Self::new(self.height, self.width)
    }

    /// The number of pixels the size covers.
    #[must_use]
    pub const fn area(self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

/// A point in the compositor's logical layout coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Point {
    /// Distance from the layout origin, positive rightwards.
    pub x: i32,
    /// Distance from the layout origin, positive downwards.
    pub y: i32,
}

impl Point {
    /// The layout origin.
    pub const ZERO: Self = Self::new(0, 0);

    /// A point at these layout coordinates.
    #[must_use]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// The same point moved by an offset, saturating instead of wrapping.
    #[must_use]
    pub const fn translate(self, dx: i32, dy: i32) -> Self {
        Self::new(self.x.saturating_add(dx), self.y.saturating_add(dy))
    }

    /// The continuous form of this point, for a transform or a hit test.
    #[must_use]
    pub fn to_f64(self) -> PointF {
        PointF::new(f64::from(self.x), f64::from(self.y))
    }
}

/// An axis-aligned rectangle in logical pixels, half-open on the right and
/// bottom edges.
///
/// The origin is signed because an output placed to the left of or above the
/// primary has a negative coordinate. The extent is unsigned, so a rectangle
/// either covers pixels or is empty; [`Rect::new`] does not round a zero extent
/// up, because "empty" is a meaningful state (a hidden surface, an output with
/// no work area left) and silently turning it into a pixel hides a bug.
///
/// # Edges are 64-bit
///
/// [`Rect::right`] and [`Rect::bottom`] return [`i64`]. An origin near
/// [`i32::MAX`] plus a [`u32`] extent does not fit in an [`i32`], and a
/// saturating [`i32`] edge would silently shrink the rectangle. Computing edges
/// in 64 bits keeps every predicate exact, and the 64-bit width of the two
/// inputs is the reason: `x + width` needs at most 33 bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Horizontal extent, zero when the rectangle is empty.
    pub width: u32,
    /// Vertical extent, zero when the rectangle is empty.
    pub height: u32,
}

impl Rect {
    /// The empty rectangle at the origin.
    pub const EMPTY: Self = Self::new(0, 0, 0, 0);

    /// A rectangle from its origin and extent, exactly as given.
    #[must_use]
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    /// A rectangle at the origin with the given size.
    #[must_use]
    pub const fn from_size(size: Size) -> Self {
        Self::new(0, 0, size.width, size.height)
    }

    /// The same rectangle moved to a new origin.
    #[must_use]
    pub const fn with_origin(self, x: i32, y: i32) -> Self {
        Self::new(x, y, self.width, self.height)
    }

    /// The extent, without the origin.
    #[must_use]
    pub const fn size(self) -> Size {
        Size::new(self.width, self.height)
    }

    /// The number of pixels the rectangle covers.
    #[must_use]
    pub const fn area(self) -> u64 {
        self.size().area()
    }

    /// Whether the rectangle covers no pixel.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// The first column past the right edge.
    #[must_use]
    pub fn right(self) -> i64 {
        i64::from(self.x) + i64::from(self.width)
    }

    /// The first row past the bottom edge.
    #[must_use]
    pub fn bottom(self) -> i64 {
        i64::from(self.y) + i64::from(self.height)
    }

    /// Whether the two rectangles share at least one pixel.
    ///
    /// Interior overlap only: touching along an edge is not an overlap, because
    /// that is exactly how two monitors in a layout are arranged. An empty
    /// rectangle shares nothing, not even the line its origin sits on.
    #[must_use]
    pub fn intersects(self, other: Self) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && i64::from(self.x) < other.right()
            && i64::from(other.x) < self.right()
            && i64::from(self.y) < other.bottom()
            && i64::from(other.y) < self.bottom()
    }

    /// Whether the two rectangles share an edge over a non-zero span.
    ///
    /// This is the connection an output layout requires: a pointer must be able
    /// to cross between two monitors at some point, which a corner-to-corner
    /// touch does not allow.
    #[must_use]
    pub fn touches(self, other: Self) -> bool {
        if self.is_empty() || other.is_empty() {
            return false;
        }
        let horizontal_span = i64::from(self.x) < other.right() && i64::from(other.x) < self.right();
        let vertical_span = i64::from(self.y) < other.bottom() && i64::from(other.y) < self.bottom();
        let vertical_edge = self.right() == i64::from(other.x) || other.right() == i64::from(self.x);
        let horizontal_edge = self.bottom() == i64::from(other.y) || other.bottom() == i64::from(self.y);
        (vertical_edge && vertical_span) || (horizontal_edge && horizontal_span)
    }

    /// Whether `point` lies inside, with the far edges excluded.
    #[must_use]
    pub fn contains(self, point: Point) -> bool {
        let x = i64::from(point.x);
        let y = i64::from(point.y);
        x >= i64::from(self.x) && x < self.right() && y >= i64::from(self.y) && y < self.bottom()
    }

    /// Whether the whole of this rectangle lies inside `bounds`.
    #[must_use]
    pub fn is_inside(self, bounds: Self) -> bool {
        i64::from(self.x) >= i64::from(bounds.x)
            && i64::from(self.y) >= i64::from(bounds.y)
            && self.right() <= bounds.right()
            && self.bottom() <= bounds.bottom()
    }

    /// The area both rectangles cover, or [`None`] when they do not overlap.
    ///
    /// An empty rectangle intersects nothing, including itself: an empty
    /// rectangle has no pixels to share. The result is always inside both
    /// arguments, because it is built from the inner pair of edges.
    #[must_use]
    pub fn intersection(self, other: Self) -> Option<Self> {
        if self.is_empty() || other.is_empty() {
            return None;
        }
        let left = i64::from(self.x).max(i64::from(other.x));
        let top = i64::from(self.y).max(i64::from(other.y));
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right <= left || bottom <= top {
            return None;
        }
        Some(Self::from_edges(left, top, right, bottom))
    }

    /// The smallest rectangle covering both.
    ///
    /// An empty rectangle covers no pixel, so it contributes nothing: the union
    /// of an empty rectangle and a real one is the real one, and the union of
    /// two empty rectangles is [`Rect::EMPTY`] rather than whichever operand
    /// happened to be first, so the operation stays symmetric.
    ///
    /// The extents are saturated at [`u32::MAX`], which takes a pair of
    /// rectangles that together span more than four billion pixels: no such
    /// layout exists, and a saturated union still contains both inputs' origins.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        match (self.is_empty(), other.is_empty()) {
            (true, true) => Self::EMPTY,
            (true, false) => other,
            (false, true) => self,
            (false, false) => {
                let left = i64::from(self.x).min(i64::from(other.x));
                let top = i64::from(self.y).min(i64::from(other.y));
                let right = self.right().max(other.right());
                let bottom = self.bottom().max(other.bottom());
                Self::from_edges(left, top, right, bottom)
            }
        }
    }

    /// The same rectangle moved by an offset, saturating the origin.
    #[must_use]
    pub fn translate(self, dx: i32, dy: i32) -> Self {
        Self::new(
            self.x.saturating_add(dx),
            self.y.saturating_add(dy),
            self.width,
            self.height,
        )
    }

    /// The same rectangle with every empty extent raised to one pixel.
    #[must_use]
    pub const fn at_least_one(self) -> Self {
        let size = self.size().at_least_one();
        Self::new(self.x, self.y, size.width, size.height)
    }

    /// The rectangle moved as little as possible to sit inside `bounds`.
    ///
    /// A rectangle larger than the bounds is aligned to the bounds' origin and
    /// keeps its size, because shrinking a client's window is the compositor's
    /// decision to send as a configure, not a geometry side effect.
    #[must_use]
    pub fn clamp_inside(self, bounds: Self) -> Self {
        let x = if i64::from(self.width) >= i64::from(bounds.width) {
            i64::from(bounds.x)
        } else {
            let last = bounds.right() - i64::from(self.width);
            i64::from(self.x).max(i64::from(bounds.x)).min(last)
        };
        let y = if i64::from(self.height) >= i64::from(bounds.height) {
            i64::from(bounds.y)
        } else {
            let last = bounds.bottom() - i64::from(self.height);
            i64::from(self.y).max(i64::from(bounds.y)).min(last)
        };
        Self::new(saturating_i32(x), saturating_i32(y), self.width, self.height)
    }

    /// The point inside this rectangle nearest to `point`.
    ///
    /// An empty rectangle has no inside, so it clamps to its own origin: that is
    /// the one answer that cannot describe a pixel the rectangle does not own.
    #[must_use]
    pub fn clamp_point(self, point: Point) -> Point {
        if self.is_empty() {
            return Point::new(self.x, self.y);
        }
        let x = i64::from(point.x).clamp(i64::from(self.x), self.right() - 1);
        let y = i64::from(point.y).clamp(i64::from(self.y), self.bottom() - 1);
        Point::new(saturating_i32(x), saturating_i32(y))
    }

    /// The centre of the rectangle, rounded down to a whole pixel.
    #[must_use]
    pub fn centre(self) -> Point {
        Point::new(
            saturating_i32(i64::from(self.x) + i64::from(self.width) / 2),
            saturating_i32(i64::from(self.y) + i64::from(self.height) / 2),
        )
    }

    /// The continuous form of this rectangle.
    #[must_use]
    pub fn to_f64(self) -> RectF {
        RectF::new(
            f64::from(self.x),
            f64::from(self.y),
            f64::from(self.width),
            f64::from(self.height),
        )
    }

    /// Build a rectangle from 64-bit edges, saturating into the value's ranges.
    fn from_edges(left: i64, top: i64, right: i64, bottom: i64) -> Self {
        let x = saturating_i32(left);
        let y = saturating_i32(top);
        let width = u32::try_from(right.saturating_sub(left)).unwrap_or(u32::MAX);
        let height = u32::try_from(bottom.saturating_sub(top)).unwrap_or(u32::MAX);
        Self::new(x, y, width, height)
    }
}

/// Space reserved by panels and bars, counted inwards from each edge.
///
/// The values are signed so an offset can be negative, which a shell uses to
/// reserve less than nothing (a bar that overlaps the screen edge). Insets are
/// not clamped here: applying them to a rectangle saturates, and rounding a
/// work area up to one pixel is the caller's policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    /// No reservation.
    pub const ZERO: Self = Self::uniform(0);

    /// An inset on each edge.
    #[must_use]
    pub const fn new(top: i32, right: i32, bottom: i32, left: i32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    /// The same inset on every edge.
    #[must_use]
    pub const fn uniform(value: i32) -> Self {
        Self::new(value, value, value, value)
    }

    /// The space the two horizontal insets take together.
    #[must_use]
    pub const fn horizontal(self) -> i64 {
        self.left as i64 + self.right as i64
    }

    /// The space the two vertical insets take together.
    #[must_use]
    pub const fn vertical(self) -> i64 {
        self.top as i64 + self.bottom as i64
    }

    /// Whether nothing is reserved.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.top == 0 && self.right == 0 && self.bottom == 0 && self.left == 0
    }

    /// The rectangle left after reserving this space on every edge.
    ///
    /// A reservation larger than the rectangle empties it rather than
    /// producing a negative extent; [`Rect::at_least_one`] is how a caller that
    /// needs a usable work area rounds the result up.
    #[must_use]
    pub fn applied_to(self, rect: Rect) -> Rect {
        let x = i64::from(rect.x).saturating_add(i64::from(self.left));
        let y = i64::from(rect.y).saturating_add(i64::from(self.top));
        let width = i64::from(rect.width).saturating_sub(self.horizontal());
        let height = i64::from(rect.height).saturating_sub(self.vertical());
        Rect::new(
            saturating_i32(x),
            saturating_i32(y),
            u32::try_from(width.max(0)).unwrap_or(u32::MAX),
            u32::try_from(height.max(0)).unwrap_or(u32::MAX),
        )
    }
}

/// A point on an output plane, in continuous logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointF {
    /// Horizontal position.
    pub x: f64,
    /// Vertical position.
    pub y: f64,
}

impl PointF {
    /// The origin.
    pub const ZERO: Self = Self::new(0.0, 0.0);

    /// A point from its components.
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Whether both components are finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    /// The whole-pixel point that contains this one.
    ///
    /// Rounding is flooring, so a value inside a pixel maps to that pixel. If
    /// either component is non-finite, both map to zero. A caller that needs a
    /// covering rectangle uses [`RectF::to_pixel`].
    #[must_use]
    pub fn to_i32(self) -> Point {
        if !self.is_finite() {
            return Point::ZERO;
        }
        Point::new(
            saturating_i32_from_float(self.x.floor()),
            saturating_i32_from_float(self.y.floor()),
        )
    }
}

/// A rectangle in continuous logical space.
///
/// It deliberately does not clamp a zero or negative extent: a transform can
/// collapse a node to a line or a point, and the collapse is information the
/// damage step needs. [`RectF::to_pixel`] is where a rectangle becomes at least
/// one pixel again.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectF {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Horizontal extent.
    pub width: f64,
    /// Vertical extent.
    pub height: f64,
}

impl RectF {
    /// The empty rectangle at the origin.
    pub const EMPTY: Self = Self::new(0.0, 0.0, 0.0, 0.0);

    /// A rectangle from its origin and extent.
    #[must_use]
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self { x, y, width, height }
    }

    /// The continuous form of a pixel rectangle.
    #[must_use]
    pub fn from_rect(rect: Rect) -> Self {
        rect.to_f64()
    }

    /// The first column past the right edge.
    #[must_use]
    pub fn right(self) -> f64 {
        self.x + self.width
    }

    /// The first row past the bottom edge.
    #[must_use]
    pub fn bottom(self) -> f64 {
        self.y + self.height
    }

    /// The centre, which is the pivot every 2D rotation and scale uses today.
    ///
    /// The depth is zero: the pivot of a node sits on the plane that node
    /// occupies until depth support lands.
    #[must_use]
    pub fn centre(self) -> Vec3 {
        Vec3::new(self.x + self.width / 2.0, self.y + self.height / 2.0, 0.0)
    }

    /// The smallest rectangle containing both.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self::new(x, y, right - x, bottom - y)
    }

    /// Whether this rectangle already contains `other`.
    #[must_use]
    pub fn contains(self, other: Self) -> bool {
        other.x >= self.x && other.y >= self.y && other.right() <= self.right() && other.bottom() <= self.bottom()
    }

    /// Whether every component is finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.width.is_finite() && self.height.is_finite()
    }

    /// The pixel rectangle that covers this one, at least one pixel in each
    /// direction.
    ///
    /// Flooring the near edges and ceiling the far edges means a sub-pixel move
    /// still damages both the pixel it left and the pixel it entered, which is
    /// what makes damage coverage an invariant rather than a best effort. A
    /// non-finite rectangle becomes one pixel at the origin rather than a
    /// nonsensical damage region.
    #[must_use]
    pub fn to_pixel(self) -> Rect {
        if !self.is_finite() {
            return Rect::new(0, 0, 1, 1);
        }
        let x = saturating_i32_from_float(self.x.floor());
        let y = saturating_i32_from_float(self.y.floor());
        let right = saturating_i32_from_float(self.right().ceil());
        let bottom = saturating_i32_from_float(self.bottom().ceil());
        let width = u32::try_from(i64::from(right) - i64::from(x))
            .unwrap_or(u32::MAX)
            .max(1);
        let height = u32::try_from(i64::from(bottom) - i64::from(y))
            .unwrap_or(u32::MAX)
            .max(1);
        Rect::new(x, y, width, height)
    }
}

/// A point or vector in three-axis continuous space.
///
/// It is what a [`crate::Transform`] carries: a 3D-ready node keeps three axes
/// from the start so enabling depth adds capabilities instead of changing a
/// released type. The axes are rightwards, downwards and away from the viewer,
/// which is the layout plane extended by a depth.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    /// First axis, rightwards in the default 2D view.
    pub x: f64,
    /// Second axis, downwards in the default 2D view.
    pub y: f64,
    /// Third axis, away from the viewer in the default 2D view.
    pub z: f64,
}

impl Vec3 {
    /// The origin.
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);

    /// A unit value on every axis, which is an identity scale and no offset.
    pub const ONE: Self = Self::new(1.0, 1.0, 1.0);

    /// A value from its components.
    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// A value on the layout plane, with no depth.
    #[must_use]
    pub const fn flat(x: f64, y: f64) -> Self {
        Self::new(x, y, 0.0)
    }

    /// The same value on every axis.
    #[must_use]
    pub const fn splat(value: f64) -> Self {
        Self::new(value, value, value)
    }

    /// Whether every component is finite.
    ///
    /// A non-finite transform would make every derived bound non-finite and
    /// silently turn damage into nonsense, so it is refused at the scene edge.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

/// Component-wise addition.
impl std::ops::Add for Vec3 {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }
}

/// Component-wise subtraction.
impl std::ops::Sub for Vec3 {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }
}

/// Component-wise multiplication.
impl std::ops::Mul for Vec3 {
    type Output = Self;

    fn mul(self, other: Self) -> Self {
        Self::new(self.x * other.x, self.y * other.y, self.z * other.z)
    }
}

/// A 64-bit signed value saturated into the range of an [`i32`].
fn saturating_i32(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(if value.is_negative() { i32::MIN } else { i32::MAX })
}

/// A continuous value saturated into the range of an [`i32`], NaN mapping to zero.
fn saturating_i32_from_float(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    if value <= f64::from(i32::MIN) {
        i32::MIN
    } else if value >= f64::from(i32::MAX) {
        i32::MAX
    } else {
        value as i32
    }
}
