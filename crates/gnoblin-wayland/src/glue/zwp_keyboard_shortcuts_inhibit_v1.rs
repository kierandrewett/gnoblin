//! `gnoblin-wayland-zwp-keyboard-shortcuts-inhibit-v1`: a covered session never inhibits.

use gnoblin_wayland_protocols::zwp_keyboard_shortcuts_inhibit_v1::{self as inhibit, Host};
use smithay::wayland::keyboard_shortcuts_inhibit::{
    KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState, KeyboardShortcutsInhibitor,
};

use crate::State;

impl Host for State {
    fn session_unlocked(&self) -> bool {
        self.lock.state() == gnoblin_lock::State::Unlocked
    }

    fn shortcuts_held(&mut self) -> &mut Vec<KeyboardShortcutsInhibitor> {
        &mut self.shortcuts_held
    }
}

impl KeyboardShortcutsInhibitHandler for State {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.shortcuts_inhibit_state
    }

    fn new_inhibitor(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        inhibit::new_inhibitor(self, inhibitor);
    }

    fn inhibitor_destroyed(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        inhibit::inhibitor_destroyed(self, &inhibitor);
    }
}
