//! `gnoblin-wayland-xdg-toplevel-icon-v1`: the icon is read on the next toplevel commit.

use gnoblin_wayland_protocols::xdg_toplevel_icon_v1;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::xdg_toplevel_icon::XdgToplevelIconHandler;

use crate::State;

impl XdgToplevelIconHandler for State {
    fn set_icon(&mut self, toplevel: XdgToplevel, wl_surface: WlSurface) {
        xdg_toplevel_icon_v1::set_icon(&toplevel, &wl_surface);
    }
}
