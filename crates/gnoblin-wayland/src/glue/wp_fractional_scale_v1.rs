//! `gnoblin-wayland-wp-fractional-scale-v1`: the scale is the output's, and the surfaces that asked live on the state.

use gnoblin_wayland_protocols::wp_fractional_scale_v1::Host;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use crate::State;

impl Host for State {
    fn surface_scale(&self, surface: &WlSurface) -> f64 {
        State::surface_scale(self, surface)
    }

    fn fractional_surfaces(&mut self) -> &mut Vec<WlSurface> {
        &mut self.fractional_surfaces
    }
}
