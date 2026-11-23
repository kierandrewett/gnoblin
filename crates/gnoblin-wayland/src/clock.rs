//! The source of the session clock's time.
//!
//! The loop reads a [`Clock`] once per turn and advances the session clock by the time
//! that passed since the previous turn. [`SystemClock`] is the real clock a session uses
//! by default. [`ManualClock`] moves only when a test moves it, so a rest or idle timer
//! never expires during a round trip the test did not ask for.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A source of monotonic time for the session clock.
pub trait Clock: Send + Sync + std::fmt::Debug {
    /// The current instant. It never runs backwards.
    fn now(&self) -> Instant;
}

/// The real monotonic clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// A clock that moves only when [`ManualClock::advance`] is called.
#[derive(Debug)]
pub struct ManualClock {
    origin: Instant,
    elapsed: Mutex<Duration>,
}

impl ManualClock {
    /// A clock at its origin, the moment it was made.
    #[must_use]
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
            elapsed: Mutex::new(Duration::ZERO),
        }
    }

    /// Moves the clock forward by `by`.
    pub fn advance(&self, by: Duration) {
        *self.elapsed.lock().expect("the clock lock") += by;
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        self.origin + *self.elapsed.lock().expect("the clock lock")
    }
}

/// What the loop makes of its reads of a [`Clock`]: the whole milliseconds that passed since the previous turn.
///
/// The session clock counts milliseconds, and the loop turns far more often than once a millisecond while input
/// arrives or a client commits, so the time between two turns is rarely a whole number of them. A turn that dropped the
/// rest of its time would make the session clock run slow by half a millisecond a turn, on average: at a hundred turns
/// a second the two-second lease of the lock screen would last two seconds and a tenth on the wall clock. The rest is
/// kept for the next turn instead, so the count is never ahead of the clock and is never a millisecond behind it.
#[derive(Debug, Default)]
pub(crate) struct TurnClock {
    /// The instant up to which time has been counted: the last turn's read, less the rest that did not fill a
    /// millisecond.
    counted_to: Option<Instant>,
}

impl TurnClock {
    /// The whole milliseconds between what has been counted and `now`, or `None` on the first turn, which only fixes
    /// where counting starts.
    pub(crate) fn elapsed_ms(&mut self, now: Instant) -> Option<u64> {
        let Some(counted_to) = self.counted_to else {
            self.counted_to = Some(now);
            return None;
        };
        let whole = u64::try_from(now.saturating_duration_since(counted_to).as_millis()).unwrap_or(u64::MAX);
        self.counted_to = Some(counted_to.checked_add(Duration::from_millis(whole)).unwrap_or(now));
        Some(whole)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRACTIONAL: Duration = Duration::from_micros(500);

    #[test]
    fn the_first_turn_only_starts_the_count() {
        let mut turns = TurnClock::default();
        assert_eq!(turns.elapsed_ms(Instant::now()), None);
    }

    #[test]
    fn turns_of_whole_milliseconds_count_exactly() {
        let origin = Instant::now();
        let mut turns = TurnClock::default();
        turns.elapsed_ms(origin);
        assert_eq!(turns.elapsed_ms(origin + Duration::from_millis(7)), Some(7));
        assert_eq!(turns.elapsed_ms(origin + Duration::from_millis(7)), Some(0));
        assert_eq!(turns.elapsed_ms(origin + Duration::from_millis(1_007)), Some(1_000));
    }

    /// Input arrives faster than a turn a millisecond: a second of turns half a millisecond apart is a second.
    #[test]
    fn turns_shorter_than_a_millisecond_lose_no_time() {
        let origin = Instant::now();
        let mut turns = TurnClock::default();
        turns.elapsed_ms(origin);
        let counted: u64 = (1..=2_000u32)
            .map(|turn| turns.elapsed_ms(origin + FRACTIONAL * turn).expect("a later turn"))
            .sum();
        assert_eq!(counted, 1_000, "2000 turns of half a millisecond are a second");
    }

    /// Turns of 1.7 ms never add up to more than the wall clock's time, and fall short of it by less than a millisecond:
    /// a lease must not lapse early, and must not lapse late by a share of its length.
    #[test]
    fn the_count_is_never_ahead_of_the_clock_and_never_a_millisecond_behind() {
        let origin = Instant::now();
        let mut turns = TurnClock::default();
        turns.elapsed_ms(origin);
        let mut counted = 0;
        for turn in 1..=3_000u32 {
            let now = origin + Duration::from_micros(1_700) * turn;
            counted += turns.elapsed_ms(now).expect("a later turn");
            let real = now.duration_since(origin).as_micros();
            assert!(
                u128::from(counted) * 1_000 <= real,
                "turn {turn}: counted {counted} ms of {real} µs"
            );
            assert!(
                real - u128::from(counted) * 1_000 < 1_000,
                "turn {turn}: counted {counted} ms of {real} µs"
            );
        }
    }

    #[test]
    fn a_clock_that_has_not_moved_counts_nothing() {
        let origin = Instant::now();
        let mut turns = TurnClock::default();
        turns.elapsed_ms(origin);
        assert_eq!(turns.elapsed_ms(origin), Some(0));
        assert_eq!(turns.elapsed_ms(origin + FRACTIONAL), Some(0));
        assert_eq!(turns.elapsed_ms(origin + FRACTIONAL), Some(0));
        assert_eq!(turns.elapsed_ms(origin + FRACTIONAL * 2), Some(1));
    }

    #[test]
    fn a_long_gap_between_turns_counts_in_full() {
        let origin = Instant::now();
        let mut turns = TurnClock::default();
        turns.elapsed_ms(origin);
        let hour = Duration::from_secs(3_600) + FRACTIONAL;
        assert_eq!(turns.elapsed_ms(origin + hour), Some(3_600_000));
        assert_eq!(turns.elapsed_ms(origin + hour + FRACTIONAL), Some(1));
    }
}
