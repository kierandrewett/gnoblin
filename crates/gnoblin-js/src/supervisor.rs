//! The strike policy for a generation that was interrupted or that panicked.
//!
//! An interrupted script skipped its `finally` blocks and a panic unwound through the host's frames, so
//! the generation it ran in is suspect (design/js-research/quickjs-embedding.md, section 6). The
//! recovery is to stage a fresh generation from the files on disk. A config that always trips the
//! budget would then trip it again, so the supervisor counts strikes: `max_strikes` faults within
//! `window` latch [`JsHealth::Degraded`] and the caller stops restaging until the `cool_down` has passed
//! or the user reloads on purpose.
//!
//! This is pure logic. It reads a [`JsClock`] and nothing else: it spawns nothing and touches no file,
//! so the strike counting and the cool-down are tested with a clock the test moves.

use std::collections::VecDeque;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::error::{JsError, JsErrorKind};

/// A monotonic clock, injectable so the policy can be tested without sleeping.
pub trait JsClock {
    /// The time since an arbitrary fixed origin. It never goes backwards.
    fn now(&self) -> Duration;
}

/// The real clock: time since the first read in this process.
///
/// A fixed process-wide origin keeps the type a unit struct, so any number of supervisors share it.
#[derive(Debug, Clone, Copy, Default)]
pub struct JsSystemClock;

impl JsClock for JsSystemClock {
    fn now(&self) -> Duration {
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        ORIGIN.get_or_init(Instant::now).elapsed()
    }
}

/// How many faults, in what span, and how long to stay degraded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsStrikePolicy {
    /// The number of faults inside `window` that latches [`JsHealth::Degraded`].
    pub max_strikes: usize,
    /// A fault counts against the generation this long.
    pub window: Duration,
    /// Once degraded, the supervisor refuses to restage for this long after it latched.
    pub cool_down: Duration,
}

impl Default for JsStrikePolicy {
    /// Three strikes in 30 s, then five minutes without retries.
    fn default() -> Self {
        Self {
            max_strikes: 3,
            window: Duration::from_secs(30),
            cool_down: Duration::from_secs(300),
        }
    }
}

/// A fault that makes the running generation suspect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsFault {
    /// A call or a staging ran out of its wall-clock budget and was interrupted.
    Budget,
    /// A host callback panicked while the generation was running.
    Panic,
}

impl JsFault {
    /// The fault `error` is, if it is one. Only an interrupt counts: a thrown exception is the script
    /// reporting its own failure and leaves the generation consistent, and a memory error is catchable.
    pub fn from_error(error: &JsError) -> Option<Self> {
        (error.kind == JsErrorKind::Budget).then_some(Self::Budget)
    }
}

/// Whether the runtime may keep restaging itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsHealth {
    /// Faults, if any, are within the policy.
    Healthy,
    /// The strike limit was reached; no restage happens until the cool-down passes.
    Degraded {
        /// When the last strike latched this, on the supervisor's clock.
        since: Duration,
    },
}

/// What to do about a fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsFaultAction {
    /// Discard the suspect generation and stage a fresh one from disk.
    Restage,
    /// Stop retrying. Keep the last good generation if there is one, otherwise run none.
    GiveUp,
}

/// Counts faults and decides between restaging and giving up.
#[derive(Debug)]
pub struct JsSupervisor<C = JsSystemClock> {
    policy: JsStrikePolicy,
    clock: C,
    /// Times of the faults still inside the window, oldest first.
    strikes: VecDeque<Duration>,
    degraded_since: Option<Duration>,
    last_fault: Option<JsFault>,
}

impl JsSupervisor {
    /// A supervisor on the real clock with the default policy.
    pub fn with_defaults() -> Self {
        Self::new(JsStrikePolicy::default(), JsSystemClock)
    }
}

impl<C: JsClock> JsSupervisor<C> {
    /// A healthy supervisor with no strikes.
    pub fn new(policy: JsStrikePolicy, clock: C) -> Self {
        Self {
            policy,
            clock,
            strikes: VecDeque::new(),
            degraded_since: None,
            last_fault: None,
        }
    }

    /// Counts `fault` and says what to do about it.
    ///
    /// While degraded and inside the cool-down the fault is not counted: nothing is being retried, so
    /// nothing can strike. After the cool-down the slate is clean and this fault is the first strike.
    pub fn record_fault(&mut self, fault: JsFault) -> JsFaultAction {
        // Both kinds weigh the same; the kind is kept so a report can say what struck last.
        self.last_fault = Some(fault);
        let now = self.clock.now();
        if let Some(since) = self.degraded_since {
            if now.saturating_sub(since) < self.policy.cool_down {
                return JsFaultAction::GiveUp;
            }
            self.degraded_since = None;
            self.strikes.clear();
        }
        self.prune(now);
        self.strikes.push_back(now);
        if self.strikes.len() >= self.policy.max_strikes {
            self.degraded_since = Some(now);
            return JsFaultAction::GiveUp;
        }
        JsFaultAction::Restage
    }

    /// Whether restaging is allowed now.
    pub fn health(&self) -> JsHealth {
        match self.degraded_since {
            Some(since) if self.clock.now().saturating_sub(since) < self.policy.cool_down => {
                JsHealth::Degraded { since }
            }
            _ => JsHealth::Healthy,
        }
    }

    /// The most recent fault recorded, for a report.
    pub fn last_fault(&self) -> Option<JsFault> {
        self.last_fault
    }

    /// The faults still inside the window.
    pub fn strikes(&self) -> usize {
        let now = self.clock.now();
        self.strikes
            .iter()
            .filter(|at| now.saturating_sub(**at) < self.policy.window)
            .count()
    }

    /// The user asked for a reload, by saving a file or by `gnoblinctl reload`. That is a deliberate
    /// retry with possibly fixed code, so the strikes and the degraded state are cleared.
    pub fn user_reload(&mut self) {
        self.strikes.clear();
        self.degraded_since = None;
        self.last_fault = None;
    }

    /// Drops the strikes that have left the window.
    fn prune(&mut self, now: Duration) {
        while self
            .strikes
            .front()
            .is_some_and(|at| now.saturating_sub(*at) >= self.policy.window)
        {
            self.strikes.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    use super::{JsClock, JsFault, JsFaultAction, JsHealth, JsStrikePolicy, JsSupervisor};
    use crate::error::{JsError, JsErrorKind};

    /// A clock the test moves by hand.
    #[derive(Clone, Default)]
    struct FakeClock(Rc<Cell<Duration>>);

    impl FakeClock {
        fn advance(&self, by: Duration) {
            self.0.set(self.0.get() + by);
        }
    }

    impl JsClock for FakeClock {
        fn now(&self) -> Duration {
            self.0.get()
        }
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn supervisor() -> (JsSupervisor<FakeClock>, FakeClock) {
        let clock = FakeClock::default();
        // Start away from zero so "since" values are distinguishable from a default.
        clock.advance(secs(1000));
        (JsSupervisor::new(JsStrikePolicy::default(), clock.clone()), clock)
    }

    #[test]
    fn the_first_two_faults_restage_and_the_third_gives_up() {
        let (mut sup, clock) = supervisor();
        assert_eq!(sup.health(), JsHealth::Healthy);
        assert_eq!(sup.record_fault(JsFault::Budget), JsFaultAction::Restage);
        clock.advance(secs(5));
        assert_eq!(sup.record_fault(JsFault::Panic), JsFaultAction::Restage);
        assert_eq!(sup.strikes(), 2);
        assert_eq!(sup.last_fault(), Some(JsFault::Panic));
        clock.advance(secs(5));
        assert_eq!(sup.record_fault(JsFault::Budget), JsFaultAction::GiveUp);
        assert_eq!(sup.health(), JsHealth::Degraded { since: secs(1010) });
    }

    #[test]
    fn strikes_older_than_the_window_do_not_count() {
        let (mut sup, clock) = supervisor();
        assert_eq!(sup.record_fault(JsFault::Budget), JsFaultAction::Restage);
        assert_eq!(sup.record_fault(JsFault::Budget), JsFaultAction::Restage);
        // The two strikes were 30 s ago: exactly the window, so they have aged out.
        clock.advance(secs(30));
        assert_eq!(sup.strikes(), 0);
        assert_eq!(sup.record_fault(JsFault::Budget), JsFaultAction::Restage);
        assert_eq!(sup.strikes(), 1);
        assert_eq!(sup.health(), JsHealth::Healthy);
    }

    #[test]
    fn a_strike_just_inside_the_window_still_counts() {
        let (mut sup, clock) = supervisor();
        assert_eq!(sup.record_fault(JsFault::Budget), JsFaultAction::Restage);
        clock.advance(secs(15));
        assert_eq!(sup.record_fault(JsFault::Budget), JsFaultAction::Restage);
        clock.advance(secs(14));
        // The first strike is 29 s old.
        assert_eq!(sup.record_fault(JsFault::Budget), JsFaultAction::GiveUp);
    }

    #[test]
    fn degraded_refuses_until_the_cool_down_passes_and_then_starts_clean() {
        let (mut sup, clock) = supervisor();
        for _ in 0..3 {
            sup.record_fault(JsFault::Budget);
        }
        assert!(matches!(sup.health(), JsHealth::Degraded { .. }));

        // Faults inside the cool-down are refused and are not counted.
        clock.advance(secs(299));
        assert_eq!(sup.record_fault(JsFault::Budget), JsFaultAction::GiveUp);
        assert!(matches!(sup.health(), JsHealth::Degraded { .. }));

        clock.advance(secs(1));
        assert_eq!(sup.health(), JsHealth::Healthy);
        assert_eq!(sup.record_fault(JsFault::Budget), JsFaultAction::Restage);
        assert_eq!(sup.strikes(), 1);
    }

    #[test]
    fn a_user_reload_clears_strikes_and_the_degraded_state() {
        let (mut sup, _clock) = supervisor();
        for _ in 0..3 {
            sup.record_fault(JsFault::Budget);
        }
        assert!(matches!(sup.health(), JsHealth::Degraded { .. }));
        sup.user_reload();
        assert_eq!(sup.health(), JsHealth::Healthy);
        assert_eq!(sup.strikes(), 0);
        assert_eq!(sup.record_fault(JsFault::Budget), JsFaultAction::Restage);
    }

    #[test]
    fn a_custom_policy_is_honoured() {
        let clock = FakeClock::default();
        let policy = JsStrikePolicy {
            max_strikes: 1,
            window: secs(1),
            cool_down: secs(2),
        };
        let mut sup = JsSupervisor::new(policy, clock.clone());
        assert_eq!(sup.record_fault(JsFault::Panic), JsFaultAction::GiveUp);
        clock.advance(secs(2));
        assert_eq!(sup.health(), JsHealth::Healthy);
    }

    #[test]
    fn only_a_budget_interrupt_is_a_fault() {
        let budget = JsError {
            kind: JsErrorKind::Budget,
            message: "interrupted".to_owned(),
            location: None,
            stack: None,
        };
        assert_eq!(JsFault::from_error(&budget), Some(JsFault::Budget));
        for kind in [
            JsErrorKind::Exception,
            JsErrorKind::Ts,
            JsErrorKind::Load,
            JsErrorKind::Memory,
            JsErrorKind::Engine,
        ] {
            let error = JsError {
                kind,
                message: String::new(),
                location: None,
                stack: None,
            };
            assert_eq!(JsFault::from_error(&error), None, "{kind:?}");
        }
    }
}
