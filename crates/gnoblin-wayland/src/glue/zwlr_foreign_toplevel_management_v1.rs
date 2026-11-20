//! `gnoblin-wayland-zwlr-foreign-toplevel-management-v1`: the list lives on the state, and a taskbar's asks reach the
//! window manager as window requests.

use gnoblin_types::WindowId;
use gnoblin_wayland_protocols::zwlr_foreign_toplevel_management_v1::{ForeignToplevels, Host};
use gnoblin_wayland_types::window::WindowRequest;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use crate::State;

impl Host for State {
    fn foreign_toplevels(&mut self) -> &mut ForeignToplevels {
        &mut self.foreign_toplevels
    }

    fn window_request(&mut self, window: WindowId, request: WindowRequest) {
        State::window_request(self, window, request);
    }

    fn surface_of(&self, window: WindowId) -> Option<WlSurface> {
        State::surface_of(self, window)
    }

    fn sync_foreign_toplevels(&mut self) {
        State::sync_foreign_toplevels(self);
    }
}
