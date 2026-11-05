//! The caller's monotonic clock, and the arithmetic the guardian does on it.
//!
//! This module is the one place to change when `gnoblin-types` becomes a
//! dependency: `gnoblin_types::Tick` is the same caller-supplied millisecond
//! reading with the same saturating arithmetic, so the swap is
//! `Millis::from_millis` to `Tick::new`, `Millis::as_millis` to `Tick::get` and
//! `elapsed_since` to `since`. `saturating_mul` is the only operation the
//! guardian needs beyond what `Tick` offers today, and it exists for the backoff
//! ladder's doubling step.

/// A moment on the caller's monotonic clock, in milliseconds.
///
/// The guardian holds no clock of its own: every [`crate::Event`] carries the
/// time it happened, so one event sequence always produces one effect sequence
/// and a test can step past a deadline without sleeping. Every operation here
/// saturates instead of wrapping: a session that has been up for longer than a
/// `u64` of milliseconds must still stop cleanly, and a restart ladder must not
/// wrap into an immediate restart, which is the "backoff overflow" failure mode.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Millis(u64);

impl Millis {
    /// The instant the caller's clock started at, which is the only origin the
    /// guardian assumes.
    pub const ZERO: Self = Self(0);

    /// A moment `value` milliseconds after the clock's origin.
    #[must_use]
    pub const fn from_millis(value: u64) -> Self {
        Self(value)
    }

    /// A moment `seconds` after the clock's origin, saturating rather than
    /// wrapping when `seconds` cannot be represented in milliseconds.
    #[must_use]
    pub const fn from_secs(seconds: u64) -> Self {
        match seconds.checked_mul(1000) {
            Some(value) => Self(value),
            None => Self(u64::MAX),
        }
    }

    /// The moment as a count of milliseconds.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.0
    }

    /// This moment plus `other`, pinned at [`u64::MAX`] instead of wrapping.
    #[must_use]
    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    /// This moment minus `other`, pinned at [`Millis::ZERO`] instead of
    /// wrapping, so an event whose timestamp went backwards reads as no elapsed
    /// time rather than as an enormous one.
    #[must_use]
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    /// This moment scaled by `factor`, pinned at [`u64::MAX`] instead of
    /// wrapping. A doubling ladder reaches its cap long before this matters, but
    /// a caller-supplied rule can ask for anything.
    #[must_use]
    pub const fn saturating_mul(self, factor: u64) -> Self {
        Self(self.0.saturating_mul(factor))
    }

    /// How long has passed since `origin`, saturating at zero.
    #[must_use]
    pub const fn elapsed_since(self, origin: Self) -> Self {
        self.saturating_sub(origin)
    }
}
