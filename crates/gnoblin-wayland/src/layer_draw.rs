//! What the session does with a layer surface that is on the screen: where the pointer finds it, which of its clients is
//! answered a frame, and who gets the keyboard when the user clicks it.
//!
//! The protocol side (the rectangle, the configure, the exclusive zone, the exclusive keyboard) is
//! `layer_shell.rs`, and the pixels are the composer's (`gnoblin-backend`, `render/compose/layers.rs`).
//! This module is the part of the protocol that is neither: what a layer surface is to the seat.
//!
//! # Where the pointer finds a layer surface
//!
//! The pointer finds what is drawn, so a lookup follows the frame's order, topmost first:
//!
//! 1. the popups of every layer surface, which are above everything,
//! 2. the surfaces of the `overlay` and `top` layers, above the windows and the shell's own nodes,
//! 3. the shell's nodes and the windows (`State::surface_under`),
//! 4. the surfaces of the `bottom` and `background` layers.
//!
//! A surface answers only where its input region says so, so a bar's transparent corner lets the pointer through to the
//! window beneath it. While a layer surface is under the pointer nothing under it hears the pointer: not a window and not a
//! node of the shell's.
//!
//! # Who has the keyboard
//!
//! An exclusive surface on the `top` or `overlay` layer holds the keyboard from the moment it is drawn (the protocol
//! module). A surface that asked for focus on demand, and an exclusive one on a layer that grants none, takes it when the
//! user clicks it, and keeps it until the user clicks something else, a window takes the focus, or the surface goes. A
//! surface that asked for no keyboard never has it, whatever is clicked.
//!
//! # Failure modes
//!
//! 1. **A window under a bar hears a click on the bar.** A press on a layer surface is the layer surface's alone:
//!    `the_pointer_reaches_a_layer_surface_where_it_is_drawn_and_not_the_window_beneath`.
//! 2. **A click on a window leaves the keyboard on a bar.** A press anywhere but on the surface that holds the keyboard on
//!    demand gives the keyboard back to the window: `a_click_gives_the_keyboard_to_a_surface_that_asked_for_it_on_demand`.
//! 3. **A hidden surface answers a frame.** A surface the shell hid is not shown, so its client is left waiting for the
//!    callback: `a_layer_surface_is_answered_its_frame_callbacks`.

use smithay::desktop::utils::under_from_surface_tree;
use smithay::desktop::{PopupKind, PopupManager, WindowSurfaceType};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};

use gnoblin_backend::render::compose::TreeOwner;
use gnoblin_lock::State as LockState;
use gnoblin_types::PointF;

use crate::State;
use crate::popup::popup_origin;

/// Which layer surfaces a lookup considers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayerPick {
    /// The popups of every layer surface: they are above everything.
    Popups,
    /// The surfaces on the `top` and `overlay` layers, without their popups: they are above the windows.
    Above,
    /// The surfaces on the `bottom` and `background` layers, without their popups: they are below the windows.
    Below,
}

impl State {
    /// The surface of a layer surface's tree under a global position, topmost first, with the surface's own origin in global
    /// logical coordinates: the shape [`State::surface_under`] answers. Only a surface the user can see answers, and only
    /// while the session is unlocked: the lock is the only thing a locked session shows.
    pub(crate) fn layer_surface_under(
        &self,
        position: Point<f64, Logical>,
        pick: LayerPick,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        if self.lock.state() != LockState::Unlocked {
            return None;
        }
        for node in self.composer.layer_stack().into_iter().rev() {
            let wanted = match (pick, self.composer.layer_band(node)) {
                (LayerPick::Popups, Some(_)) => true,
                (LayerPick::Above, Some(band)) => band.above_windows(),
                (LayerPick::Below, Some(band)) => !band.above_windows(),
                (_, None) => false,
            };
            if !wanted {
                continue;
            }
            let Some(entry) = self.layers.entry_of_node(node).filter(|entry| entry.mapped) else {
                continue;
            };
            let Some(local) = self
                .composer
                .layer_local_point(node, PointF::new(position.x, position.y))
            else {
                continue;
            };
            let owner = TreeOwner::Layer(node);
            let root = entry.surface.wl_surface();
            let popups = pick == LayerPick::Popups;
            let found = if self.owner_has_surface_overrides(owner) {
                // A tree with a surface the shell placed, restacked or hid is picked from the order the scene draws it in.
                self.shell_surface_under(owner, position, popups)
            } else {
                Self::layer_tree_under(root, local, position, popups)
            };
            if found.is_some() {
                return found;
            }
        }
        None
    }

    /// The surface of one layer surface's tree that holds the point `local` of the surface's own plane, and its origin on
    /// the output plane: `position` is where the pointer is there, and the surface starts that far from it.
    fn layer_tree_under(
        root: &WlSurface,
        local: PointF,
        position: Point<f64, Logical>,
        popups: bool,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        let inside = Point::<f64, Logical>::from((local.x, local.y));
        let (surface, offset) = if popups {
            PopupManager::popups_for_surface(root).find_map(|(popup, _)| {
                let PopupKind::Xdg(xdg) = &popup else {
                    return None;
                };
                let at = popup_origin(root, &popup, xdg);
                under_from_surface_tree(popup.wl_surface(), inside, at, WindowSurfaceType::ALL)
            })?
        } else {
            under_from_surface_tree(root, inside, (0, 0), WindowSurfaceType::ALL)?
        };
        let origin = Point::from((
            position.x - local.x + f64::from(offset.x),
            position.y - local.y + f64::from(offset.y),
        ));
        Some((surface, origin))
    }

    /// Whether a layer surface that is above the windows, or the popup of one, is under a position: nothing beneath it, no
    /// shell node and no window, is under the pointer there.
    pub(crate) fn layer_above_at(&self, position: Point<f64, Logical>) -> bool {
        self.layer_surface_under(position, LayerPick::Popups).is_some()
            || self.layer_surface_under(position, LayerPick::Above).is_some()
    }

    /// The root surface of the layer surface a surface belongs to, as its own surface, a subsurface of it or a popup of it:
    /// `None` for a surface that belongs to a window or to nothing.
    fn layer_root_of(&self, surface: &WlSurface) -> Option<WlSurface> {
        match self.tree_owner_of(surface)? {
            TreeOwner::Layer(node) => Some(self.layers.entry_of_node(node)?.surface.wl_surface().clone()),
            TreeOwner::Window(_) => None,
        }
    }

    /// Notes a press at `position`, before the windows and the shell hear it.
    ///
    /// A press that lands on a layer surface focuses it when it asked for focus on demand, unless an exclusive surface holds
    /// the keyboard: that one keeps it, as the protocol says. A press anywhere else gives the keyboard back, when a layer
    /// surface had it on demand: the window that was focused before gets it again. A press on a surface that asked for no
    /// keyboard changes nothing, so a bar's clock can be clicked without the typing going away.
    pub(crate) fn layer_press(&mut self, position: Point<f64, Logical>) {
        if self.lock.state() != LockState::Unlocked {
            return;
        }
        let pressed = self
            .surface_under(position)
            .and_then(|(surface, _)| self.layer_root_of(&surface));
        match pressed {
            Some(root) => {
                if self
                    .layers
                    .entry_of(&root)
                    .is_some_and(|entry| entry.takes_focus_on_click())
                {
                    self.layers.on_demand = Some(root);
                    self.refresh_layer_focus();
                }
            }
            None => {
                if self.layers.on_demand.take().is_some() {
                    self.refresh_layer_focus();
                }
            }
        }
    }

    /// Answers the frame callbacks of every layer surface the user can see. A client that paces its drawing by them gets one
    /// answer per frame, as a window does; a surface the shell hid is left waiting, because a client that cannot be seen
    /// must not draw a frame nobody will look at.
    pub(crate) fn answer_layer_frame_callbacks(&self, time: u32) {
        for entry in &self.layers.entries {
            let hidden = entry.node.is_none_or(|node| self.composer.layer_hidden(node));
            if entry.mapped && entry.output.is_some() && !hidden {
                Self::done_frame_callbacks(entry.surface.wl_surface(), time);
            }
        }
    }
}
