//! The graph's revision counter.
//!
//! One counter per graph, bumped once per published observation. A revision is
//! therefore two things at once: a total order over everything the graph said,
//! and the stamp a subscription's snapshot carries so the client can tell which
//! diffs it already has.
//!
//! Revisions are deliberately not per object on the wire: a stream is ordered
//! globally, and a per-object counter would make two subscribers with different
//! filters disagree about which of two changes came first.

use std::fmt;

/// A point in the graph's history. The first published observation is revision 1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(u64);

impl Revision {
    /// Before anything has been published.
    pub const ZERO: Revision = Revision(0);

    /// Wrap a raw number, for a wire decoder or a test.
    pub const fn new(raw: u64) -> Revision {
        Revision(raw)
    }

    /// The raw number.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The revision one step later.
    pub(crate) const fn next(self) -> Revision {
        Revision(self.0 + 1)
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "revision {}", self.0)
    }
}
