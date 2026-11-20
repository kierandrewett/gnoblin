//! XML protocol `xdg_decoration_unstable_v1` (`<protocol name="xdg_decoration_unstable_v1">`); interfaces served: `zxdg_decoration_manager_v1`.
//!
//! `zxdg_decoration_manager_v1`.
//!
//! Who draws a window's frame is negotiated here and published on the window's node: `decoration_wanted` is what the
//! client asked for (`client`, `server`, or `unset`) and `decoration` is the answer (`client`, `server`, or `none`).
//! The two enums are [`DecorationWanted`] and [`WindowDecoration`], in `gnoblin-wayland-types`.
//!
//! Gnoblin's own decorations are the stock `Titlebar` shell node, not a protocol a client speaks.
//! The compositor draws no frame itself, so it promises server-side
//! only while a shell that holds the tree has declared that it draws one: the main seat's `decorations_available`. A
//! window that asked for server-side with nobody drawing would be undecorated and unmovable, so with no shell, or
//! one that has not declared decorations, every client is told client-side ([`negotiated`]).
//!
//! - A client that asks for client-side is told client-side, and so is one that asks for no preference.
//! - A client that asks for server-side is told server-side while the shell declares decorations, and is sent
//!   client-side again when that stops (a shell reload that turns decorations off, or a shell that dies).
//! - A Wayland client that never binds the manager draws its own frame: it is `client` and `unset`.
//! - An X11 window has no decoration protocol. The shell frames it, so it is `server` while the shell declares
//!   decorations and `none` while it does not.
//!
//! The compositor answers the requests: it implements Smithay's `XdgDecorationHandler`, reads the ask with
//! [`wanted_of`] and sends the answer with [`send_mode`]. This crate holds the global and the table that catches a
//! protocol error Smithay does not.
//!
//! # Failure modes
//!
//! 1. **A toplevel destroyed before its decoration.** The protocol makes it the client's error: the decoration object
//!    must be destroyed first. Smithay keeps the decoration object where nothing else can read it and says nothing, so
//!    the compositor remembers which toplevels have a live decoration and the manager that made it
//!    ([`DecorationLinks`]), and answers the destruction of such a toplevel with `orphaned` on that manager, which is
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
use smithay::reexports::wayland_server::{Dispatch, GlobalDispatch, Resource, Weak};
use smithay::wayland::shell::xdg::ToplevelSurface;
use smithay::wayland::shell::xdg::decoration::{
    XdgDecorationHandler, XdgDecorationManagerGlobalData, XdgDecorationState,
};

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};
use gnoblin_wayland_types::decoration::{DecorationWanted, WindowDecoration};

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "zxdg-decoration-v1",
    interfaces: &[ServedInterface::new("zxdg_decoration_manager_v1", 1)],
    privilege: Privilege::Core,
};

/// Creates the `zxdg_decoration_manager_v1` global.
///
/// The compositor calls this only while the protocol is on in the configuration, so a client of a session that turned it
/// off sees no `xdg-decoration` and draws its own frame.
pub fn register<D>(registrar: &Registrar<'_>) -> XdgDecorationState
where
    D: GlobalDispatch<ZxdgDecorationManagerV1, XdgDecorationManagerGlobalData>
        + Dispatch<ZxdgDecorationManagerV1, smithay::wayland::GlobalData>
        + XdgDecorationHandler
        + 'static,
{
    XdgDecorationState::new::<D>(registrar.display())
}

/// A toplevel that has a live decoration object, and the manager that created it.
struct DecorationLink {
    toplevel: Weak<XdgToplevel>,
    manager: ZxdgDecorationManagerV1,
}

/// Whether a link is for `toplevel`.
fn is_for(link: &DecorationLink, toplevel: &XdgToplevel) -> bool {
    link.toplevel.upgrade().is_ok_and(|held| held == *toplevel)
}

/// The decoration objects that are alive and the managers that made them.
#[derive(Default)]
pub struct DecorationLinks {
    links: Vec<DecorationLink>,
}

impl DecorationLinks {
    /// A manager is asked to decorate `toplevel`: remembers the link, unless Smithay is going to refuse the request
    /// (a toplevel that already has a decoration or is gone), which it does with its own error.
    pub fn requested(&mut self, manager: &ZxdgDecorationManagerV1, toplevel: &XdgToplevel) {
        self.links
            .retain(|link| link.manager.is_alive() && link.toplevel.upgrade().is_ok());
        if !toplevel.is_alive() || self.links.iter().any(|link| is_for(link, toplevel)) {
            return;
        }
        self.links.push(DecorationLink {
            toplevel: toplevel.downgrade(),
            manager: manager.clone(),
        });
    }

    /// A decoration object is destroyed, which is how a client ends it in order.
    pub fn destroyed(&mut self, decoration: &ZxdgToplevelDecorationV1) {
        let Some(toplevel) = decoration
            .data::<ToplevelSurface>()
            .map(|surface| surface.xdg_toplevel().clone())
        else {
            return;
        };
        self.links.retain(|link| !is_for(link, &toplevel));
    }

    /// A toplevel is being destroyed: if its decoration is still alive the client broke the protocol's order.
    pub fn toplevel_destroying(&mut self, toplevel: &XdgToplevel) {
        let Some(position) = self.links.iter().position(|link| is_for(link, toplevel)) else {
            return;
        };
        let link = self.links.swap_remove(position);
        link.manager.post_error(
            Error::Orphaned,
            "the xdg_toplevel was destroyed before its decoration object",
        );
    }
}

/// What a client that asked for `wanted` is told, given whether a shell draws decorations. Server-side only when the
/// client asked for it and somebody will draw the frame.
#[must_use]
pub const fn negotiated(wanted: DecorationWanted, available: bool) -> WindowDecoration {
    match (wanted, available) {
        (DecorationWanted::Server, true) => WindowDecoration::Server,
        _ => WindowDecoration::Client,
    }
}

/// What a client asked for with a mode: server-side, or anything else as client-side.
#[must_use]
pub const fn wanted_of(mode: Mode) -> DecorationWanted {
    match mode {
        Mode::ServerSide => DecorationWanted::Server,
        _ => DecorationWanted::Client,
    }
}

/// Puts the answer in the toplevel's pending state and sends it. A toplevel that has not had its initial configure
/// sent carries it in that one: sending a configure earlier would put two in flight for a client that has not committed.
pub fn send_mode(toplevel: &ToplevelSurface, decoration: WindowDecoration) {
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
