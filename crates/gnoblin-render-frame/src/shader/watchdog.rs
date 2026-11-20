//! The frame-cost watchdog of the user shader pass (`design/modular-framework.md` 37.2): the policy that reads what the renderers
//! recorded in [`ShaderTimings`](super::ShaderTimings).
//!
//! The state is per source, keyed by [`ShaderKey`] (the hash of the text), because the cost is the program's and two nodes with
//! the same source run the same program:
//!
//! - a frame in which one source took more than [`SHADER_FRAME_BUDGET`] is *over*; [`FRAMES_OVER_BUDGET`] over frames in a row are
//!   a **strike**, and a frame within budget ends the run. A strike switches the source's passes off for [`DISABLE_FOR`]: the
//!   frame builder leaves them out (`OutputFrame::drop_shader_passes`) and the node's `skipped` names them. They come back on their
//!   own when the time has passed, and the next strike is the next of three.
//! - [`STRIKES_TO_REFUSE`] strikes **refuse** the source for the session: its passes stay off, and a write of the same source is
//!   answered `bad-value`. The refusal list ([`ShaderRefusals`]) is shared with the tree's write path, which is on another thread.
//! - a device loss in a draw whose frame held shader passes strikes every source in that frame at once (the renderer cannot say which
//!   one hung the device), before the recovery that follows a loss rebuilds the renderer.
//!
//! Nothing here reads a clock or draws: the caller passes the time (a monotonic [`Duration`] from any origin: the composer's own clock,
//! which the session advances), and a test passes costs it made up.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use gnoblin_render::ShaderKey;

use super::{FRAMES_OVER_BUDGET, SHADER_FRAME_BUDGET, STRIKES_TO_REFUSE, ShaderReport};

/// How long a strike switches a source off for before its passes are drawn again.
pub const DISABLE_FOR: Duration = Duration::from_secs(10);

/// The sources refused for the rest of the session, as a handle every holder shares.
///
/// The watchdog adds to it; the tree's write path asks it before it validates a shader, so a refused source is answered `bad-value`
/// without being compiled.
#[derive(Clone, Debug, Default)]
pub struct ShaderRefusals(Arc<Mutex<BTreeSet<u64>>>);

impl ShaderRefusals {
    /// Whether `key` is refused for the session.
    #[must_use]
    pub fn refuses(&self, key: ShaderKey) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&key.get())
    }

    fn refuse(&self, key: ShaderKey) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).insert(key.get());
    }
}

/// Why a source's passes are not drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switched {
    /// A strike switched it off for [`DISABLE_FOR`].
    OverBudget,
    /// Three strikes refused it for the session.
    Refused,
}

impl Switched {
    /// The reason as the node's `skipped` list words it.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::OverBudget => "over-budget",
            Self::Refused => "refused-for-session",
        }
    }
}

/// What caused a strike.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// [`FRAMES_OVER_BUDGET`] frames in a row over [`SHADER_FRAME_BUDGET`]; the cost of the last of them.
    OverBudget(Duration),
    /// The device was lost in a draw whose frame held the source.
    DeviceLost,
}

/// Something the caller logs: a strike or a refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ruling {
    /// The source took a strike and is switched off for [`DISABLE_FOR`].
    Struck {
        /// The source.
        key: ShaderKey,
        /// Its strikes so far, this one included.
        strikes: u32,
        /// What caused it.
        cause: Cause,
    },
    /// The source reached [`STRIKES_TO_REFUSE`] and is refused for the session.
    Refused {
        /// The source.
        key: ShaderKey,
    },
}

/// What the watchdog holds for one source.
#[derive(Clone, Copy, Debug, Default)]
struct Tally {
    /// Frames in a row over budget.
    over: u32,
    /// Strikes taken.
    strikes: u32,
    /// The passes stay off until this time.
    off_until: Option<Duration>,
}

/// The watchdog's state. One per compositor.
#[derive(Debug, Default)]
pub struct Watchdog {
    standing: BTreeMap<ShaderKey, Tally>,
    refusals: ShaderRefusals,
}

impl Watchdog {
    /// A watchdog that adds the sources it refuses to `refusals`.
    #[must_use]
    pub fn new(refusals: ShaderRefusals) -> Self {
        Self {
            standing: BTreeMap::new(),
            refusals,
        }
    }

    /// The shared refusal list.
    #[must_use]
    pub fn refusals(&self) -> &ShaderRefusals {
        &self.refusals
    }

    /// Replaces the shared refusal list. The session's host does it once at start, before any frame, so the tree's write path and the
    /// watchdog hold the same list.
    pub fn set_refusals(&mut self, refusals: ShaderRefusals) {
        self.refusals = refusals;
    }

    /// Why the passes of `key` are not drawn at `now`, or [`None`] when they are.
    #[must_use]
    pub fn switched(&self, key: ShaderKey, now: Duration) -> Option<Switched> {
        if self.refusals.refuses(key) {
            return Some(Switched::Refused);
        }
        self.standing
            .get(&key)
            .and_then(|standing| standing.off_until)
            .filter(|until| now < *until)
            .map(|_| Switched::OverBudget)
    }

    /// Folds one draw into the state: `report` is what [`ShaderTimings::take`](super::ShaderTimings::take) returned for the frame, and
    /// `drawn` is every source the frame held (`OutputFrame::shader_keys`). Returns what to log.
    pub fn observe(&mut self, now: Duration, drawn: &[ShaderKey], report: &ShaderReport) -> Vec<Ruling> {
        let mut events = Vec::new();
        if report.lost {
            for key in drawn {
                self.strike(*key, now, Cause::DeviceLost, &mut events);
            }
            return events;
        }
        for cost in &report.costs {
            if self.refusals.refuses(cost.key) {
                continue;
            }
            if cost.elapsed > SHADER_FRAME_BUDGET {
                let standing = self.standing.entry(cost.key).or_default();
                standing.over += 1;
                if standing.over >= FRAMES_OVER_BUDGET {
                    self.strike(cost.key, now, Cause::OverBudget(cost.elapsed), &mut events);
                }
            } else if let Some(standing) = self.standing.get_mut(&cost.key) {
                standing.over = 0;
                // A source that took no strike leaves nothing behind.
                if standing.strikes == 0 {
                    self.standing.remove(&cost.key);
                }
            }
        }
        events
    }

    /// Records a strike on `key`: switches it off for [`DISABLE_FOR`], and refuses it for the session at [`STRIKES_TO_REFUSE`].
    fn strike(&mut self, key: ShaderKey, now: Duration, cause: Cause, events: &mut Vec<Ruling>) {
        if self.refusals.refuses(key) {
            return;
        }
        let standing = self.standing.entry(key).or_default();
        standing.over = 0;
        standing.strikes += 1;
        standing.off_until = Some(now + DISABLE_FOR);
        events.push(Ruling::Struck {
            key,
            strikes: standing.strikes,
            cause,
        });
        if standing.strikes >= STRIKES_TO_REFUSE {
            self.refusals.refuse(key);
            events.push(Ruling::Refused { key });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ShaderCost;
    use super::*;

    fn key(n: u32) -> ShaderKey {
        ShaderKey::of(&format!("fn effect{n}() {{}}"))
    }

    fn report(costs: &[(ShaderKey, u64)]) -> ShaderReport {
        ShaderReport {
            costs: costs
                .iter()
                .map(|(key, millis)| ShaderCost {
                    key: *key,
                    elapsed: Duration::from_millis(*millis),
                })
                .collect(),
            ..ShaderReport::default()
        }
    }

    const SLOW: u64 = 51;
    const FAST: u64 = 50;

    /// Feeds `frames` draws that each cost `key` `millis`, a frame apart, and returns every event.
    fn run(dog: &mut Watchdog, now: &mut Duration, key: ShaderKey, millis: u64, frames: u32) -> Vec<Ruling> {
        let mut events = Vec::new();
        for _ in 0..frames {
            events.extend(dog.observe(*now, &[key], &report(&[(key, millis)])));
            *now += Duration::from_millis(16);
        }
        events
    }

    #[test]
    fn three_frames_in_a_row_over_the_budget_switch_the_pass_off_with_a_strike() {
        let mut dog = Watchdog::default();
        let mut now = Duration::ZERO;
        let k = key(1);
        assert!(
            run(&mut dog, &mut now, k, SLOW, 2).is_empty(),
            "two slow frames are not a strike"
        );
        assert_eq!(dog.switched(k, now), None);
        let events = run(&mut dog, &mut now, k, SLOW, 1);
        assert_eq!(
            events,
            [Ruling::Struck {
                key: k,
                strikes: 1,
                cause: Cause::OverBudget(Duration::from_millis(SLOW))
            }]
        );
        assert_eq!(dog.switched(k, now), Some(Switched::OverBudget));
        assert!(!dog.refusals().refuses(k), "one strike does not refuse");
    }

    #[test]
    fn the_budget_is_a_limit_and_a_frame_within_it_ends_the_run() {
        let mut dog = Watchdog::default();
        let mut now = Duration::ZERO;
        let k = key(1);
        // Exactly the budget is within it.
        assert!(run(&mut dog, &mut now, k, FAST, 10).is_empty());
        // Two slow, one fast, two slow: the fast frame resets the count, so no run of three.
        run(&mut dog, &mut now, k, SLOW, 2);
        run(&mut dog, &mut now, k, FAST, 1);
        assert!(run(&mut dog, &mut now, k, SLOW, 2).is_empty());
        assert_eq!(dog.switched(k, now), None);
    }

    #[test]
    fn a_source_is_switched_back_on_when_its_time_is_up_and_the_third_strike_refuses_it_for_the_session() {
        let mut dog = Watchdog::default();
        let mut now = Duration::ZERO;
        let k = key(1);
        let other = key(2);
        for strike in 1..=2 {
            let events = run(&mut dog, &mut now, k, SLOW, 3);
            assert!(
                matches!(events[..], [Ruling::Struck { strikes, .. }] if strikes == strike),
                "{events:?}"
            );
            assert_eq!(dog.switched(k, now), Some(Switched::OverBudget));
            now += DISABLE_FOR;
            assert_eq!(
                dog.switched(k, now),
                None,
                "the pass is drawn again after {DISABLE_FOR:?}"
            );
        }
        let events = run(&mut dog, &mut now, k, SLOW, 3);
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(matches!(events[0], Ruling::Struck { strikes: 3, .. }));
        assert_eq!(events[1], Ruling::Refused { key: k });
        assert!(dog.refusals().refuses(k));
        now += DISABLE_FOR * 100;
        assert_eq!(
            dog.switched(k, now),
            Some(Switched::Refused),
            "a refusal does not expire"
        );
        assert!(!dog.refusals().refuses(other), "another source is untouched");
        assert_eq!(dog.switched(other, now), None);
        // A refused source reports nothing more.
        assert!(run(&mut dog, &mut now, k, SLOW, 5).is_empty());
    }

    #[test]
    fn a_source_is_judged_alone() {
        let mut dog = Watchdog::default();
        let now = Duration::ZERO;
        let (slow, fine) = (key(1), key(2));
        let mut events = Vec::new();
        for _ in 0..3 {
            events.extend(dog.observe(now, &[slow, fine], &report(&[(slow, 200), (fine, 3)])));
        }
        assert!(
            matches!(events[..], [Ruling::Struck { key, .. }] if key == slow),
            "{events:?}"
        );
        assert_eq!(dog.switched(fine, now), None);
    }

    #[test]
    fn a_device_loss_strikes_every_source_of_the_frame_first() {
        let mut dog = Watchdog::default();
        let now = Duration::ZERO;
        let (a, b, absent) = (key(1), key(2), key(3));
        let lost = ShaderReport {
            lost: true,
            ..ShaderReport::default()
        };
        let events = dog.observe(now, &[a, b], &lost);
        assert_eq!(
            events,
            [
                Ruling::Struck {
                    key: a,
                    strikes: 1,
                    cause: Cause::DeviceLost
                },
                Ruling::Struck {
                    key: b,
                    strikes: 1,
                    cause: Cause::DeviceLost
                }
            ]
        );
        assert_eq!(dog.switched(a, now), Some(Switched::OverBudget));
        assert_eq!(dog.switched(b, now), Some(Switched::OverBudget));
        assert_eq!(
            dog.switched(absent, now),
            None,
            "a source not in the frame is not struck"
        );
        // A loss with no shader in the frame strikes nothing.
        assert!(dog.observe(now, &[], &lost).is_empty());
        // Every loss is a strike, so the third on one source refuses it.
        dog.observe(now, &[a], &lost);
        let events = dog.observe(now, &[a], &lost);
        assert_eq!(events.last(), Some(&Ruling::Refused { key: a }), "{events:?}");
        assert!(dog.refusals().refuses(a));
        assert!(!dog.refusals().refuses(b));
    }

    #[test]
    fn a_source_that_took_no_strike_leaves_no_state_behind() {
        let mut dog = Watchdog::default();
        let mut now = Duration::ZERO;
        let k = key(1);
        run(&mut dog, &mut now, k, SLOW, 2);
        run(&mut dog, &mut now, k, 1, 1);
        assert!(dog.standing.is_empty(), "{:?}", dog.standing);
        for n in 0..1000 {
            run(&mut dog, &mut now, key(1000 + n), 1, 1);
        }
        assert!(
            dog.standing.is_empty(),
            "sources that were never slow are not remembered"
        );
    }

    #[test]
    fn the_refusal_list_is_shared_with_every_clone() {
        let refusals = ShaderRefusals::default();
        let mut dog = Watchdog::new(refusals.clone());
        let mut now = Duration::ZERO;
        let k = key(1);
        for _ in 0..3 {
            run(&mut dog, &mut now, k, SLOW, 3);
            now += DISABLE_FOR;
        }
        assert!(
            refusals.refuses(k),
            "the tree's write path sees what the watchdog refused"
        );
        let mut replaced = Watchdog::default();
        replaced.set_refusals(refusals);
        assert_eq!(replaced.switched(k, now), Some(Switched::Refused));
    }
}
