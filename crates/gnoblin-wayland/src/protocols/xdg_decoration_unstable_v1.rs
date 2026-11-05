//! `zxdg_decoration_manager_v1`.
//!
//! The session draws no frames in this slice, so it cannot promise server-side
//! decorations: a window drawn without a title bar and without telling the client
//! would be undecorated and unmovable. The answer is therefore always client-side,
//! which is what a GTK or Qt client falls back to when it hears nothing at all —
//! answering explicitly is better, because a client that asked stops guessing.
//!
//! When the window-frame protocol lands, this module answers with the server mode
//! and the frame module draws it; the handler and its single decision point are
//! already here, so the change is the `Mode` and nothing else.

use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
use smithay::wayland::shell::xdg::ToplevelSurface;
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;
use tracing::debug;

use crate::State;

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        self.answer(toplevel);
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: Mode) {
        debug!(?mode, "[decoration] the client asked for a mode");
        self.answer(toplevel);
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.answer(toplevel);
    }
}

impl State {
    /// Sends the decoration the session will draw: none, so the client draws its
    /// own.
    fn answer(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ClientSide);
        });
        // Only a toplevel that has already been configured can carry a decoration
        // configure: sending one before the initial configure would put two
        // configure events in flight for a client that has not committed yet.
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }
}
