//! The two monotonic counters: a caller-supplied clock reading and a graph
//! history stamp.

use std::fmt;

/// A point on the caller's monotonic clock, in milliseconds.
///
/// Nothing in Gnoblin reads a clock: the compositor passes the reading it
/// already has and a test passes a literal, so the same event sequence with the
/// same ticks always produces the same decisions. One tick is one millisecond of
/// the caller's clock; a crate that measures something else converts first.
///
/// Every operation saturates instead of wrapping and never panics. A counter that
/// wrapped would turn a deadline in the future into one in the past and a policy
/// decision would expire before it was sent, so the unreachable deadline is the
/// safe direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tick(u64);

impl Tick {
    /// The first tick.
    pub const ZERO: Self = Self(0);

    /// A tick with this reading, for a wire decoder, a frame dump or a test.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw reading, in milliseconds.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The tick this many milliseconds later, saturating at [`u64::MAX`].
    #[must_use]
    pub const fn saturating_add(self, milliseconds: u64) -> Self {
        Self(self.0.saturating_add(milliseconds))
    }

    /// Milliseconds elapsed since an earlier tick, zero when `earlier` is not in
    /// the past.
    ///
    /// An out-of-order reading therefore counts as "no time has passed" rather
    /// than as a huge idle period or a wrapped deadline.
    #[must_use]
    pub const fn since(self, earlier: Self) -> u64 {
        self.0.saturating_sub(earlier.0)
    }

    /// Whether this tick has reached a deadline.
    #[must_use]
    pub const fn reached(self, deadline: Self) -> bool {
        self.0 >= deadline.0
    }
}

impl fmt::Display for Tick {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// A point in the object graph's history. The first published observation is
/// revision 1.
///
/// One counter per graph, bumped once per published observation, so a revision
/// is two things at once: a total order over everything the graph said, and the
/// stamp a subscription's snapshot carries so a client can tell which diffs it
/// already has. Revisions are deliberately not per object on the wire: a stream
/// is ordered globally, and a per-object counter would make two subscribers with
/// different filters disagree about which of two changes came first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(u64);

impl Revision {
    /// Before anything has been published.
    pub const ZERO: Self = Self(0);

    /// A revision with this number, for a wire decoder, a frame dump or a test.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw number.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The revision one step later, saturating at [`u64::MAX`].
    ///
    /// Saturation rather than wrapping means a graph that somehow published
    /// [`u64::MAX`] observations keeps handing out the newest stamp instead of
    /// issuing one a subscriber has already seen.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "revision {}", self.0)
    }
}
