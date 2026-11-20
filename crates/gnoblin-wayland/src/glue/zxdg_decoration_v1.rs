//! `gnoblin-wayland-zxdg-decoration-v1`: a client's ask is recorded on its window and answered by the state.

use gnoblin_wayland_protocols::zxdg_decoration_v1::wanted_of;
use gnoblin_wayland_types::decoration::DecorationWanted;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
use smithay::wayland::shell::xdg::ToplevelSurface;
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;

use crate::State;
use crate::log;

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        self.decoration_asked(&toplevel, DecorationWanted::Unset);
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: Mode) {
        log::debug!(area = "decoration", ?mode, "the client asked for a mode");
        self.decoration_asked(&toplevel, wanted_of(mode));
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.decoration_asked(&toplevel, DecorationWanted::Unset);
    }
}
