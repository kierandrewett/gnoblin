//! Per-window frame callback pacing: the shell's `frame_policy` of a window.
//!
//! A client draws its next frame when the compositor answers the `wl_surface.frame` callback it asked for, so the
//! callback is the client's pace. While a shell holds the tree it may write a window node's `frame_policy`:
//!
//! - `auto` answers after every frame that shows the window, as a session with no shell does.
//! - `hold` never answers. The client waits, which is how a shell suspends a window it does not show.
//! - `throttle` answers at most `frame_rate` times a second.
//!
//! The pacer decides each window's answer when [`State::send_frame_callbacks`](crate::State) runs, and keeps what it
//! needs between frames: when a throttled window was last answered, and whether one is still waiting for its time.
//!
//! # Failure modes
//!
//! 1. **A held window is never answered again.** The policy is read from the node on every pass, so a write of
//!    `auto` is answered at the next frame, and [`CallbackPacer::changed`] asks for that frame even when nothing was
//!    drawn: `a_held_window_is_answered_once_the_shell_sets_it_back_to_auto`.
//! 2. **A throttled window stalls.** Frames are drawn only when something changed, so a window waiting out its
//!    interval would never be answered on a still screen. The pacer reports it ([`CallbackPacer::waiting`]) and the
//!    pass arms a one-shot timer for the remaining time, which wakes the loop:
//!    `a_throttled_window_is_answered_no_faster_than_its_rate`.
//! 3. **A window stays held with no shell.** The compositor reads the policy only while a shell holds the tree and
//!    sets it back to `auto` when the shell lets go, so a crashed shell cannot leave a window frozen:
//!    `a_held_window_is_answered_again_when_the_shell_lets_go_of_the_tree`.
//! 4. **A closed window's record lingers.** Each pass drops the records of windows the session no longer has.

use std::collections::BTreeMap;

use gnoblin_backend::render::compose::FramePolicy;
use gnoblin_types::WindowId;

/// What the pacing of one window asks for at one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pacing {
    /// Answer the window's callbacks now.
    Now,
    /// Leave them waiting; the window is held.
    Held,
    /// Leave them waiting for `ms` more milliseconds.
    After { ms: u32 },
}

/// The pacing state the session carries between frames.
#[derive(Debug, Default)]
pub(crate) struct CallbackPacer {
    /// The frame time each throttled window was last answered at.
    last: BTreeMap<WindowId, u32>,
    /// The policy each window had when [`CallbackPacer::changed`] last looked.
    seen: BTreeMap<WindowId, FramePolicy>,
    /// Whether the last pass left a throttled window with callbacks waiting for its time.
    waiting: bool,
    /// Whether a timer is armed to wake the loop for that.
    timer_armed: bool,
}

impl CallbackPacer {
    /// What `policy` asks for at frame time `now` for a window last answered at `last`. Frame times are milliseconds
    /// that wrap, so the difference is taken with wrapping arithmetic.
    pub(crate) fn decide(policy: FramePolicy, last: Option<u32>, now: u32) -> Pacing {
        match policy {
            FramePolicy::Auto => Pacing::Now,
            FramePolicy::Hold => Pacing::Held,
            FramePolicy::Throttle { interval_ms } => match last {
                Some(last) => {
                    let elapsed = now.wrapping_sub(last);
                    if elapsed >= interval_ms {
                        Pacing::Now
                    } else {
                        Pacing::After {
                            ms: interval_ms - elapsed,
                        }
                    }
                }
                None => Pacing::Now,
            },
        }
    }

    /// The pacing of `window` now, which also records the time a throttled window is answered at.
    pub(crate) fn pace(&mut self, window: WindowId, policy: FramePolicy, now: u32) -> Pacing {
        let pacing = Self::decide(policy, self.last.get(&window).copied(), now);
        if pacing == Pacing::Now && matches!(policy, FramePolicy::Throttle { .. }) {
            self.last.insert(window, now);
        }
        pacing
    }

    /// Starts a pass: nothing is waiting until the pass finds a window that is.
    pub(crate) fn begin_pass(&mut self, live: impl Fn(WindowId) -> bool) {
        self.waiting = false;
        self.last.retain(|window, _| live(*window));
    }

    /// Notes that a throttled window has callbacks waiting for its time, and returns whether a timer must be armed
    /// for it: `true` once, until the timer fires.
    pub(crate) fn wait_for(&mut self) -> bool {
        self.waiting = true;
        !std::mem::replace(&mut self.timer_armed, true)
    }

    /// The armed timer fired.
    pub(crate) fn timer_fired(&mut self) {
        self.timer_armed = false;
    }

    /// Whether the last pass left a window waiting for its time, so the next iteration must run the pass again.
    pub(crate) fn waiting(&self) -> bool {
        self.waiting
    }

    /// Whether any window's policy differs from the one seen at the last look. The policies of `windows` are noted.
    /// A change asks for a pass at the next frame: a window that was held, or throttled, may be owed an answer now.
    pub(crate) fn changed(&mut self, windows: impl Iterator<Item = (WindowId, FramePolicy)>) -> bool {
        let mut changed = false;
        let mut now = BTreeMap::new();
        for (window, policy) in windows {
            let before = self.seen.get(&window).copied().unwrap_or_default();
            changed |= before != policy;
            now.insert(window, policy);
        }
        self.seen = now;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THROTTLE_50MS: FramePolicy = FramePolicy::Throttle { interval_ms: 50 };

    #[test]
    fn auto_answers_every_frame_and_hold_never_does() {
        assert_eq!(CallbackPacer::decide(FramePolicy::Auto, Some(5), 5), Pacing::Now);
        assert_eq!(CallbackPacer::decide(FramePolicy::Hold, None, 1_000_000), Pacing::Held);
    }

    #[test]
    fn a_throttled_window_is_answered_once_its_interval_passed() {
        assert_eq!(
            CallbackPacer::decide(THROTTLE_50MS, None, 0),
            Pacing::Now,
            "the first frame"
        );
        assert_eq!(
            CallbackPacer::decide(THROTTLE_50MS, Some(100), 120),
            Pacing::After { ms: 30 }
        );
        assert_eq!(CallbackPacer::decide(THROTTLE_50MS, Some(100), 150), Pacing::Now);
        assert_eq!(CallbackPacer::decide(THROTTLE_50MS, Some(100), 400), Pacing::Now);
    }

    #[test]
    fn the_frame_clock_wrapping_does_not_stall_a_throttled_window() {
        assert_eq!(
            CallbackPacer::decide(THROTTLE_50MS, Some(u32::MAX - 10), 5),
            Pacing::After { ms: 34 },
            "16 ms passed across the wrap"
        );
        assert_eq!(
            CallbackPacer::decide(THROTTLE_50MS, Some(u32::MAX - 10), 45),
            Pacing::Now
        );
    }

    #[test]
    fn pacing_a_throttled_window_records_the_time_and_an_auto_window_records_nothing() {
        let mut pacer = CallbackPacer::default();
        let throttled = WindowId::new(1);
        let auto = WindowId::new(2);
        assert_eq!(pacer.pace(throttled, THROTTLE_50MS, 10), Pacing::Now);
        assert_eq!(pacer.pace(throttled, THROTTLE_50MS, 30), Pacing::After { ms: 30 });
        assert_eq!(pacer.pace(auto, FramePolicy::Auto, 30), Pacing::Now);
        assert!(!pacer.last.contains_key(&auto));
        assert_eq!(pacer.pace(throttled, THROTTLE_50MS, 60), Pacing::Now);
    }

    #[test]
    fn a_pass_forgets_windows_that_are_gone_and_clears_the_wait() {
        let mut pacer = CallbackPacer::default();
        pacer.pace(WindowId::new(1), THROTTLE_50MS, 10);
        pacer.pace(WindowId::new(2), THROTTLE_50MS, 10);
        assert!(pacer.wait_for(), "the first wait arms the timer");
        assert!(pacer.waiting());
        pacer.begin_pass(|window| window == WindowId::new(2));
        assert!(!pacer.waiting(), "a pass starts with nothing waiting");
        assert!(!pacer.last.contains_key(&WindowId::new(1)));
        assert!(pacer.last.contains_key(&WindowId::new(2)));
    }

    #[test]
    fn one_timer_is_armed_until_it_fires() {
        let mut pacer = CallbackPacer::default();
        assert!(pacer.wait_for());
        assert!(!pacer.wait_for(), "armed already");
        pacer.timer_fired();
        assert!(pacer.wait_for(), "armed again after it fired");
    }

    #[test]
    fn a_change_of_policy_is_noticed_once_and_auto_is_the_default() {
        let mut pacer = CallbackPacer::default();
        let window = WindowId::new(1);
        assert!(
            !pacer.changed([(window, FramePolicy::Auto)].into_iter()),
            "auto is the default"
        );
        assert!(pacer.changed([(window, FramePolicy::Hold)].into_iter()));
        assert!(
            !pacer.changed([(window, FramePolicy::Hold)].into_iter()),
            "no change the second time"
        );
        assert!(
            pacer.changed([(window, FramePolicy::Auto)].into_iter()),
            "held, then released"
        );
        assert!(pacer.changed([(window, THROTTLE_50MS)].into_iter()));
        assert!(
            !pacer.changed(std::iter::empty()),
            "a window that is gone is not a change; its record is dropped"
        );
        assert!(!pacer.changed([(window, FramePolicy::Auto)].into_iter()));
    }
}
