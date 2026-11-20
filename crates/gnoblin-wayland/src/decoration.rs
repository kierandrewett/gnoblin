//! `xdg-decoration`: who draws a window's frame.
//!
//! The protocol crate (`gnoblin-wayland-zxdg-decoration-v1`) holds the global and the answer's rule. This module is the
//! compositor's side: it records what a toplevel's client asked for on the window record, and sends each window the
//! decoration the session draws for it now, publishing it on the window's node.

use gnoblin_types::WindowId;
use gnoblin_wayland_protocols::zxdg_decoration_v1::{negotiated, send_mode};
use gnoblin_wayland_types::decoration::{DecorationWanted, WindowDecoration};
use smithay::wayland::shell::xdg::ToplevelSurface;

use crate::State;
use crate::log;
use crate::state::Managed;

impl State {
    /// Records what a toplevel's client asked for and answers it.
    pub(crate) fn decoration_asked(&mut self, toplevel: &ToplevelSurface, wanted: DecorationWanted) {
        let Some(window) = self.window_of(toplevel.wl_surface()) else {
            return;
        };
        if let Some(entry) = self.windows.get_mut(&window) {
            entry.decoration_asked = Some(wanted);
        }
        self.apply_decoration(window);
    }

    /// Sends a window the decoration the session draws for it now, and publishes it on the node: the answer a
    /// Wayland client that bound the manager hears, and the frame the shell gives an X11 window.
    pub(crate) fn apply_decoration(&mut self, window: WindowId) {
        let available = self.composer.decorations_available();
        let Some(entry) = self.windows.get(&window) else {
            return;
        };
        let (wanted, decoration) = match &entry.surface {
            Managed::X11(_) => (
                DecorationWanted::Unset,
                if available {
                    WindowDecoration::Server
                } else {
                    WindowDecoration::None
                },
            ),
            Managed::Wayland(toplevel) => match entry.decoration_asked {
                // No decoration object: the client draws what it draws and was told nothing.
                None => (DecorationWanted::Unset, WindowDecoration::Client),
                Some(wanted) => {
                    let decoration = negotiated(wanted, available);
                    send_mode(toplevel, decoration);
                    (wanted, decoration)
                }
            },
        };
        self.composer.set_window_decoration(window, wanted, decoration);
    }

    /// Keeps the windows' answers in line with whether the shell draws decorations. The shell's declaration is
    /// released when the shell no longer holds the tree (its connection dropped, or it crashed), and a change in
    /// whether decorations are available is sent to every window, so a reload that turns them off or on never leaves
    /// a window that asked for server-side without a frame.
    pub(crate) fn follow_decorations_available(&mut self) {
        if !self.composer.holder_attached() && self.composer.decorations_declared() {
            self.composer.release_decorations_declared();
        }
        let available = self.composer.decorations_available();
        if available == self.decorations_seen {
            return;
        }
        self.decorations_seen = available;
        let windows: Vec<WindowId> = self.windows.keys().copied().collect();
        for window in windows {
            self.apply_decoration(window);
        }
        log::debug!(area = "decoration", available, "the shell's decorations changed");
    }
}
