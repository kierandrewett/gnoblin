//! The callback budget.
//!
//! Native code cannot be interrupted. A callback that runs for longer than [`CALLBACK_BUDGET`] (2 s) is therefore *observed*: a
//! watchdog thread sees it run past the budget, logs it once with the registration and the generation, and counts a **strike**
//! against the generation while the callback is still running, so a callback that never returns is counted all the same. The
//! policy is the script runtimes' (`gnoblin_shell_host::supervisor`): three strikes within 30 s latch `Degraded`.
//!
//! What a strike changes:
//!
//! - It is visible: the log line, the counters of `host::budget_status()` and the summary of `gnoblinctl shell status`.
//! - **`Degraded` stops the shell from restaging itself.** While it is latched a reload the *shell* asked for
//!   (`request_reload`) is refused and the running generation, the last good one, keeps running; a deliberate reload (`gnoblinctl
//!   shell reload`, or a rebuilt library the watcher saw) is a retry with possibly fixed code and clears the strikes. The latch
//!   lapses by itself after the policy's cool-down (five minutes).
//! - **A generation whose `init` overran [`INIT_BUDGET`] (5 s, the boot wait) does not take over a reload.** Its `init` returned,
//!   but the shell thread was held that long with the old generation waiting, so the reload is abandoned (what it registered and
//!   wrote is dropped) and the last good generation keeps serving. The first generation has no predecessor, so it only strikes.
//!
//! Nothing here stops the callback: a hung callback keeps the shell thread, and only the shell's own events with it. The
//! compositor never runs shell code (section 5.4), and the lock lease is what protects a locked session (`native_shell_failures.rs`).
//!
//! The bookkeeping is [`BudgetTracker`], pure over a [`Clock`], so the strike counting is tested with a clock the test moves; the
//! watchdog thread only asks it to sweep.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard, Once, PoisonError};
use std::time::Duration;

use gnoblin_shell_host::supervisor::{
    Clock, FaultAction, GenerationFault, Health, StrikePolicy, StrikeSupervisor, SystemClock,
};

use crate::log;

/// How long a callback may run before it counts as a strike.
pub const CALLBACK_BUDGET: Duration = Duration::from_secs(2);

/// How long a generation's `init` may run before it counts as a strike (and, in a reload, before the generation is refused): the
/// boot wait of `tree_attach`.
pub const INIT_BUDGET: Duration = Duration::from_secs(5);

/// The two limits. [`BudgetLimits::default`] is the design's; tests and drivers give shorter ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BudgetLimits {
    /// A callback's limit.
    pub callback: Duration,
    /// A generation's `init` limit.
    pub init: Duration,
}

impl Default for BudgetLimits {
    fn default() -> Self {
        Self {
            callback: CALLBACK_BUDGET,
            init: INIT_BUDGET,
        }
    }
}

/// One callback or `init` that is running now.
struct WatchedCall {
    token: u64,
    generation: u64,
    what: Arc<str>,
    since: Duration,
    limit: Duration,
    init: bool,
    struck: bool,
}

/// A callback or `init` that ran past its limit, as the sweep saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetOverrun {
    /// The generation it ran in.
    pub generation: u64,
    /// What ran: `on_signal window.request`, `timer_add`, `init`.
    pub what: String,
    /// The limit it passed.
    pub limit: Duration,
    /// Whether it was the generation's `init`.
    pub init: bool,
    /// The strikes inside the window, this one included.
    pub strikes: usize,
    /// Whether this strike latched `Degraded`.
    pub latched: bool,
    /// Whether the generation was already degraded, and so this one was not counted.
    pub uncounted: bool,
}

impl fmt::Display for BudgetOverrun {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} of generation {} ran past its {:?} budget",
            self.what, self.generation, self.limit
        )
    }
}

/// How a callback that struck ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchEnded {
    /// How long it ran in all.
    pub ran: Duration,
    /// The generation it ran in.
    pub generation: u64,
    /// What it was.
    pub what: Arc<str>,
    /// Whether it had struck while it ran.
    pub struck: bool,
}

/// What `gnoblinctl shell status` shows about the budget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetStatus {
    /// The limits in force.
    pub limits: BudgetLimits,
    /// Whether the shell is degraded now: three strikes inside the window, and the cool-down has not passed.
    pub degraded: bool,
    /// The strikes inside the window now.
    pub strikes: usize,
    /// Overruns seen since the shell instance started, counted or not.
    pub overruns: u64,
    /// The strikes of each generation that has had one, in generation order.
    pub by_generation: Vec<(u64, u64)>,
    /// The last overrun, in words.
    pub last: Option<String>,
    /// The callback running now that has run past its limit: what it is and for how long.
    pub stuck: Option<(String, Duration)>,
}

impl fmt::Display for BudgetStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "callback budget {:?}: {} strike(s) in the last 30 s, {} overrun(s) in all",
            self.limits.callback, self.strikes, self.overruns
        )?;
        if self.degraded {
            write!(
                f,
                "; DEGRADED: the shell's own reload requests are refused and the last good generation keeps running \
                 (`gnoblinctl shell reload` retries)"
            )?;
        }
        if let Some((what, ran)) = &self.stuck {
            write!(f, "; {what} has run for {ran:.1?} and has not returned")?;
        }
        if let Some(last) = &self.last {
            write!(f, "; last: {last}")?;
        }
        Ok(())
    }
}

/// The bookkeeping: what is running, the strike policy and what has been counted.
pub struct BudgetTracker<C: Clock> {
    clock: C,
    limits: BudgetLimits,
    next_token: u64,
    running: Vec<WatchedCall>,
    supervisor: StrikeSupervisor<C>,
    overruns: u64,
    per_generation: HashMap<u64, u64>,
    init_overran: HashSet<u64>,
    last: Option<String>,
}

impl<C: Clock + Clone> BudgetTracker<C> {
    /// A tracker with nothing running and no strikes.
    pub fn new(clock: C, limits: BudgetLimits) -> Self {
        Self::with_policy(clock, limits, StrikePolicy::default())
    }

    /// [`BudgetTracker::new`] with a strike policy of its own.
    pub fn with_policy(clock: C, limits: BudgetLimits, policy: StrikePolicy) -> Self {
        Self {
            supervisor: StrikeSupervisor::new(policy, clock.clone()),
            clock,
            limits,
            next_token: 1,
            running: Vec::new(),
            overruns: 0,
            per_generation: HashMap::new(),
            init_overran: HashSet::new(),
            last: None,
        }
    }

    /// Replaces the limits. A callback already running keeps the one it started under.
    pub fn set_limits(&mut self, limits: BudgetLimits) {
        self.limits = limits;
    }

    /// Starts watching a callback (or, when `init`, a generation's `init`); the token ends the watch.
    pub fn enter(&mut self, generation: u64, what: impl Into<Arc<str>>, init: bool) -> u64 {
        let token = self.next_token;
        self.next_token += 1;
        self.running.push(WatchedCall {
            token,
            generation,
            what: what.into(),
            since: self.clock.now(),
            limit: if init { self.limits.init } else { self.limits.callback },
            init,
            struck: false,
        });
        token
    }

    /// Stops watching. `None` for a token that is not running.
    pub fn leave(&mut self, token: u64) -> Option<WatchEnded> {
        let position = self.running.iter().position(|running| running.token == token)?;
        let running = self.running.swap_remove(position);
        Some(WatchEnded {
            ran: self.clock.now().saturating_sub(running.since),
            generation: running.generation,
            what: running.what,
            struck: running.struck,
        })
    }

    /// Counts a strike for every watched callback that has run past its limit and has not struck yet, and says what struck.
    pub fn sweep(&mut self) -> Vec<BudgetOverrun> {
        let now = self.clock.now();
        let mut struck = Vec::new();
        for running in &mut self.running {
            if !running.struck && now.saturating_sub(running.since) >= running.limit {
                running.struck = true;
                struck.push((
                    running.generation,
                    running.what.to_string(),
                    running.limit,
                    running.init,
                ));
            }
        }
        struck
            .into_iter()
            .map(|(generation, what, limit, init)| {
                self.overruns += 1;
                let degraded_before = matches!(self.supervisor.health(), Health::Degraded { .. });
                let action = self.supervisor.record_fault(GenerationFault::Budget);
                let degraded_now = matches!(self.supervisor.health(), Health::Degraded { .. });
                if !degraded_before {
                    *self.per_generation.entry(generation).or_default() += 1;
                }
                if init {
                    self.init_overran.insert(generation);
                }
                let overrun = BudgetOverrun {
                    generation,
                    what,
                    limit,
                    init,
                    strikes: self.supervisor.strikes(),
                    latched: degraded_now && !degraded_before && action == FaultAction::GiveUp,
                    uncounted: degraded_before,
                };
                self.last = Some(overrun.to_string());
                overrun
            })
            .collect()
    }

    /// How long until the next watched callback passes its limit, `None` when nothing is watched that has not struck.
    pub fn next_deadline(&self) -> Option<Duration> {
        let now = self.clock.now();
        self.running
            .iter()
            .filter(|running| !running.struck)
            .map(|running| (running.since + running.limit).saturating_sub(now))
            .min()
    }

    /// Whether the shell is degraded.
    pub fn degraded(&self) -> bool {
        matches!(self.supervisor.health(), Health::Degraded { .. })
    }

    /// A deliberate reload: the strikes and the degraded state are cleared.
    pub fn user_reload(&mut self) {
        self.supervisor.user_reload();
    }

    /// Why `generation` may not take over a reload, `None` when it may: its `init` ran past the init limit.
    pub fn vet(&self, generation: u64) -> Option<String> {
        self.init_overran.contains(&generation).then(|| {
            format!(
                "had an init that held the shell thread past the {:?} init budget, so it does not replace the generation that was serving",
                self.limits.init
            )
        })
    }

    /// Forgets everything: for a new shell instance.
    pub fn reset(&mut self) {
        self.running.clear();
        self.supervisor.user_reload();
        self.overruns = 0;
        self.per_generation.clear();
        self.init_overran.clear();
        self.last = None;
    }

    /// What the status shows.
    pub fn status(&self) -> BudgetStatus {
        let now = self.clock.now();
        let mut by_generation: Vec<(u64, u64)> = self.per_generation.iter().map(|(g, n)| (*g, *n)).collect();
        by_generation.sort_unstable();
        BudgetStatus {
            limits: self.limits,
            degraded: self.degraded(),
            strikes: self.supervisor.strikes(),
            overruns: self.overruns,
            by_generation,
            last: self.last.clone(),
            stuck: self
                .running
                .iter()
                .filter(|running| running.struck)
                .map(|running| (running.what.to_string(), now.saturating_sub(running.since)))
                .max_by_key(|(_, ran)| *ran),
        }
    }
}

// ---- the process's one tracker and its watchdog ----------------------------------------------------------------------

struct BudgetShared {
    tracker: Mutex<BudgetTracker<SystemClock>>,
    wake: Condvar,
}

static SHARED: LazyLock<BudgetShared> = LazyLock::new(|| BudgetShared {
    tracker: Mutex::new(BudgetTracker::new(SystemClock, BudgetLimits::default())),
    wake: Condvar::new(),
});

fn shared() -> &'static BudgetShared {
    &SHARED
}

fn tracker() -> MutexGuard<'static, BudgetTracker<SystemClock>> {
    shared().tracker.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Starts the watchdog on first use. It lives for the process: it sleeps until something is watched.
fn ensure_watchdog() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let spawned = std::thread::Builder::new()
            .name("gnoblin-budget".to_owned())
            .spawn(watchdog);
        if let Err(error) = spawned {
            log::error!("cannot start the callback budget watchdog: {error}; hung callbacks will not be counted");
        }
    });
}

fn watchdog() {
    let shared = shared();
    let mut tracker = shared.tracker.lock().unwrap_or_else(PoisonError::into_inner);
    loop {
        let overruns = tracker.sweep();
        if !overruns.is_empty() {
            drop(tracker);
            for overrun in &overruns {
                report(overrun);
            }
            tracker = shared.tracker.lock().unwrap_or_else(PoisonError::into_inner);
            continue;
        }
        tracker = match tracker.next_deadline() {
            // Round up so the sweep after the wait finds the callback past its limit, not a hair short of it.
            Some(wait) => {
                shared
                    .wake
                    .wait_timeout(tracker, wait + Duration::from_millis(1))
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
            }
            None => shared.wake.wait(tracker).unwrap_or_else(PoisonError::into_inner),
        };
    }
}

/// The log lines of one strike. Logged once per callback: the sweep marks it.
fn report(overrun: &BudgetOverrun) {
    let what = if overrun.init { "init" } else { "callback" };
    if overrun.uncounted {
        log::error!(
            generation = overrun.generation,
            "the {what} {overrun}: the shell is degraded already, so this is not counted again"
        );
    } else if overrun.latched {
        log::error!(
            generation = overrun.generation,
            "the {what} {overrun}: strike {} within 30 s, so the shell is DEGRADED: its own reload requests are refused and the \
             last good generation keeps running until `gnoblinctl shell reload` or the cool-down",
            overrun.strikes
        );
    } else {
        log::error!(
            generation = overrun.generation,
            "the {what} {overrun}: strike {} within 30 s (three latch the shell degraded); a native callback cannot be \
             interrupted, so the shell thread stays held until it returns",
            overrun.strikes
        );
    }
}

/// Watches one callback or `init` until it is dropped.
#[must_use = "the watch ends when the guard is dropped"]
pub(crate) struct BudgetWatch(Option<u64>);

impl Drop for BudgetWatch {
    fn drop(&mut self) {
        let Some(token) = self.0.take() else { return };
        let finished = tracker().leave(token);
        if let Some(finished) = finished.filter(|finished| finished.struck) {
            log::warn!(
                generation = finished.generation,
                "{} of generation {} returned after {:.1?}",
                finished.what,
                finished.generation,
                finished.ran
            );
        }
    }
}

fn enter(generation: u64, what: &Arc<str>, init: bool) -> BudgetWatch {
    ensure_watchdog();
    let (token, first) = {
        let mut tracker = tracker();
        let first = tracker.next_deadline().is_none();
        (tracker.enter(generation, Arc::clone(what), init), first)
    };
    if first {
        shared().wake.notify_one();
    }
    BudgetWatch(Some(token))
}

/// Watches a callback of `generation` called `what` (`timer_add`, `on_signal window.request`).
pub(crate) fn watch(generation: u64, what: &Arc<str>) -> BudgetWatch {
    enter(generation, what, false)
}

/// The `init`s running now, by generation: the loader tells the lifecycle observer when each begins and ends.
static INITS: LazyLock<Mutex<HashMap<u64, BudgetWatch>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// The `init` of `generation` begins: it is watched against the init limit until [`init_ended`].
pub(crate) fn init_begun(generation: u64) {
    let guard = enter(generation, &Arc::from("init"), true);
    INITS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(generation, guard);
}

/// The `init` of `generation` returned, failed or panicked.
pub(crate) fn init_ended(generation: u64) {
    // The guard is dropped outside the table's lock: it takes the tracker's.
    let guard = INITS.lock().unwrap_or_else(PoisonError::into_inner).remove(&generation);
    drop(guard);
}

/// The budget status of this process's shell.
pub fn status() -> BudgetStatus {
    tracker().status()
}

/// Whether the shell is degraded.
pub fn degraded() -> bool {
    tracker().degraded()
}

/// A deliberate reload is about to run: the strikes and the degraded state are cleared.
pub fn user_reload() {
    tracker().user_reload();
}

/// Why `generation` may not take over a reload, `None` when it may.
pub(crate) fn vet(generation: u64) -> Option<String> {
    tracker().vet(generation)
}

/// Replaces the limits. For the tests and the scenario driver: the design's limits are the defaults and the only ones a session has.
pub fn set_limits(limits: BudgetLimits) {
    tracker().set_limits(limits);
    // A watch already in a long wait must look again at the shorter limit.
    shared().wake.notify_one();
}

/// Forgets every strike and watch: a new shell instance starts clean.
pub fn reset() {
    tracker().reset();
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gnoblin_shell_host::supervisor::ManualClock;

    use super::{BudgetLimits, BudgetTracker};

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn tracker() -> (BudgetTracker<ManualClock>, ManualClock) {
        let clock = ManualClock::default();
        // The manual clock reads zero until it is moved; start it a minute in so no strike is "older than the window" by accident.
        clock.advance(secs(60));
        (BudgetTracker::new(clock.clone(), BudgetLimits::default()), clock)
    }

    #[test]
    fn a_callback_inside_its_budget_is_not_a_strike() {
        let (mut tracker, clock) = tracker();
        let token = tracker.enter(1, "timer_add", false);
        clock.advance(Duration::from_millis(1999));
        assert!(tracker.sweep().is_empty());
        let finished = tracker.leave(token).expect("it was running");
        assert!(!finished.struck);
        assert_eq!(finished.ran, Duration::from_millis(1999));
        assert_eq!(tracker.status().overruns, 0);
    }

    #[test]
    fn a_callback_past_the_budget_strikes_once_while_it_is_still_running() {
        let (mut tracker, clock) = tracker();
        let token = tracker.enter(7, "on_signal window.request", false);
        assert_eq!(tracker.next_deadline(), Some(secs(2)));
        clock.advance(secs(2));
        let overruns = tracker.sweep();
        assert_eq!(overruns.len(), 1, "{overruns:?}");
        assert_eq!(overruns[0].generation, 7);
        assert_eq!(overruns[0].what, "on_signal window.request");
        assert_eq!(overruns[0].strikes, 1);
        assert!(!overruns[0].latched);
        // Logged once: the sweep does not strike the same callback again, however long it runs.
        clock.advance(secs(20));
        assert!(tracker.sweep().is_empty());
        assert_eq!(
            tracker.next_deadline(),
            None,
            "a callback that struck is not waited for again"
        );
        let status = tracker.status();
        assert_eq!(status.strikes, 1);
        assert_eq!(status.stuck.as_ref().map(|stuck| stuck.1), Some(secs(22)));
        let finished = tracker.leave(token).expect("running");
        assert!(finished.struck);
        assert_eq!(finished.ran, secs(22));
        assert!(tracker.status().stuck.is_none());
    }

    #[test]
    fn three_strikes_in_thirty_seconds_latch_degraded_and_a_deliberate_reload_clears_it() {
        let (mut tracker, clock) = tracker();
        for strike in 1..=3 {
            let token = tracker.enter(4, "timer_add", false);
            clock.advance(secs(3));
            let overruns = tracker.sweep();
            assert_eq!(overruns.len(), 1);
            assert_eq!(overruns[0].strikes, strike);
            assert_eq!(overruns[0].latched, strike == 3, "strike {strike}");
            tracker.leave(token);
            clock.advance(secs(1));
        }
        assert!(tracker.degraded());
        assert_eq!(tracker.status().by_generation, [(4, 3)]);
        // A fourth overrun while degraded is seen but not counted.
        let token = tracker.enter(4, "timer_add", false);
        clock.advance(secs(3));
        let overruns = tracker.sweep();
        assert!(overruns[0].uncounted && !overruns[0].latched, "{overruns:?}");
        tracker.leave(token);
        assert_eq!(tracker.status().by_generation, [(4, 3)]);
        assert_eq!(tracker.status().overruns, 4);

        tracker.user_reload();
        assert!(!tracker.degraded());
        assert_eq!(tracker.status().strikes, 0);
    }

    #[test]
    fn strikes_older_than_the_window_do_not_latch() {
        let (mut tracker, clock) = tracker();
        for _ in 0..2 {
            let token = tracker.enter(1, "timer_add", false);
            clock.advance(secs(3));
            tracker.sweep();
            tracker.leave(token);
        }
        clock.advance(secs(30));
        let token = tracker.enter(1, "timer_add", false);
        clock.advance(secs(3));
        let overruns = tracker.sweep();
        tracker.leave(token);
        assert_eq!(overruns[0].strikes, 1, "the first two aged out");
        assert!(!tracker.degraded());
    }

    #[test]
    fn the_latch_lapses_after_the_cool_down() {
        let (mut tracker, clock) = tracker();
        for _ in 0..3 {
            let token = tracker.enter(1, "timer_add", false);
            clock.advance(secs(3));
            tracker.sweep();
            tracker.leave(token);
        }
        assert!(tracker.degraded());
        clock.advance(secs(301));
        assert!(!tracker.degraded());
    }

    #[test]
    fn an_init_has_its_own_limit_and_a_generation_whose_init_overran_may_not_take_over() {
        let (mut tracker, clock) = tracker();
        let token = tracker.enter(9, "init", true);
        clock.advance(secs(4));
        assert!(tracker.sweep().is_empty(), "an init may use five seconds");
        clock.advance(secs(1));
        let overruns = tracker.sweep();
        assert_eq!(overruns.len(), 1);
        assert!(overruns[0].init);
        tracker.leave(token);
        let why = tracker.vet(9).expect("it overran");
        assert!(why.contains("init budget"), "{why}");
        assert_eq!(tracker.vet(10), None);
    }

    #[test]
    fn a_reset_forgets_the_instance() {
        let (mut tracker, clock) = tracker();
        let token = tracker.enter(1, "init", true);
        clock.advance(secs(6));
        tracker.sweep();
        tracker.leave(token);
        tracker.reset();
        assert_eq!(tracker.vet(1), None);
        let status = tracker.status();
        assert_eq!((status.overruns, status.strikes, status.last), (0, 0, None));
    }

    #[test]
    fn the_status_says_what_a_person_needs() {
        let (mut tracker, clock) = tracker();
        for _ in 0..3 {
            let token = tracker.enter(2, "timer_add", false);
            clock.advance(secs(3));
            tracker.sweep();
            tracker.leave(token);
        }
        let text = tracker.status().to_string();
        assert!(text.contains("3 strike(s)"), "{text}");
        assert!(text.contains("DEGRADED"), "{text}");
        assert!(text.contains("timer_add of generation 2"), "{text}");
    }
}
