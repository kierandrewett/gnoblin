//! `gnoblin-wayland-zwp-input-method-v2`: the popups live on the state, and are placed against its windows.

use gnoblin_wayland_protocols::zwp_input_method_v2 as input_method;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Rectangle};
use smithay::wayland::input_method::{InputMethodHandler, PopupSurface};

use crate::State;

impl input_method::Host for State {
    fn input_method_popups(&mut self) -> &mut Vec<PopupSurface> {
        &mut self.input_method_popups
    }
}

impl InputMethodHandler for State {
    fn new_popup(&mut self, surface: PopupSurface) {
        input_method::new_popup(self, surface);
    }

    fn dismiss_popup(&mut self, surface: PopupSurface) {
        input_method::dismiss_popup(self, &surface);
    }

    fn popup_repositioned(&mut self, surface: PopupSurface) {
        input_method::popup_repositioned(&surface);
    }

    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, Logical> {
        input_method::parent_geometry(self, parent)
    }
}
