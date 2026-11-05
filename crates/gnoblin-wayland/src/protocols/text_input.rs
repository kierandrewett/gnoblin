//! Text input: `zwp_text_input_manager_v3`, `zwp_input_method_manager_v2` and
//! `zwp_virtual_keyboard_v1`.
//!
//! These three protocols are one conversation. A client that wants to type in a language
//! an input method serves creates a `zwp_text_input_v3` and enables it while its surface
//! has focus; the input method (IBus, fcitx, an on-screen keyboard) holds a
//! `zwp_input_method_v2` on the same seat and answers with `commit_string`,
//! `preedit_string` and `delete_surrounding_text`; and an on-screen keyboard that types
//! into whatever has focus holds a `zwp_virtual_keyboard_v1`, which is nothing but a
//! source of key events.
//!
//! Smithay carries the conversation: `TextInputHandle` holds the focused text input and the
//! active one, and `InputMethodHandle` forwards an input method's commits to it. What this
//! module owns is the three things a compositor must decide:
//!
//! - **Focus.** `enter` and `leave` are sent when the keyboard focus moves, because a text
//!   input that is not told it has focus never asks to be enabled.
//! - **Who may be an input method, and who may type.** Both are privileged: an input method
//!   sees what the user types into every window, and a virtual keyboard can type into every
//!   window. [`AccessPolicy`](crate::access::AccessPolicy) answers for the connection, so a
//!   sandboxed client sees neither global and cannot become either.
//! - **The key events an on-screen keyboard injects.** They take the same path as a physical
//!   key, minus the loop: the virtual keyboard is the way an input method forwards keys it
//!   did not consume, so its events bypass the grab that would send them back to the input
//!   method.
//!
//! # Failure modes
//!
//! 1. **A text input is never told it has focus.** Focus moves set the handle and send
//!    `enter`, so a client's `enable` is accepted rather than discarded:
//!    `text_input_enter_and_commit_round_trip` in `tests/protocols.rs`.
//! 2. **A committed string never reaches the client.** An input method's `commit_string`
//!    goes to the active text input of the focused surface, and only there: the same test
//!    proves the round trip end to end.
//! 3. **A sandboxed client becomes an input method.** Both managers filter by
//!    [`AccessPolicy`](crate::access::AccessPolicy), so the global is not advertised to a
//!    sandboxed client at all:
//!    `a_sandboxed_client_does_not_see_the_privileged_globals`.
//! 4. **An injected key bypasses the compositor's own bindings.** A virtual keyboard's keys
//!    are routed through the same [`crate::State::dispatch_key`] as a physical one, so a
//!    binding still wins over an on-screen keyboard.

use std::sync::Arc;

use smithay::reexports::wayland_protocols::wp::text_input::zv3::server::zwp_text_input_v3::{
    ContentHint, ContentPurpose,
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{DisplayHandle, Resource};
use smithay::utils::{Logical, Rectangle};
use smithay::wayland::input_method::{InputMethodHandler, InputMethodManagerState, PopupSurface};
use smithay::wayland::text_input::{TextInputActivation, TextInputManagerState};
use smithay::wayland::virtual_keyboard::{VirtualKeyboardBackend, VirtualKeyboardHandler, VirtualKeyboardManagerState};
use tracing::{debug, warn};

use crate::State;
use crate::access::{Access, AccessPolicy, Global};

/// Creates the `zwp_text_input_manager_v3` global.
pub(crate) fn text_input_global(display: &DisplayHandle) -> TextInputManagerState {
    TextInputManagerState::new::<State>(display)
}

/// Creates the `zwp_input_method_manager_v2` global.
///
/// The filter is the access policy: an input method sees every keystroke in the session, so
/// only a client the policy calls privileged may be one.
pub(crate) fn input_method_global(display: &DisplayHandle, access: &Arc<Access>) -> InputMethodManagerState {
    let access = Arc::clone(access);
    InputMethodManagerState::new::<State, _>(display, move |client| {
        AccessPolicy::allows(&*access, &client.id(), Global::InputMethod)
    })
}

/// Creates the `zwp_virtual_keyboard_manager_v1` global.
///
/// A virtual keyboard types into whatever has focus, so it is as privileged as an input
/// method and filtered the same way.
pub(crate) fn virtual_keyboard_global(display: &DisplayHandle, access: &Arc<Access>) -> VirtualKeyboardManagerState {
    let access = Arc::clone(access);
    VirtualKeyboardManagerState::new::<State, _>(display, move |client| {
        AccessPolicy::allows(&*access, &client.id(), Global::VirtualKeyboard)
    })
}

impl TextInputActivation for State {
    fn activated(&mut self, content_type: Option<(ContentHint, ContentPurpose)>) {
        debug!(
            hint = ?content_type.map(|(hint, _)| hint),
            "[text-input] a client enabled its text input"
        );
    }

    fn deactivated(&mut self) {
        debug!("[text-input] the active text input was disabled");
    }
}

impl InputMethodHandler for State {
    fn new_popup(&mut self, surface: PopupSurface) {
        debug!(popup = ?surface.wl_surface().id(), "[input-method] the input method created a popup");
        self.input_method_popups.push(surface);
    }

    fn dismiss_popup(&mut self, surface: PopupSurface) {
        self.input_method_popups
            .retain(|held| held.wl_surface() != surface.wl_surface());
    }

    fn popup_repositioned(&mut self, surface: PopupSurface) {
        debug!(
            popup = ?surface.wl_surface().id(),
            "[input-method] the input method moved its popup"
        );
    }

    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, Logical> {
        // The popup is placed against the focused window, which is the surface the input
        // method's text input belongs to. A surface with no window is at the origin, which
        // is what a popup with no parent gets.
        match self.window_of(parent).and_then(|window| self.geometry_of(window)) {
            Some(rect) => Rectangle::new(
                (rect.x, rect.y).into(),
                (
                    i32::try_from(rect.width).unwrap_or(0),
                    i32::try_from(rect.height).unwrap_or(0),
                )
                    .into(),
            ),
            None => Rectangle::from_size((0, 0).into()),
        }
    }
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
                debug!("[virtual-keyboard] the device set changed");
            }
            _ => warn!("[virtual-keyboard] a virtual keyboard sent an event it cannot send"),
        }
    }
}
