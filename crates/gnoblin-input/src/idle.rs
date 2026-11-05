//! Idle and activity tracking as a pure counter.
//!
//! The session's idle state is derived from input activity, and the derivation has to
//! be exact: a session that reads idle too early dims or locks while the user is typing,
//! and one that reads it too late never idles. So this is a counter and a last-activity
//! tick, with no timer of its own: the caller samples [`Activity::is_idle`] with the same
//! tick it feeds the rest of the input path.
//!
//! Ticks only move the idle clock forwards. A late or out-of-order tick never moves the
//! last activity backwards, because doing so would make the session look busier than it
//! was and delay idle forever.

use crate::tick::Tick;

/// Input activity since the session started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Activity {
    events: u64,
    last: Option<Tick>,
}

impl Activity {
    /// No activity yet.
    pub const fn new() -> Self {
        Self { events: 0, last: None }
    }

    /// Record one input event at `tick`.
    ///
    /// Returns whether this event advanced the idle clock. An event older than the last
    /// one recorded still counts as activity, but does not move the clock back, so the
    /// return value is `false` in that case.
    pub fn record(&mut self, tick: Tick) -> bool {
        self.events = self.events.saturating_add(1);
        match self.last {
            Some(last) if tick <= last => false,
            _ => {
                self.last = Some(tick);
                true
            }
        }
    }

    /// How many input events have been recorded.
    pub const fn events(&self) -> u64 {
        self.events
    }

    /// When the last activity happened, or `None` if there has been none.
    pub const fn last(&self) -> Option<Tick> {
        self.last
    }

    /// Ticks since the last activity.
    ///
    /// Before any activity this is the whole tick counter, because a session that has
    /// never been touched has been idle since it started.
    pub const fn idle_ticks(&self, now: Tick) -> u64 {
        match self.last {
            Some(last) => now.since(last),
            None => now.raw(),
        }
    }

    /// Whether the session has been idle for at least `timeout` ticks.
    ///
    /// A timeout of zero means idle immediately, which is what a caller uses to find out
    /// "is anything happening at all" without waiting.
    pub const fn is_idle(&self, now: Tick, timeout: u64) -> bool {
        self.idle_ticks(now) >= timeout
    }
}

#[cfg(test)]
mod tests {
    use super::Activity;
    use crate::tick::Tick;

    #[test]
    fn a_fresh_session_is_idle_from_the_start() {
        let activity = Activity::new();
        assert_eq!(activity.events(), 0);
        assert_eq!(activity.last(), None);
        assert_eq!(activity.idle_ticks(Tick::new(100)), 100);
        assert!(activity.is_idle(Tick::new(100), 50));
        assert!(activity.is_idle(Tick::ZERO, 0));
    }

    #[test]
    fn activity_restarts_the_idle_clock() {
        let mut activity = Activity::new();
        assert!(activity.record(Tick::new(100)));
        assert_eq!(activity.events(), 1);
        assert_eq!(activity.idle_ticks(Tick::new(150)), 50);
        assert!(!activity.is_idle(Tick::new(150), 51));
        assert!(activity.is_idle(Tick::new(150), 50));
    }

    #[test]
    fn an_out_of_order_tick_never_moves_the_clock_backwards() {
        let mut activity = Activity::new();
        assert!(activity.record(Tick::new(100)));
        assert!(!activity.record(Tick::new(90)));
        assert_eq!(activity.last(), Some(Tick::new(100)));
        assert_eq!(activity.idle_ticks(Tick::new(110)), 10);
        assert_eq!(activity.events(), 2, "the event still happened");
        assert!(!activity.record(Tick::new(100)), "the same tick is not progress");
    }
}
