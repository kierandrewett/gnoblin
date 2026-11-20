//! `gnoblin-wayland-zwp-virtual-keyboard-v1`: the seat's keymap table and key delivery are the state's.

use gnoblin_types::KeyState;
use gnoblin_wayland_protocols::zwp_virtual_keyboard_v1::Host;
use smithay::backend::input::InputTime;
use smithay::utils::Serial;

use crate::State;

impl Host for State {
    fn new_virtual_keyboard(&mut self) -> u64 {
        State::new_virtual_keyboard(self)
    }

    fn set_virtual_keymap(&mut self, id: u64, text: String) {
        State::set_virtual_keymap(self, id, text);
    }

    fn release_virtual_keymap(&mut self, id: u64) {
        State::release_virtual_keymap(self, id);
    }

    fn use_virtual_keymap(&mut self, id: u64) -> bool {
        State::use_virtual_keymap(self, id)
    }

    fn set_modifier_state(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
        State::set_modifier_state(self, depressed, latched, locked, group);
    }

    fn deliver_virtual_key(&mut self, keycode: u32, state: KeyState, serial: Serial, time: InputTime) {
        State::deliver_virtual_key(self, keycode, state, serial, time);
    }
}
