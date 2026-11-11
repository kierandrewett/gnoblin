//! XML protocol `xdg_shell` (`<protocol name="xdg_shell">`); interfaces served: `xdg_shell`.
//!
//! `xdg_shell`: toplevels, popups, configure and acknowledge.
//!
//! A toplevel is taken into management when the client creates it, and it takes
//! focus when it first commits a buffer: focus follows the map, which is what the
//! window manager's activation event describes and what a person sees.
//!
//! ## Configure ordering
//!
//! The initial configure is sent during the client's first commit, not when the
//! toplevel is created. xdg-shell says the compositor answers the first commit of a
//! surface with its initial configure, and a configure sent before that commit is
//! out of order for a client that has not committed yet. Later configures — a
//! maximise, a resize, an output change — are sent as soon as the window manager
//! asks for them.
//!
//! ## Popups
//!
//! A popup is tracked by Smithay's popup manager so a parentless popup is remembered
//! until it has a parent, and a popup's initial configure is sent on its own first
//! commit. Grabs, the popup under the pointer and dismissal are in `popup.rs`.

use smithay::desktop::{PopupKind, find_popup_root_surface, get_popup_toplevel_coords};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_seat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Serial, Size};
use smithay::wayland::shell::xdg::{
    Configure, PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
};

use crate::State;
use crate::observation::{MenuAnchor, MenuSource};
use crate::tree_frame::TreeRequest;
use crate::window_request::WindowRequest;

use crate::log;

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    /// The client asks for the compositor's window menu at a point on its surface.
    ///
    /// The request is honoured only with a serial the session handed to this client on
    /// an input event, which is the protocol's rule against a client opening menus on its
    /// own. A serial the session never issued, or one that has aged out, is ignored with
    /// a debug log, and nothing is shown. The point is surface-local, so it is shifted by
    /// the window's own origin to give the output position the menu is anchored at.
    fn show_window_menu(
        &mut self,
        surface: ToplevelSurface,
        _seat: wl_seat::WlSeat,
        serial: Serial,
        location: Point<i32, Logical>,
    ) {
        if self.request_to_shell(surface.wl_surface(), TreeRequest::Menu) {
            return;
        }
        if !self.serial_issued(u32::from(serial)) {
            log::debug!(
                area = "xdg",
                serial = u32::from(serial),
                "a window menu with an unissued serial was ignored"
            );
            return;
        }
        let Some(window) = self.window_of(surface.wl_surface()) else {
            log::debug!(area = "xdg", "a window menu for an unmanaged surface was ignored");
            return;
        };
        let Some(origin) = self.geometry_of(window) else {
            log::debug!(area = "xdg", ?window, "a window menu for a closed window was ignored");
            return;
        };
        let anchor = MenuAnchor::point(origin.x + location.x, origin.y + location.y);
        self.request_window_menu(window, anchor, MenuSource::Client);
    }

    fn client_pong(&mut self, client: smithay::wayland::shell::xdg::ShellClient) {
        self.record_pong(&client);
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // The configure is sent on the first commit, not here: see the module
        // documentation.
        self.toplevel_created(surface);
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        if let Err(error) = self.popups.track_popup(PopupKind::Xdg(surface)) {
            log::warn!(area = "xdg", ?error, "a popup could not be tracked");
        }
    }

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|state| {
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        self.grab_popup(surface, &seat, serial);
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        self.toplevel_request(&surface, WindowRequest::Maximise(true));
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        self.toplevel_request(&surface, WindowRequest::Maximise(false));
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, _output: Option<WlOutput>) {
        self.toplevel_request(&surface, WindowRequest::Fullscreen(true));
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        self.toplevel_request(&surface, WindowRequest::Fullscreen(false));
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        self.toplevel_request(&surface, WindowRequest::Minimise(true));
    }

    fn move_request(&mut self, surface: ToplevelSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        self.request_to_shell(surface.wl_surface(), TreeRequest::Move);
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        _seat: wl_seat::WlSeat,
        _serial: Serial,
        _edges: xdg_toplevel::ResizeEdge,
    ) {
        self.request_to_shell(surface.wl_surface(), TreeRequest::Resize);
    }

    fn ack_configure(&mut self, surface: WlSurface, configure: Configure) {
        // The acknowledgement is the client's confirmation of a configure. Nothing
        // waits on it: an unacknowledged configure costs that client its own
        // window, and the compositor keeps serving every other one.
        let serial = match configure {
            Configure::Toplevel(toplevel) => toplevel.serial,
            Configure::Popup(popup) => popup.serial,
        };
        log::debug!(area = "xdg", serial = ?serial, surface = ?surface.id(), "configure acknowledged");
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.toplevel_destroyed(&surface);
    }

    fn parent_changed(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_of(surface.wl_surface()) {
            self.sync_window_role(window);
        }
    }

    fn popup_destroyed(&mut self, surface: PopupSurface) {
        log::trace!(area = "xdg", surface = ?surface.wl_surface().id(), "popup destroyed");
        self.popup_gone(&surface);
    }
}

impl State {
    /// Tells the popup manager about a commit and sends a popup's initial configure.
    pub(crate) fn popup_committed(&mut self, surface: &WlSurface) {
        let Some(PopupKind::Xdg(popup)) = self.popups.find_popup(surface) else {
            return;
        };
        // A popup is not drawn, so its commit damages nothing by itself; the outputs are repainted so the frame
        // that answers its frame callback is one that happens.
        self.damage_outputs();
        if popup.is_initial_configure_sent() {
            return;
        }
        if let Err(error) = popup.send_configure() {
            log::warn!(area = "xdg", ?error, "a popup configure could not be sent");
        }
    }

    /// Hands a toplevel's request to whoever decides it: the shell, or the window manager when none is attached.
    fn toplevel_request(&mut self, surface: &ToplevelSurface, request: WindowRequest) {
        if let Some(window) = self.window_of(surface.wl_surface()) {
            self.window_request(window, request);
        }
    }

    /// Moves a popup inside its output.
    ///
    /// A positioner places a popup relative to its parent's window geometry; the
    /// result is clamped to the monitor the parent is on, so a menu opened at the
    /// bottom of the screen flips up instead of hanging off the edge. A popup whose
    /// root is a layer surface is clamped to that surface's own rectangle instead: a
    /// bar's menu belongs over the bar, not over the whole monitor.
    pub(crate) fn unconstrain_popup(&mut self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else {
            log::trace!(area = "xdg", "a popup has no root surface yet");
            return;
        };
        let (rectangle, origin) = match (self.window_of(&root), self.layer_rect(&root)) {
            (Some(window), _) => {
                let (Some(geometry), Some(monitor)) = (self.geometry_of(window), self.monitor_of(window)) else {
                    return;
                };
                (
                    Rectangle::new(
                        Point::from((monitor.x, monitor.y)),
                        Size::from((monitor.width as i32, monitor.height as i32)),
                    ),
                    Point::from((geometry.x, geometry.y)),
                )
            }
            (None, Some(rect)) => (
                Rectangle::new(
                    Point::from((rect.x, rect.y)),
                    Size::from((rect.width as i32, rect.height as i32)),
                ),
                Point::from((rect.x, rect.y)),
            ),
            (None, None) => return,
        };
        let mut target = rectangle;
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        target.loc -= origin;
        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}
