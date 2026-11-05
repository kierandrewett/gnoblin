//! Directions, edges and corners, and the axis they lie on.
//!
//! These are the words placement, snapping, resize handles and the output layout
//! share. Each type is total over its geometry: every [`Direction`] has exactly
//! one [`Edge`], every [`Edge`] has exactly one [`Direction`], and every
//! [`Corner`] pairs one horizontal edge with one vertical edge. A caller that
//! matches on `(left, right)` pairs to decide something therefore has one
//! vocabulary instead of four local enums.

/// An axis of the layout plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Axis {
    /// The x axis: left and right.
    Horizontal,
    /// The y axis: top and bottom.
    Vertical,
}

/// Which way something moves or grows.
///
/// `Left` and `Up` are the negative directions of their axis, which is what
/// makes [`Direction::is_positive`] meaningful for a sign test.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Direction {
    /// Negative along the x axis.
    Left,
    /// Positive along the x axis.
    Right,
    /// Negative along the y axis.
    Up,
    /// Positive along the y axis.
    Down,
}

impl Direction {
    /// Every direction, in the order left, right, up, down.
    pub const ALL: [Self; 4] = [Self::Left, Self::Right, Self::Up, Self::Down];

    /// The opposite direction.
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            Self::Up => Self::Down,
            Self::Down => Self::Up,
        }
    }

    /// The axis this direction runs along.
    #[must_use]
    pub const fn axis(self) -> Axis {
        match self {
            Self::Left | Self::Right => Axis::Horizontal,
            Self::Up | Self::Down => Axis::Vertical,
        }
    }

    /// Whether this direction increases its coordinate.
    #[must_use]
    pub const fn is_positive(self) -> bool {
        match self {
            Self::Left | Self::Up => false,
            Self::Right | Self::Down => true,
        }
    }

    /// The edge of a rectangle this direction names.
    ///
    /// Moving left means leaving through the left edge, so the two vocabularies
    /// agree without a table at every call site.
    #[must_use]
    pub const fn edge(self) -> Edge {
        match self {
            Self::Left => Edge::Left,
            Self::Right => Edge::Right,
            Self::Up => Edge::Top,
            Self::Down => Edge::Bottom,
        }
    }
}

/// One side of a rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Edge {
    /// The left side, at the smaller x.
    Left,
    /// The right side, at the larger x.
    Right,
    /// The top side, at the smaller y.
    Top,
    /// The bottom side, at the larger y.
    Bottom,
}

impl Edge {
    /// Every edge, in the order left, right, top, bottom.
    pub const ALL: [Self; 4] = [Self::Left, Self::Right, Self::Top, Self::Bottom];

    /// The opposite edge.
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
        }
    }

    /// The axis this edge runs along.
    #[must_use]
    pub const fn axis(self) -> Axis {
        match self {
            Self::Left | Self::Right => Axis::Vertical,
            Self::Top | Self::Bottom => Axis::Horizontal,
        }
    }

    /// The direction that leaves the rectangle through this edge.
    #[must_use]
    pub const fn direction(self) -> Direction {
        match self {
            Self::Left => Direction::Left,
            Self::Right => Direction::Right,
            Self::Top => Direction::Up,
            Self::Bottom => Direction::Down,
        }
    }

    /// Whether this is the near edge of its axis, at the smaller coordinate.
    #[must_use]
    pub const fn is_start(self) -> bool {
        matches!(self, Self::Left | Self::Top)
    }

    /// The corner at the start of this edge.
    #[must_use]
    pub const fn start_corner(self) -> Corner {
        match self {
            Self::Left => Corner::TopLeft,
            Self::Right => Corner::TopRight,
            Self::Top => Corner::TopLeft,
            Self::Bottom => Corner::BottomLeft,
        }
    }

    /// The corner at the end of this edge.
    #[must_use]
    pub const fn end_corner(self) -> Corner {
        match self {
            Self::Left => Corner::BottomLeft,
            Self::Right => Corner::BottomRight,
            Self::Top => Corner::TopRight,
            Self::Bottom => Corner::BottomRight,
        }
    }
}

/// One corner of a rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Corner {
    /// The corner at the smaller x and smaller y.
    TopLeft,
    /// The corner at the larger x and smaller y.
    TopRight,
    /// The corner at the smaller x and larger y.
    BottomLeft,
    /// The corner at the larger x and larger y.
    BottomRight,
}

impl Corner {
    /// Every corner, in the order top-left, top-right, bottom-left, bottom-right.
    pub const ALL: [Self; 4] = [Self::TopLeft, Self::TopRight, Self::BottomLeft, Self::BottomRight];

    /// The diagonally opposite corner.
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::TopLeft => Self::BottomRight,
            Self::TopRight => Self::BottomLeft,
            Self::BottomLeft => Self::TopRight,
            Self::BottomRight => Self::TopLeft,
        }
    }

    /// The horizontal edge this corner lies on.
    #[must_use]
    pub const fn horizontal_edge(self) -> Edge {
        match self {
            Self::TopLeft | Self::TopRight => Edge::Top,
            Self::BottomLeft | Self::BottomRight => Edge::Bottom,
        }
    }

    /// The vertical edge this corner lies on.
    #[must_use]
    pub const fn vertical_edge(self) -> Edge {
        match self {
            Self::TopLeft | Self::BottomLeft => Edge::Left,
            Self::TopRight | Self::BottomRight => Edge::Right,
        }
    }

    /// Both edges meeting at this corner, the horizontal one first.
    #[must_use]
    pub const fn edges(self) -> [Edge; 2] {
        [self.horizontal_edge(), self.vertical_edge()]
    }

    /// Whether the corner lies on the near sides of both axes.
    #[must_use]
    pub const fn is_start(self) -> bool {
        matches!(self, Self::TopLeft)
    }
}
