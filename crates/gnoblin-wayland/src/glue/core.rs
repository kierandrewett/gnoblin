//! The host traits of `gnoblin-wayland-api`, which more than one protocol crate asks of the state.

use gnoblin_types::{OutputId, Rect, WindowId};
use gnoblin_wayland_api::host::{CoreHost, OutputHost, WindowHost};
use smithay::output::Output;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use crate::State;

impl CoreHost for State {
    fn display_handle(&self) -> &DisplayHandle {
        &self.display_handle
    }
}

impl WindowHost for State {
    fn window_of(&self, surface: &WlSurface) -> Option<WindowId> {
        State::window_of(self, surface)
    }

    fn sync_window_role(&mut self, window: WindowId) {
        State::sync_window_role(self, window);
    }
}

impl OutputHost for State {
    fn output_id_of(&self, output: &Output) -> Option<OutputId> {
        State::output_id_of(self, output)
    }

    fn output_rect(&self, output: OutputId) -> Option<Rect> {
        State::output_rect(self, output)
    }
}
