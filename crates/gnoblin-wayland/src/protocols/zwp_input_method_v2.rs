//! XML protocol `input_method_unstable_v2` (`<protocol name="input_method_unstable_v2">`); interfaces served: `zwp_input_method_manager_v2`.
//!
//! `zwp_input_method_manager_v2` and `zwp_input_method_v2` (XML `input-method-unstable-v2`).
//!
//! An input method (IBus, fcitx, an on-screen keyboard) holds a `zwp_input_method_v2` on the
//! seat and answers the focused text input with `commit_string`, `preedit_string` and
//! `delete_surrounding_text`. It is privileged: it sees what the user types everywhere, so
//! [`AccessPolicy`](crate::access::AccessPolicy) decides who may bind the global.

use std::sync::Arc;

use gnoblin_log::debug_in;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Rectangle};
use smithay::wayland::input_method::{InputMethodHandler, InputMethodManagerState, PopupSurface};

use crate::State;
use crate::access::{Access, AccessPolicy, Global};

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

impl InputMethodHandler for State {
    fn new_popup(&mut self, surface: PopupSurface) {
        debug_in!("wayland", popup = ?surface.wl_surface().id(), "[input-method] the input method created a popup");
        self.input_method_popups.push(surface);
    }

    fn dismiss_popup(&mut self, surface: PopupSurface) {
        self.input_method_popups
            .retain(|held| held.wl_surface() != surface.wl_surface());
    }

    fn popup_repositioned(&mut self, surface: PopupSurface) {
        debug_in!("wayland",
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
