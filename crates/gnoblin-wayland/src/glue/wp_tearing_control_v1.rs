//! `gnoblin-wayland-wp-tearing-control-v1`: the hints live on the state.

use gnoblin_wayland_protocols::wp_tearing_control_v1::{Host, WpTearingControlV1Hint};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use crate::State;

impl Host for State {
    fn tearing_hints(&self) -> &[(WlSurface, WpTearingControlV1Hint)] {
        &self.tearing
    }

    fn tearing_hints_mut(&mut self) -> &mut Vec<(WlSurface, WpTearingControlV1Hint)> {
        &mut self.tearing
    }
}
