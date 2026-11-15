//! The memory an output holds, published on its screen node (`gpu_memory_*`, design/gpu-memory.md).
//!
//! The numbers come from two places that only this crate sees together: the renderer's own footprint (the backend
//! states it) and the frames capture keeps. They are published at most once a period (a second by default,
//! [`crate::Config::gpu_memory_period`]), and the tree announces a value only when it differs from the node's, so a
//! steady session publishes nothing. The first publication comes a period after the session starts. A publication is a
//! change batch and advances the tree's serial, so a test that reads the serial sets the period to zero, which never
//! publishes. Read-only: nothing here limits or changes an allocation.

use std::time::{Duration, Instant};

use gnoblin_backend::Backend;

use crate::State;

/// How often the numbers are published by default: a staging buffer or a snapshot changes with every drawn frame, and a
/// tree change is a message to every observer.
pub(crate) const DEFAULT_PERIOD: Duration = Duration::from_secs(1);

/// When the next publication is due.
#[derive(Debug)]
pub(crate) struct Publisher {
    period: Duration,
    next: Option<Instant>,
}

impl Publisher {
    /// A publisher that publishes every `period`, or never when it is zero.
    pub(crate) fn new(period: Duration) -> Self {
        Self { period, next: None }
    }

    /// Whether a publication is due at `now`. The first call starts the period and answers no.
    fn due(&mut self, now: Instant) -> bool {
        if self.period.is_zero() {
            return false;
        }
        match self.next {
            None => {
                self.next = Some(now + self.period);
                false
            }
            Some(next) if now >= next => {
                self.next = Some(now + self.period);
                true
            }
            Some(_) => false,
        }
    }
}

impl State {
    /// Publishes, for every output, the bytes its renderer holds and the bytes capture keeps of its last frame.
    pub(crate) fn publish_gpu_memory(&mut self, backend: &dyn Backend<State>) {
        if !self.gpu_memory.due(Instant::now()) {
            return;
        }
        for output in self.output_ids() {
            let footprint = backend.footprint(output);
            let capture = self.capture_cache_bytes(output);
            self.composer.set_output_memory(output, &footprint, capture);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_publication_waits_a_period_and_later_ones_are_rate_limited() {
        let mut publisher = Publisher::new(DEFAULT_PERIOD);
        let start = Instant::now();
        assert!(!publisher.due(start), "the first call only starts the period");
        assert!(!publisher.due(start + Duration::from_millis(500)));
        assert!(publisher.due(start + DEFAULT_PERIOD));
        assert!(!publisher.due(start + DEFAULT_PERIOD + Duration::from_millis(1)));
        assert!(publisher.due(start + DEFAULT_PERIOD * 2));
    }

    #[test]
    fn a_zero_period_never_publishes() {
        let mut publisher = Publisher::new(Duration::ZERO);
        let start = Instant::now();
        for step in 0..5 {
            assert!(!publisher.due(start + Duration::from_secs(step * 10)));
        }
    }
}
