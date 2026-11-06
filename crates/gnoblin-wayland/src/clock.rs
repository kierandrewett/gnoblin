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
