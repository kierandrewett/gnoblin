//! A deterministic clock for observations.
//!
//! The parity harness strips absolute times before comparing, so a recorded
//! stamp never decides whether a scenario passes. It still has to exist, and a
//! wall clock would make two runs of the same scenario differ byte for byte,
//! which hides real differences in the noise of timestamps. This clock starts at
//! a fixed instant and advances by a fixed step, so a run is reproducible on any
//! machine.

/// 2026-01-01T00:00:00Z in milliseconds since the Unix epoch, the instant every
/// deterministic run starts from. A fixed, readable date makes an accidental
/// leak of a stamp into a comparison obvious rather than plausible.
pub const START_MILLIS: u64 = 1_767_225_600_000;

/// A millisecond clock that only moves when the test moves it.
#[derive(Debug, Clone)]
pub struct Clock {
    now_millis: u64,
    step_millis: u64,
}

impl Clock {
    /// A clock at [`START_MILLIS`] that moves one millisecond per tick.
    pub fn new() -> Self {
        Self::with_step(1)
    }

    /// A clock at [`START_MILLIS`] that moves `step_millis` per tick. A coarser
    /// step is useful when a scenario models seconds-long timeouts, and a zero
    /// step when a test wants every observation stamped identically.
    pub fn with_step(step_millis: u64) -> Self {
        Self {
            now_millis: START_MILLIS,
            step_millis,
        }
    }

    /// The current stamp, in milliseconds since the Unix epoch. Reading does not
    /// move the clock, so an observation can carry the same time as the one
    /// before it.
    pub fn now(&self) -> u64 {
        self.now_millis
    }

    /// Move one step and return the new stamp.
    pub fn tick(&mut self) -> u64 {
        self.now_millis = self.now_millis.saturating_add(self.step_millis);
        self.now_millis
    }

    /// Move by an explicit duration in milliseconds and return the new stamp.
    /// Saturation keeps a runaway scenario from wrapping into the epoch and
    /// making a nonsense comparison look ordered.
    pub fn advance_by(&mut self, millis: u64) -> u64 {
        self.now_millis = self.now_millis.saturating_add(millis);
        self.now_millis
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_starts_at_the_fixed_epoch() {
        assert_eq!(Clock::new().now(), START_MILLIS);
        assert_eq!(START_MILLIS, 1_767_225_600_000);
    }

    #[test]
    fn ticks_are_deterministic_and_monotonic() {
        let mut first = Clock::new();
        let mut second = Clock::new();
        let first_run: Vec<u64> = (0..3).map(|_| first.tick()).collect();
        let second_run: Vec<u64> = (0..3).map(|_| second.tick()).collect();
        assert_eq!(first_run, second_run, "two clocks must produce the same stamps");
        assert_eq!(first_run, vec![START_MILLIS + 1, START_MILLIS + 2, START_MILLIS + 3]);
    }

    #[test]
    fn a_wide_step_models_a_seconds_long_timeout() {
        let mut clock = Clock::with_step(1_500);
        assert_eq!(clock.tick(), START_MILLIS + 1_500);
        assert_eq!(clock.advance_by(10), START_MILLIS + 1_510);
    }

    #[test]
    fn reading_does_not_move_the_clock() {
        let clock = Clock::new();
        assert_eq!(clock.now(), clock.now());
    }
}
