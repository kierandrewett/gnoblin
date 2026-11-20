//! `gnoblin-wayland-zwp-idle-inhibit-v1`: the inhibitors are recorded with the idle notifications.

use gnoblin_wayland_protocols::zwp_idle_inhibit_v1;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::idle_inhibit::IdleInhibitHandler;

use crate::State;

impl zwp_idle_inhibit_v1::Host for State {
    fn set_idle_inhibited(&mut self, surface: &WlSurface, inhibited: bool) {
        State::set_idle_inhibited(self, surface, inhibited);
    }
}

impl IdleInhibitHandler for State {
    fn inhibit(&mut self, surface: WlSurface) {
        zwp_idle_inhibit_v1::inhibit(self, &surface);
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        zwp_idle_inhibit_v1::uninhibit(self, &surface);
    }
}
