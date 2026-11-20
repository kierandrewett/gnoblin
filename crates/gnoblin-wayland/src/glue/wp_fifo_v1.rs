//! `gnoblin-wayland-wp-fifo-v1`: the barriers of every mapped window are released from the frame loop.

use gnoblin_wayland_protocols::wp_fifo_v1::Host;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use crate::State;

impl Host for State {
    fn mapped_surfaces(&self) -> Vec<WlSurface> {
        self.windows
            .values()
            .filter(|entry| entry.mapped)
            .filter_map(|entry| entry.surface.wl_surface())
            .collect()
    }
}
