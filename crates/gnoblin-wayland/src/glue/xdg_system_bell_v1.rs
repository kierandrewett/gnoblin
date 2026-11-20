//! `gnoblin-wayland-xdg-system-bell-v1`: a ring is counted and flashed by the state.

use gnoblin_types::WindowId;
use gnoblin_wayland_protocols::xdg_system_bell_v1;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::xdg_system_bell::XdgSystemBellHandler;

use crate::State;

impl xdg_system_bell_v1::Host for State {
    fn ring_bell(&mut self, window: Option<WindowId>) {
        State::ring_bell(self, window);
    }
}

impl XdgSystemBellHandler for State {
    fn ring(&mut self, surface: Option<WlSurface>) {
        xdg_system_bell_v1::ring(self, surface);
    }
}
