//! The retry limiter: wrong passwords slow the next attempt down, so guessing at the lock is slow.
//!
//! Time is a monotonic millisecond tick supplied by the caller, so the limiter reads no clock and
//! its tests run on fixed numbers. A caller asks [`RateLimiter::allows`] before each attempt and
//! reports the result with [`RateLimiter::record`].

use crate::AuthOutcome;

/// Consecutive denials that start a delay.
pub const DENIALS_BEFORE_DELAY: u32 = 3;

/// How long, in milliseconds, the next attempt waits once the denials reach the threshold.
pub const DELAY_MS: u64 = 2_000;

/// Counts consecutive denied attempts and holds off the next one after a run of them.
///
/// `Denied` counts towards the threshold. `Accepted` clears the count and any delay. `Failed` says
/// nothing about the password, so it neither counts nor resets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RateLimiter {
    denials: u32,
    blocked_until_ms: Option<u64>,
}

impl RateLimiter {
    /// A limiter with no denials on record. Every attempt is allowed.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether an attempt may be made at `now_ms`.
    pub fn allows(&self, now_ms: u64) -> bool {
        self.blocked_until_ms.is_none_or(|until| now_ms >= until)
    }

    /// How many milliseconds after `now_ms` the next attempt may be made: zero when one may be made
    /// now.
    pub fn retry_after_ms(&self, now_ms: u64) -> u64 {
        self.blocked_until_ms.map_or(0, |until| until.saturating_sub(now_ms))
    }

    /// Record the outcome of an attempt made at `now_ms`.
    pub fn record(&mut self, outcome: &AuthOutcome, now_ms: u64) {
        match outcome {
            AuthOutcome::Accepted => {
                self.denials = 0;
                self.blocked_until_ms = None;
            }
            AuthOutcome::Denied => {
                self.denials += 1;
                if self.denials >= DENIALS_BEFORE_DELAY {
                    // The run is spent: the next run of denials earns its own delay.
                    self.denials = 0;
                    self.blocked_until_ms = Some(now_ms.saturating_add(DELAY_MS));
                }
            }
            AuthOutcome::Failed(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn denied() -> AuthOutcome {
        AuthOutcome::Denied
    }

    fn failed() -> AuthOutcome {
        AuthOutcome::Failed("no pam".to_owned())
    }

    #[test]
    fn a_new_limiter_allows_an_attempt_at_any_time() {
        let limiter = RateLimiter::new();
        assert!(limiter.allows(0));
        assert_eq!(limiter.retry_after_ms(0), 0);
    }

    #[test]
    fn two_denials_do_not_hold_the_next_attempt_off() {
        let mut limiter = RateLimiter::new();
        limiter.record(&denied(), 1_000);
        limiter.record(&denied(), 1_100);
        assert!(limiter.allows(1_100));
        assert_eq!(limiter.retry_after_ms(1_100), 0);
    }

    #[test]
    fn three_consecutive_denials_hold_the_next_attempt_off_for_two_seconds() {
        let mut limiter = RateLimiter::new();
        for _ in 0..DENIALS_BEFORE_DELAY {
            limiter.record(&denied(), 1_000);
        }
        assert!(!limiter.allows(1_000));
        assert!(!limiter.allows(2_999));
        assert_eq!(limiter.retry_after_ms(1_500), 1_500);
        assert!(limiter.allows(3_000));
        assert_eq!(limiter.retry_after_ms(3_000), 0);
    }

    #[test]
    fn the_delay_is_two_seconds() {
        assert_eq!(DELAY_MS, 2_000);
        assert_eq!(DENIALS_BEFORE_DELAY, 3);
    }

    #[test]
    fn an_accepted_attempt_resets_the_count() {
        let mut limiter = RateLimiter::new();
        limiter.record(&denied(), 0);
        limiter.record(&denied(), 0);
        limiter.record(&AuthOutcome::Accepted, 0);
        limiter.record(&denied(), 0);
        limiter.record(&denied(), 0);
        assert!(limiter.allows(0));
    }

    #[test]
    fn an_accepted_attempt_clears_an_active_delay() {
        let mut limiter = RateLimiter::new();
        for _ in 0..DENIALS_BEFORE_DELAY {
            limiter.record(&denied(), 0);
        }
        limiter.record(&AuthOutcome::Accepted, 100);
        assert!(limiter.allows(100));
    }

    #[test]
    fn a_failed_check_neither_counts_nor_resets() {
        let mut limiter = RateLimiter::new();
        limiter.record(&denied(), 0);
        limiter.record(&denied(), 0);
        limiter.record(&failed(), 0);
        limiter.record(&denied(), 0);
        assert!(!limiter.allows(0));
    }

    #[test]
    fn after_a_delay_a_fresh_run_of_three_denials_is_needed() {
        let mut limiter = RateLimiter::new();
        for _ in 0..DENIALS_BEFORE_DELAY {
            limiter.record(&denied(), 0);
        }
        assert!(limiter.allows(DELAY_MS));
        limiter.record(&denied(), DELAY_MS);
        limiter.record(&denied(), DELAY_MS);
        assert!(limiter.allows(DELAY_MS));
        limiter.record(&denied(), DELAY_MS);
        assert!(!limiter.allows(DELAY_MS));
        assert!(limiter.allows(2 * DELAY_MS));
    }

    #[test]
    fn a_delay_near_the_end_of_time_saturates_instead_of_wrapping() {
        let mut limiter = RateLimiter::new();
        for _ in 0..DENIALS_BEFORE_DELAY {
            limiter.record(&denied(), u64::MAX);
        }
        assert!(!limiter.allows(u64::MAX - 1));
        assert_eq!(limiter.retry_after_ms(u64::MAX), 0);
    }
}
