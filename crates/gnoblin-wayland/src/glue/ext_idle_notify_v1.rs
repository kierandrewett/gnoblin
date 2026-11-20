//! `gnoblin-wayland-ext-idle-notify-v1`: the session clock and the last input live on the state.

use gnoblin_input::Activity;
use gnoblin_types::Tick;
use gnoblin_wayland_protocols::ext_idle_notify_v1::{ExtIdleNotifyV1Tables, Host, IdleNotifyExt as _};

use crate::State;

impl Host for State {
    fn idle_tables(&mut self) -> &mut ExtIdleNotifyV1Tables {
        &mut self.idle_tables
    }

    fn idle_clock(&self) -> Tick {
        self.clock
    }

    fn idle_activity(&self) -> Activity {
        self.activity
    }

    fn publish(&mut self) {
        State::publish(self);
    }
}

impl State {
    /// Advances the session clock, in milliseconds, and answers every notification the
    /// new time changes.
    pub(crate) fn advance_clock(&mut self, milliseconds: u64) {
        self.clock = self.clock.saturating_add(milliseconds);
        self.refresh_idle();
    }

    /// Records input activity, which is what ends an idle period.
    pub(crate) fn record_activity(&mut self) {
        self.activity.record(self.clock);
        self.refresh_idle();
    }
}
