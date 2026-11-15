//! XML protocol `xdg_decoration_unstable_v1` (`<protocol name="xdg_decoration_unstable_v1">`); interfaces served: `zxdg_decoration_manager_v1`.
//!
//! `zxdg_decoration_manager_v1`.
//!
//! Who draws a window's frame is negotiated here and published on the window's node: `decoration_wanted` is what the
//! client asked for (`client`, `server`, or `unset`) and `decoration` is the answer (`client`, `server`, or `none`).
//!
//! Gnoblin's own decorations are the stock `Titlebar` shell node, not a protocol a client speaks
//! (`design/archive/node-api-refactor.md`, Decision 2). The compositor draws no frame itself, so it promises server-side
//! only while a shell that holds the tree has declared that it draws one: the main seat's `decorations_available`. A
//! window that asked for server-side with nobody drawing would be undecorated and unmovable, so with no shell, or
//! one that has not declared decorations, every client is told client-side.
//!
//! - A client that asks for client-side is told client-side, and so is one that asks for no preference.
//! - A client that asks for server-side is told server-side while the shell declares decorations, and is sent
//!   client-side again when that stops (a shell reload that turns decorations off, or a shell that dies).
//! - A Wayland client that never binds the manager draws its own frame: it is `client` and `unset`.
//! - An X11 window has no decoration protocol. The shell frames it, so it is `server` while the shell declares
//!   decorations and `none` while it does not ([`State::apply_decoration`]).
//!
//! # Failure modes
//!
//! 1. **A toplevel destroyed before its decoration.** The protocol makes it the client's error: the decoration object
//!    must be destroyed first. Smithay keeps the decoration object where nothing else can read it and says nothing, so
//!    the compositor remembers which toplevels have a live decoration and the manager that made it
//!    ([`DecorationLink`]), and answers the destruction of such a toplevel with `orphaned` on that manager, which is
//!    where Smithay posts the protocol's other two errors (`already_constructed`, and `orphaned` for a toplevel that
//!    is already gone when the decoration is asked for): `destroying_a_toplevel_before_its_decoration_is_an_error`
//!    in `tests/decoration.rs`.
//! 2. **A mode asked for before the first commit is lost.** The window record exists from `get_toplevel`, so the
//!    preference is recorded and the answer travels with the toplevel's initial configure, which is the answer to the
//!    client's first commit.

use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_decoration_manager_v1::ZxdgDecorationManagerV1;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::{
    Error, Mode, ZxdgToplevelDecorationV1,
};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel;
use smithay::reexports::wayland_server::{Resource, Weak};
use smithay::wayland::shell::xdg::ToplevelSurface;
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;

use gnoblin_backend::render::compose::{DecorationWanted, WindowDecoration};
use gnoblin_types::WindowId;

use crate::State;
use crate::state::Managed;

use crate::log;

/// A toplevel that has a live decoration object, and the manager that created it.
pub(crate) struct DecorationLink {
    toplevel: Weak<XdgToplevel>,
    manager: ZxdgDecorationManagerV1,
}

/// Whether a link is for `toplevel`.
fn is_for(link: &DecorationLink, toplevel: &XdgToplevel) -> bool {
    link.toplevel.upgrade().is_ok_and(|held| held == *toplevel)
}

/// A manager is asked to decorate `toplevel`: remembers the link, unless Smithay is going to refuse the request
/// (a toplevel that already has a decoration or is gone), which it does with its own error.
pub(crate) fn decoration_requested(state: &mut State, manager: &ZxdgDecorationManagerV1, toplevel: &XdgToplevel) {
    state
        .decoration_links
        .retain(|link| link.manager.is_alive() && link.toplevel.upgrade().is_ok());
    if !toplevel.is_alive() || state.decoration_links.iter().any(|link| is_for(link, toplevel)) {
        return;
    }
    state.decoration_links.push(DecorationLink {
        toplevel: toplevel.downgrade(),
        manager: manager.clone(),
    });
}

/// A decoration object is destroyed, which is how a client ends it in order.
pub(crate) fn decoration_destroyed(state: &mut State, decoration: &ZxdgToplevelDecorationV1) {
    let Some(toplevel) = decoration
        .data::<ToplevelSurface>()
        .map(|surface| surface.xdg_toplevel().clone())
    else {
        return;
    };
    state.decoration_links.retain(|link| !is_for(link, &toplevel));
}

/// A toplevel is being destroyed: if its decoration is still alive the client broke the protocol's order.
pub(crate) fn toplevel_destroying(state: &mut State, toplevel: &XdgToplevel) {
    let Some(position) = state.decoration_links.iter().position(|link| is_for(link, toplevel)) else {
        return;
    };
    let link = state.decoration_links.swap_remove(position);
    link.manager.post_error(
        Error::Orphaned,
        "the xdg_toplevel was destroyed before its decoration object",
    );
}

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        self.decoration_asked(&toplevel, DecorationWanted::Unset);
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: Mode) {
        log::debug!(area = "decoration", ?mode, "the client asked for a mode");
        let wanted = match mode {
            Mode::ServerSide => DecorationWanted::Server,
            _ => DecorationWanted::Client,
        };
        self.decoration_asked(&toplevel, wanted);
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.decoration_asked(&toplevel, DecorationWanted::Unset);
    }
}

/// What a client that asked for `wanted` is told, given whether a shell draws decorations. Server-side only when the
/// client asked for it and somebody will draw the frame.
#[must_use]
pub(crate) const fn negotiated(wanted: DecorationWanted, available: bool) -> WindowDecoration {
    match (wanted, available) {
        (DecorationWanted::Server, true) => WindowDecoration::Server,
        _ => WindowDecoration::Client,
    }
}

impl State {
    /// Records what a toplevel's client asked for and answers it.
    fn decoration_asked(&mut self, toplevel: &ToplevelSurface, wanted: DecorationWanted) {
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
        if !self.composer.shell_places() && self.composer.decorations_declared() {
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

/// Puts the answer in the toplevel's pending state and sends it. A toplevel that has not had its initial configure
/// carries it in that one: sending a configure earlier would put two in flight for a client that has not committed.
fn send_mode(toplevel: &ToplevelSurface, decoration: WindowDecoration) {
    let mode = match decoration {
        WindowDecoration::Server => Mode::ServerSide,
        WindowDecoration::Client | WindowDecoration::None => Mode::ClientSide,
    };
    toplevel.with_pending_state(|state| {
        state.decoration_mode = Some(mode);
    });
    if toplevel.is_initial_configure_sent() {
        toplevel.send_pending_configure();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_server_side_ask_with_a_shell_drawing_is_answered_server_side() {
        for (wanted, available, answer) in [
            (DecorationWanted::Server, true, WindowDecoration::Server),
            (DecorationWanted::Server, false, WindowDecoration::Client),
            (DecorationWanted::Client, true, WindowDecoration::Client),
            (DecorationWanted::Client, false, WindowDecoration::Client),
            (DecorationWanted::Unset, true, WindowDecoration::Client),
            (DecorationWanted::Unset, false, WindowDecoration::Client),
        ] {
            assert_eq!(
                negotiated(wanted, available),
                answer,
                "{wanted:?}, available {available}"
            );
        }
    }
}
