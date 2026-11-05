//! The caller's monotonic clock, spelt the way the plan spells it.
//!
//! The guardian keeps no clock of its own: every [`Event`](crate::Event) carries
//! the moment it happened, as the shared [`gnoblin_types::Tick`] the rest of the
//! desktop uses, so one event sequence always produces one effect sequence and a
//! test can step past a deadline without sleeping.
//!
//! The one addition is [`secs`]. A plan is written for a person to read, so its
//! deadlines are seconds (`ready_within = 10`, `stable_after = 60`) and the
//! conversion happens once, here, instead of a `10_000` scattered through the
//! defaults. It saturates like every other tick operation: a deadline that
//! cannot be represented becomes the end of the range, never a wrapped instant
//! in the past.
//!
//! # Failure modes
//!
//! - **A seconds deadline overflows the millisecond range**, which would turn a
//!   deadline in the future into one in the past and expire a token or a grant
//!   before it was sent. [`secs`] saturates. Covered by
//!   `a_seconds_deadline_saturates_instead_of_wrapping`.

pub use gnoblin_types::Tick;

/// A moment `seconds` after the clock's origin, saturating rather than wrapping
/// when `seconds` cannot be represented in milliseconds.
#[must_use]
pub const fn secs(seconds: u64) -> Tick {
    Tick::new(seconds.saturating_mul(1000))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seconds_deadline_saturates_instead_of_wrapping() {
        assert_eq!(secs(0), Tick::ZERO);
        assert_eq!(secs(1), Tick::new(1_000));
        assert_eq!(secs(60), Tick::new(60_000));
        assert_eq!(secs(u64::MAX), Tick::new(u64::MAX));
    }
}
