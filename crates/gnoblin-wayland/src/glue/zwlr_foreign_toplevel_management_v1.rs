//! `gnoblin-wayland-zwlr-foreign-toplevel-management-v1`: the list lives on the state, and a taskbar's asks reach the
//! window manager as window requests.

use gnoblin_types::WindowId;
use gnoblin_wayland_protocols::zwlr_foreign_toplevel_management_v1::{ForeignToplevels, Host, ToplevelAsk};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use crate::State;
use crate::window_request::WindowRequest;

impl Host for State {
    fn foreign_toplevels(&mut self) -> &mut ForeignToplevels {
        &mut self.foreign_toplevels
    }

    fn toplevel_ask(&mut self, window: WindowId, ask: ToplevelAsk) {
        let request = match ask {
            ToplevelAsk::Close => WindowRequest::Close,
            ToplevelAsk::Maximise(on) => WindowRequest::Maximise(on),
            ToplevelAsk::Minimise(on) => WindowRequest::Minimise(on),
            ToplevelAsk::Fullscreen(on) => WindowRequest::Fullscreen(on),
            ToplevelAsk::Activate => WindowRequest::Activate,
        };
        self.window_request(window, request);
    }

    fn surface_of(&self, window: WindowId) -> Option<WlSurface> {
        State::surface_of(self, window)
    }

    fn sync_foreign_toplevels(&mut self) {
        State::sync_foreign_toplevels(self);
    }
}
