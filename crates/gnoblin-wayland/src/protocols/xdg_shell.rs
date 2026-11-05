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
//! commit. Interactive grabs are not part of this slice: the pointer focus machinery
//! a grab needs arrives with the input slice, so a grab request for a popup with no
//! parent is answered with `xdg_popup.popup_done` and a grab for a valid popup is
//! logged and otherwise ignored rather than half-implemented.

use smithay::desktop::{PopupKind, find_popup_root_surface, get_popup_toplevel_coords};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_seat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Point, Rectangle, Serial, Size};
use smithay::wayland::shell::xdg::{
    Configure, PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
};
use tracing::{debug, trace, warn};

use gnoblin_wm::Event;

use crate::State;

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // The configure is sent on the first commit, not here: see the module
        // documentation.
        self.toplevel_created(surface);
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        if let Err(error) = self.popups.track_popup(PopupKind::Xdg(surface)) {
            warn!(?error, "[xdg] a popup could not be tracked");
        }
    }

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|state| {
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn grab(&mut self, surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        if self.popup_root_window(&surface).is_none() && self.popup_root_layer(&surface).is_none() {
            // Nothing will ever send this popup `popup_done`, because it has no
            // parent to be dismissed with, so it is dismissed here. Without this the
            // popup stays on screen for the life of the session.
            surface.send_popup_done();
            debug!("[xdg] a parentless popup grab was dismissed");
        } else {
            trace!("[xdg] popup grabs arrive with the input slice");
        }
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        if !self.is_maximised(&surface) {
            self.window_event(&surface, |window| Event::ToggleMaximise { window });
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        if self.is_maximised(&surface) {
            self.window_event(&surface, |window| Event::ToggleMaximise { window });
        }
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, _output: Option<WlOutput>) {
        if !self.is_fullscreen(&surface) {
            self.window_event(&surface, |window| Event::ToggleFullscreen { window });
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if self.is_fullscreen(&surface) {
            self.window_event(&surface, |window| Event::ToggleFullscreen { window });
        }
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        self.window_event(&surface, |window| Event::Minimise { window });
    }

    fn ack_configure(&mut self, surface: WlSurface, configure: Configure) {
        // The acknowledgement is the client's confirmation of a configure. Nothing
        // waits on it: an unacknowledged configure costs that client its own
        // window, and the compositor keeps serving every other one.
        let serial = match configure {
            Configure::Toplevel(toplevel) => toplevel.serial,
            Configure::Popup(popup) => popup.serial,
        };
        debug!(serial = ?serial, surface = ?surface.id(), "[xdg] configure acknowledged");
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
        trace!(surface = ?surface.wl_surface().id(), "[xdg] popup destroyed");
    }
}

impl State {
    /// Tells the popup manager about a commit and sends a popup's initial configure.
    pub(crate) fn popup_committed(&mut self, surface: &WlSurface) {
        let Some(PopupKind::Xdg(popup)) = self.popups.find_popup(surface) else {
            return;
        };
        if popup.is_initial_configure_sent() {
            return;
        }
        if let Err(error) = popup.send_configure() {
            warn!(?error, "[xdg] a popup configure could not be sent");
        }
    }

    /// The window a popup belongs to, if its parent chain reaches a managed one.
    fn popup_root_window(&self, surface: &PopupSurface) -> Option<gnoblin_types::WindowId> {
        let root = find_popup_root_surface(&PopupKind::Xdg(surface.clone())).ok()?;
        self.window_of(&root)
    }

    /// The layer surface a popup belongs to, if its parent chain reaches one.
    ///
    /// A bar's menu is a popup whose root is a layer surface, which is not a managed
    /// window: it has a rectangle of its own and that is the area the menu is held
    /// inside.
    fn popup_root_layer(&self, surface: &PopupSurface) -> Option<gnoblin_types::Rect> {
        let root = find_popup_root_surface(&PopupKind::Xdg(surface.clone())).ok()?;
        self.layer_rect(&root)
    }

    /// Whether the window manager has this toplevel maximised.
    fn is_maximised(&self, surface: &ToplevelSurface) -> bool {
        self.window_state(surface).is_some_and(|state| state.maximised)
    }

    /// Whether the window manager has this toplevel fullscreen.
    fn is_fullscreen(&self, surface: &ToplevelSurface) -> bool {
        self.window_state(surface).is_some_and(|state| state.fullscreen)
    }

    /// Moves a popup inside its output.
    ///
    /// A positioner places a popup relative to its parent's window geometry; the
    /// result is clamped to the monitor the parent is on, so a menu opened at the
    /// bottom of the screen flips up instead of hanging off the edge. A popup whose
    /// root is a layer surface is clamped to that surface's own rectangle instead: a
    /// bar's menu belongs over the bar, not over the whole monitor.
    fn unconstrain_popup(&mut self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else {
            trace!("[xdg] a popup has no root surface yet");
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
