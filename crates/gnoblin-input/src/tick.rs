//! The caller's clock, as a tick counter.
//!
//! Nothing in this crate reads a clock: the compositor advances a tick once per
//! input poll and passes the value down, so the same event sequence with the same
//! ticks always produces the same decisions and a test can reason about deadlines
//! and idle time in whole ticks. One tick is not a fixed duration; the caller
//! converts its budget (16 ms for a policy decision, 30 s for a shortcut capture)
//! into ticks before calling.

use std::fmt;

/// A point on the caller's tick counter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tick(u64);

impl Tick {
    /// The first tick.
    pub const ZERO: Self = Self(0);

    /// The tick with this raw value.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw counter value, for logging and for callers that keep their own clock.
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// The tick this many ticks later, saturating instead of wrapping.
    ///
    /// A counter that wrapped would turn a deadline in the future into one in the
    /// past, and a policy decision would expire before it was ever sent. Saturating
    /// keeps the deadline unreachable instead, which is the safe direction.
    pub const fn saturating_add(self, ticks: u64) -> Self {
        Self(self.0.saturating_add(ticks))
    }

    /// Ticks elapsed since an earlier tick, zero when `earlier` is not in the past.
    ///
    /// An out-of-order tick therefore reads as "no time has passed" rather than as
    /// a huge idle period or a wrapped deadline.
    pub const fn since(self, earlier: Self) -> u64 {
        self.0.saturating_sub(earlier.0)
    }

    /// Whether this tick has reached a deadline.
    pub const fn reached(self, deadline: Self) -> bool {
        self.0 >= deadline.0
    }
}

impl fmt::Display for Tick {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::Tick;

    #[test]
    fn arithmetic_saturates_instead_of_wrapping() {
        assert_eq!(Tick::new(4).since(Tick::new(10)), 0);
        assert_eq!(Tick::new(10).since(Tick::new(4)), 6);
        assert_eq!(Tick::new(u64::MAX - 1).saturating_add(9), Tick::new(u64::MAX));
        assert!(Tick::new(9).reached(Tick::new(9)));
        assert!(!Tick::new(8).reached(Tick::new(9)));
    }
}
