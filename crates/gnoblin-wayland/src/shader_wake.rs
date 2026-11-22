//! The wake for the return of a switched-off shader pass.
//!
//! The frame-cost watchdog switches a struck pass off for ten seconds and the composer switches it back on when its clock reaches
//! that time, which damages the nodes that carry it. On an output that is otherwise idle nothing would turn the loop to that clock,
//! so the loop sets one one-shot timer for the time ([`State::arm_shader_wake`]) and none while nothing is struck. The timer does
//! no work of its own: the turn it wakes advances the clock, the composer re-arms the pass and damages its nodes, and the frame
//! that follows draws it.

use std::time::Duration;

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::log;
use crate::state::State;

/// The timer the loop holds for the earliest switched-off pass: at most one at a time.
#[derive(Debug, Default)]
pub(crate) struct ShaderWake {
    /// Whether a timer is armed and has not fired.
    armed: bool,
    /// How many timers have fired. A test reads it; nothing else does.
    fired: u32,
}

impl ShaderWake {
    /// How many timers have fired.
    #[cfg(test)]
    pub(crate) fn fired(&self) -> u32 {
        self.fired
    }

    /// Whether a timer is armed and has not fired.
    #[cfg(test)]
    pub(crate) fn armed(&self) -> bool {
        self.armed
    }
}

impl State {
    /// Arms a one-shot timer to wake the loop when the earliest switched-off shader pass comes back, unless one is armed already or
    /// nothing is switched off. The wait is the composer's own time to that moment, so a session whose clock a test moves by hand arms
    /// for what is left on that clock. A session driven by hand has no loop to wake: the caller's next pass advances the clock.
    pub(crate) fn arm_shader_wake(&mut self) {
        if self.shader_wake.armed {
            return;
        }
        let Some(at) = self.composer.shader_rearm_at() else {
            return;
        };
        let Some(handle) = self.loop_handle.clone() else {
            return;
        };
        let wait = at
            .saturating_sub(self.composer.shader_clock())
            .max(Duration::from_millis(1));
        self.shader_wake.armed = true;
        let inserted = handle.insert_source(Timer::from_duration(wait), |_, (), state: &mut State| {
            state.shader_wake.armed = false;
            state.shader_wake.fired += 1;
            TimeoutAction::Drop
        });
        if let Err(error) = inserted {
            log::warn!(
                area = "frame",
                ?error,
                "the timer for a shader pass that comes back could not be armed"
            );
            self.shader_wake.armed = false;
        }
    }
}
