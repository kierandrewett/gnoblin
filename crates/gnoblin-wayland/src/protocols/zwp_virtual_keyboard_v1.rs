//! XML protocol `virtual_keyboard_unstable_v1` (`<protocol name="virtual_keyboard_unstable_v1">`); interfaces served: `zwp_virtual_keyboard_manager_v1`.
//!
//! `zwp_virtual_keyboard_manager_v1` and `zwp_virtual_keyboard_v1` (XML
//! `virtual-keyboard-unstable-v1`).
//!
//! A virtual keyboard injects key events into whatever has focus. It is privileged and
//! filtered by [`AccessPolicy`](crate::access::AccessPolicy) like the input method.

use std::sync::Arc;

use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::virtual_keyboard::{VirtualKeyboardBackend, VirtualKeyboardHandler, VirtualKeyboardManagerState};

use crate::State;
use crate::access::{Access, AccessPolicy, Global};

use crate::log;

/// Creates the `zwp_virtual_keyboard_manager_v1` global.
pub(crate) fn virtual_keyboard_global(display: &DisplayHandle, access: &Arc<Access>) -> VirtualKeyboardManagerState {
    let access = Arc::clone(access);
    VirtualKeyboardManagerState::new::<State, _>(display, move |client| {
        AccessPolicy::allows(&*access, &client.id(), Global::VirtualKeyboard)
    })
}

impl VirtualKeyboardHandler for State {
    fn process_virtual_keyboard_event(&mut self, event: smithay::backend::input::InputEvent<VirtualKeyboardBackend>) {
        use smithay::backend::input::{Event, InputEvent, KeyboardKeyEvent};
        match event {
            InputEvent::Keyboard { event, .. } => {
                let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                let state = match event.state() {
                    smithay::backend::input::KeyState::Pressed => gnoblin_types::KeyState::Pressed,
                    smithay::backend::input::KeyState::Released => gnoblin_types::KeyState::Released,
                };
                self.dispatch_key(u32::from(event.key_code()), state, serial, event.time());
            }
            InputEvent::DeviceAdded { .. } | InputEvent::DeviceRemoved { .. } => {
                log::debug!("[virtual-keyboard] the device set changed");
            }
            _ => log::warn!("[virtual-keyboard] a virtual keyboard sent an event it cannot send"),
        }
    }
}
