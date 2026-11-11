//! Popups: where the pointer finds them, how a grab holds the seat, and when one is dismissed.
//!
//! An `xdg_popup` is placed by its positioner relative to the window geometry of its parent, and a popup may be
//! the parent of another. The pointer reaches a popup wherever it is, including outside the window it belongs to:
//! a menu opened at the edge of a window hangs past it, and a pointer over the part that hangs is over the menu.
//! A popup is above its own window and above every popup beneath it in the tree, and a window above that window
//! covers both.
//!
//! A popup that asked for a grab takes the keyboard, and the pointer follows the client: a press that lands on
//! another client dismisses the grab, children first. Smithay's `PopupGrab` is the mechanism, and what this module
//! adds is the rule that is the compositor's own: a window that takes keyboard focus from the popup's window ends
//! the grab, because the popup would otherwise stay open over a window nobody can see it belongs to.
//!
//! A popup is not drawn: the scene holds one buffer per window, so a popup's pixels never reach an output. The
//! compositor answers its frame callbacks all the same (a client that paces a menu by them would never draw it
//! otherwise), and it finds the popup under the pointer from the position the compositor last configured it at.

use smithay::backend::input::InputTime;
use smithay::desktop::utils::under_from_surface_tree;
use smithay::desktop::{PopupGrab, PopupKeyboardGrab, PopupKind, PopupPointerGrab, PopupUngrabStrategy};
use smithay::desktop::{PopupManager, WindowSurfaceType, find_popup_root_surface};
use smithay::input::Seat;
use smithay::input::pointer::{Focus, MotionEvent};
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{IsAlive, Logical, Point, SERIAL_COUNTER, Serial};
use smithay::wayland::compositor::{get_parent, get_role, with_states};
use smithay::wayland::shell::xdg::{PopupSurface, SurfaceCachedState, XDG_POPUP_ROLE, XdgPopupSurfaceData};

use gnoblin_types::WindowId;

use crate::State;
use crate::log;

/// A popup grab the seat holds, with the surface of the window its popups hang from.
pub(crate) struct HeldGrab {
    /// The surface at the root of the popup chain: the window's, or a layer surface's.
    root: WlSurface,
    grab: PopupGrab<State>,
}

/// Where a popup's window geometry starts, relative to its parent's, as the compositor last configured it.
///
/// The configured position is the one that counts for finding the popup: the compositor decided it and told the
/// client, and a client that was told to move a popup draws it there from its next commit. Waiting for the client's
/// acknowledgement or commit would leave the pointer hitting the old place for the round trip between them.
fn configured_location(surface: &WlSurface) -> Point<i32, Logical> {
    with_states(surface, |states| {
        states
            .data_map
            .get::<XdgPopupSurfaceData>()
            .and_then(|data| Some(data.lock().ok()?.current_server_state().geometry.loc))
            .unwrap_or_default()
    })
}

/// A popup's window geometry origin relative to its root's window geometry origin: its own location plus that of
/// each popup it hangs from.
fn location_in_root(popup: &PopupSurface) -> Point<i32, Logical> {
    let mut total = configured_location(popup.wl_surface());
    let mut parent = popup.get_parent_surface();
    while let Some(surface) = parent.filter(|surface| get_role(surface) == Some(XDG_POPUP_ROLE)) {
        total += configured_location(&surface);
        parent = with_states(&surface, |states| {
            states
                .data_map
                .get::<XdgPopupSurfaceData>()
                .and_then(|data| data.lock().ok()?.parent.clone())
        });
    }
    total
}

/// The origin of a toplevel's xdg window geometry in its own surface, which is where a popup's position is
/// measured from.
fn geometry_origin(surface: &WlSurface) -> Point<i32, Logical> {
    with_states(surface, |states| {
        states
            .cached_state
            .get::<SurfaceCachedState>()
            .current()
            .geometry
            .map(|geometry| geometry.loc)
            .unwrap_or_default()
    })
}

impl State {
    /// The popup surface under a global position, topmost first, with its origin in global coordinates, in the
    /// shape [`State::surface_under`] answers.
    ///
    /// Windows are walked from the top of the stack down to `ceiling`, the window the scene picked under the
    /// point (every window when none is). A window above the picked one that does not hold the point still
    /// answers for its popups, which hang past its edge; one beneath it never does.
    pub(crate) fn popup_under(
        &self,
        position: Point<f64, Logical>,
        ceiling: Option<WindowId>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        for window in self.composer.window_stack() {
            if self.window_is_pickable(window)
                && let Some(hit) = self.window_popup_under(window, position)
            {
                return Some(hit);
            }
            if Some(window) == ceiling {
                break;
            }
        }
        None
    }

    /// The popup of one window under a global position, if any.
    fn window_popup_under(
        &self,
        window: WindowId,
        position: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        let root = self.windows.get(&window)?.surface.wl_surface()?;
        let origin = self.composer.window_frame(window)?;
        let origin = Point::<i32, Logical>::from((origin.x, origin.y));
        let inside = Point::<f64, Logical>::from((position.x - f64::from(origin.x), position.y - f64::from(origin.y)));
        let geometry = geometry_origin(&root);
        for (popup, _) in PopupManager::popups_for_surface(&root) {
            let PopupKind::Xdg(xdg) = &popup else {
                continue;
            };
            let at = geometry + location_in_root(xdg) - popup.geometry().loc;
            if let Some((surface, offset)) =
                under_from_surface_tree(popup.wl_surface(), inside, at, WindowSurfaceType::ALL)
            {
                let global =
                    Point::<f64, Logical>::from((f64::from(origin.x + offset.x), f64::from(origin.y + offset.y)));
                return Some((surface, global));
            }
        }
        None
    }

    /// The window the seat's keyboard focus belongs to: the focused surface's own window, or, for a popup that
    /// holds the keyboard under a grab, the window the popup hangs from.
    pub(crate) fn keyboard_focus_window(&self) -> Option<WindowId> {
        let surface = self.keyboard.current_focus()?;
        let root = self
            .popups
            .find_popup(&surface)
            .and_then(|popup| find_popup_root_surface(&popup).ok())
            .unwrap_or(surface);
        self.window_of(&root)
    }

    /// A window moved or changed size: its reactive popups are placed again against the new geometry, and the
    /// client is told where they went. A popup that did not ask to be reactive stays where it was put.
    pub(crate) fn reposition_reactive_popups(&mut self, window: WindowId) {
        let Some(root) = self.windows.get(&window).and_then(|entry| entry.surface.wl_surface()) else {
            return;
        };
        for (popup, _) in PopupManager::popups_for_surface(&root) {
            let PopupKind::Xdg(xdg) = popup else {
                continue;
            };
            if !xdg.is_initial_configure_sent() || !xdg.with_pending_state(|state| state.positioner.reactive) {
                continue;
            }
            self.unconstrain_popup(&xdg);
            if let Err(error) = xdg.send_pending_configure() {
                log::debug!(area = "xdg", ?error, "a reactive popup could not be moved");
            }
        }
    }

    /// An `xdg_popup.grab`: the popup takes the keyboard and the pointer follows its client.
    ///
    /// The request is honoured when the serial is the one of an input event the client was given, which Smithay
    /// checks against the grab already held: a grab that does not nest on the topmost popup is refused with a
    /// protocol error on that client. A popup with no window or layer surface at the root of its chain has
    /// nothing to be dismissed with, so it is dismissed here.
    pub(crate) fn grab_popup(&mut self, surface: PopupSurface, seat: &WlSeat, serial: Serial) {
        let kind = PopupKind::Xdg(surface.clone());
        let Ok(root) = find_popup_root_surface(&kind) else {
            surface.send_popup_done();
            return;
        };
        if self.window_of(&root).is_none() && self.layer_rect(&root).is_none() {
            // Nothing will ever send this popup `popup_done`, because it has no parent to be dismissed with, so it
            // is dismissed here. Without this the popup stays on screen for the life of the session.
            surface.send_popup_done();
            log::debug!(area = "xdg", "a popup grab with no window at its root was dismissed");
            return;
        }
        let Some(seat) = Seat::<State>::from_resource(seat) else {
            surface.send_popup_done();
            return;
        };
        let mut grab = match self.popups.grab_popup(root.clone(), kind, &seat, serial) {
            Ok(grab) => grab,
            Err(error) => {
                log::debug!(area = "xdg", %error, "a popup grab was refused");
                return;
            }
        };
        if let Some(keyboard) = seat.get_keyboard() {
            if keyboard.is_grabbed()
                && !(keyboard.has_grab(serial) || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
            {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            keyboard.set_focus(self, grab.current_grab(), serial);
            keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
        }
        if let Some(pointer) = seat.get_pointer() {
            if pointer.is_grabbed()
                && !(pointer.has_grab(serial) || pointer.has_grab(grab.previous_serial().unwrap_or(serial)))
            {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
        }
        self.popup_grabs.retain(|held| held.root != root);
        self.popup_grabs.push(HeldGrab { root, grab });
    }

    /// Ends every popup grab that does not belong to the window about to take the keyboard, dismissing its
    /// popups, topmost first. `keeping` is the surface keyboard focus is going to: the window a grab hangs from
    /// keeps it.
    pub(crate) fn dismiss_popup_grabs_for_focus(&mut self, keeping: Option<&WlSurface>) {
        for held in &mut self.popup_grabs {
            if Some(&held.root) != keeping && !held.grab.has_ended() {
                held.grab.ungrab(PopupUngrabStrategy::All);
            }
        }
        self.popup_grabs.retain(|held| !held.grab.has_ended());
    }

    /// A popup went away: the manager forgets it, a grab that held only it is over, the keyboard goes back to the
    /// window the grab hung from instead of staying on a surface that no longer exists, and a pointer that was over
    /// the popup is moved on to whatever is under it now, so the client hears `leave` while the surface is still
    /// there to name.
    pub(crate) fn popup_gone(&mut self, popup: &PopupSurface) {
        self.popups.cleanup();
        let (ended, held): (Vec<HeldGrab>, Vec<HeldGrab>) = std::mem::take(&mut self.popup_grabs)
            .into_iter()
            .partition(|held| held.grab.has_ended());
        self.popup_grabs = held;
        for held in ended {
            let serial = SERIAL_COUNTER.next_serial();
            let keyboard = self.keyboard.clone();
            let previous = held.grab.previous_serial().unwrap_or(held.grab.serial());
            if keyboard.is_grabbed() && (keyboard.has_grab(held.grab.serial()) || keyboard.has_grab(previous)) {
                keyboard.unset_grab(self);
                if held.root.alive() {
                    keyboard.set_focus(self, Some(held.root.clone()), serial);
                }
            }
            let pointer = self.pointer.clone();
            if pointer.is_grabbed() && pointer.has_grab(held.grab.serial()) {
                pointer.unset_grab(self, serial, InputTime::now());
            }
        }
        if let Some(mut focus) = self.pointer.current_focus() {
            while &focus != popup.wl_surface() {
                let Some(parent) = get_parent(&focus) else {
                    self.damage_outputs();
                    return;
                };
                focus = parent;
            }
            let position = self.pointer_position;
            let under = self.surface_under(position);
            let serial = SERIAL_COUNTER.next_serial();
            let pointer = self.pointer.clone();
            pointer.motion(
                self,
                under,
                &MotionEvent {
                    location: position,
                    serial,
                    time: InputTime::now(),
                },
            );
            pointer.frame(self);
        }
        self.damage_outputs();
    }
}
