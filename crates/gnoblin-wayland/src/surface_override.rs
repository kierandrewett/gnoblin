//! What a shell that holds the tree may do with a popup, a subsurface or a layer surface: the protocol side of
//! `placement_override`, `stack` and `visible` on a `surface` node.
//!
//! The composer reads the shell's writes off the nodes and draws a subsurface where the shell put it, in the order the
//! shell gave and not at all when the shell hid it. This module is what only the protocol engine can do, in three parts:
//!
//! 1. **A popup is put where the shell asked.** The shell's place is the popup's *requested* place: the positioner's own
//!    constraint adjustment (flip, slide, resize) still runs on it against the monitor, and the place the compositor
//!    settled on is written back to the node. A *reactive* popup is configured there, so the client's idea of where it is
//!    matches what is drawn. `xdg_popup` allows no configure after the first for a popup that is not reactive, so such a
//!    popup is drawn and picked at the place without being told ([`ShellPlace`]), and its client's configure stays the
//!    protocol's. Each reposition the client asks for and each reactive reposition goes through the same code
//!    ([`State::unconstrain_popup`]), so the shell's place is not undone by them, and a popup is put back where the
//!    protocol says when the shell stops placing it, the session locks or the shell detaches.
//! 2. **The pointer finds what is drawn.** A window with a surface the shell placed or hid is picked from the order the
//!    scene draws its surfaces in, instead of Smithay's walk of the protocol's tree, so a moved, restacked or hidden
//!    subsurface or popup takes the pointer where it is and not where its client put it. Each surface's own input region
//!    still decides whether the point is over it.
//! 3. **A hidden popup holds nothing.** A popup the shell hid that holds a grab is dismissed, because a popup nobody can
//!    see must not keep the keyboard and the pointer. Nothing else about a popup's grab changes: it is still the
//!    client's, ended by a press elsewhere as the protocol says.
//!
//! A layer surface's part is in `layer_shell.rs`. Overrides apply only while a shell holds the tree and
//! the session is unlocked ([`State::surface_overrides_apply`]); the composer gives the placement back on detach.
//!
//! # Failure modes
//!
//! 1. **A shell's place is applied to a popup the compositor has not configured.** A popup whose initial configure has
//!    not been sent is left alone, so the first configure is the one `xdg_popup` mandates and not an early one.
//! 2. **A popup that may not be configured is configured.** A popup whose positioner is not reactive is never sent a
//!    configure for the shell's place, which would be a protocol error on the client side —
//!    `a_popup_that_is_not_reactive_is_moved_without_being_told_because_the_protocol_forbids_a_configure`.
//! 3. **The pointer reaches a hidden surface.** The pick skips it and everything under it.
//! 4. **A moved subsurface is unreachable outside its window.** The window's hit extent grows to cover it
//!    (`Composer::placed_tree_extent`).

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};

use smithay::backend::renderer::utils::RendererSurfaceStateUserData;
use smithay::desktop::find_popup_root_surface;
use smithay::desktop::utils::under_from_surface_tree;
use smithay::desktop::{PopupKind, PopupManager, WindowSurfaceType};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::compositor::{TraversalAction, with_states, with_surface_tree_downward};
use smithay::wayland::shell::xdg::PopupSurface;

use gnoblin_backend::render::compose::{TreeOwner, TreeSurfaceRole};
use gnoblin_lock::State as LockState;

use crate::State;
use crate::log;
use crate::surface_tree::key_of;

/// The popups the shell placed, so one that stops being placed is put back where the protocol says.
#[derive(Debug, Default)]
pub(crate) struct SurfaceOverrides {
    /// The protocol id of each popup placed at the shell's place.
    popups: BTreeSet<u64>,
}

/// The place the shell put a popup at that its client was not told of, relative to its parent's window geometry, as a
/// popup's configured place is. It is data of the popup's surface, so the draw, the pick and the popups that hang from it
/// all read the same place ([`crate::popup`]).
#[derive(Default)]
struct ShellPlace(Mutex<Option<Point<i32, Logical>>>);

/// The place the shell put a popup at without telling its client, if it did.
pub(crate) fn shell_place(surface: &WlSurface) -> Option<Point<i32, Logical>> {
    with_states(surface, |states| {
        states
            .data_map
            .get::<ShellPlace>()
            .and_then(|place| *place.0.lock().unwrap_or_else(PoisonError::into_inner))
    })
}

/// Keeps `place` as the place the shell put a popup at, or forgets it. Whether it changed.
fn set_shell_place(surface: &WlSurface, place: Option<Point<i32, Logical>>) -> bool {
    with_states(surface, |states| {
        states.data_map.insert_if_missing_threadsafe(ShellPlace::default);
        let Some(held) = states.data_map.get::<ShellPlace>() else {
            return false;
        };
        let mut held = held.0.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::replace(&mut *held, place) != place
    })
}

impl State {
    /// Whether the shell's overrides count now: a shell holds the tree and the session is unlocked. A locked session shows
    /// no window, and what the compositor decides under the lock is not the shell's to change.
    pub(crate) fn surface_overrides_apply(&self) -> bool {
        self.composer.holder_attached() && self.lock.state() == LockState::Unlocked
    }

    /// Follows what the shell wrote on the surface nodes since the last frame, after the frame's transactions applied.
    ///
    /// The popups go first, because they read the places the shell *asked* for off the nodes, and the composer writes the
    /// place a popup is at over a node when it builds the nodes again. The windows whose trees changed are then built
    /// again at once, so a popup that was just configured is drawn at the place it settled on in this frame.
    pub(crate) fn follow_surface_overrides(&mut self) {
        self.follow_popup_overrides();
        self.follow_layer_overrides();
        for owner in self.composer.follow_surface_shell() {
            self.mark_surface_tree_dirty(owner);
        }
        self.flush_surface_trees();
    }

    /// The place, relative to its parent's window geometry, the shell asked a popup to be configured at; `None` when the
    /// shell asked for none, or the overrides do not apply.
    ///
    /// The shell writes the popup node's `x` and `y` in its parent's plane, which is where the popup's *surface* starts. A
    /// popup's configured place is where its *window geometry* starts, so the geometry's own offset in the popup's surface
    /// is added, and the offset of the parent popup's geometry is taken off for a popup that hangs from another.
    pub(crate) fn popup_override_place(&self, popup: &PopupSurface) -> Option<Point<i32, Logical>> {
        if !self.surface_overrides_apply() {
            return None;
        }
        let kind = PopupKind::Xdg(popup.clone());
        let root = find_popup_root_surface(&kind).ok()?;
        let owner = self.root_owner(&root)?;
        let key = key_of(popup.wl_surface());
        let (x, y) = self
            .composer
            .surface_shell(owner)
            .into_iter()
            .find(|surface| surface.key == key && surface.role == TreeSurfaceRole::Popup)?
            .placed?;
        let own = kind.geometry().loc;
        let parent = popup.get_parent_surface()?;
        let parent_plane = if parent == root {
            Point::default()
        } else {
            self.popups
                .find_popup(&parent)
                .map(|parent| parent.geometry().loc)
                .unwrap_or_default()
        };
        Some(Point::from((
            x.round() as i32 + own.x - parent_plane.x,
            y.round() as i32 + own.y - parent_plane.y,
        )))
    }

    /// Puts the popups the shell placed at the places it asked for, and the ones it stopped placing back; and dismisses the
    /// grab of a popup the shell hid.
    ///
    /// What the client is told depends on the popup's positioner, because `xdg_popup` allows a configure after the first
    /// only for a *reactive* popup (and as the answer to a reposition). A reactive popup is configured at the place, so
    /// the client's own idea of where it is matches the screen. A popup that is not reactive is drawn and picked at the
    /// place without being told, and its client's configure stays the protocol's: the compositor moves what it shows, as
    /// it does for a subsurface, and never sends the client an event it may not receive.
    fn follow_popup_overrides(&mut self) {
        let apply = self.surface_overrides_apply();
        let mut seen = BTreeSet::new();
        let mut hidden = BTreeSet::new();
        for (owner, root) in self.tree_roots() {
            let shell = self.composer.surface_shell(owner);
            if shell.is_empty() && self.surface_overrides.popups.is_empty() {
                continue;
            }
            for (popup, _) in PopupManager::popups_for_surface(&root) {
                let PopupKind::Xdg(xdg) = popup else {
                    continue;
                };
                let key = key_of(xdg.wl_surface());
                seen.insert(key);
                let node = shell
                    .iter()
                    .find(|surface| surface.key == key && surface.role == TreeSurfaceRole::Popup);
                if apply && node.is_some_and(|node| node.hidden) {
                    hidden.insert(key);
                }
                let placed = apply && node.is_some_and(|node| node.placed.is_some());
                if !placed && !self.surface_overrides.popups.contains(&key) {
                    continue;
                }
                // The initial configure is the compositor's own, sent when the popup is first committed.
                if !xdg.is_initial_configure_sent() {
                    continue;
                }
                let reactive = xdg.with_pending_state(|state| state.positioner.reactive);
                let changed = if placed {
                    self.surface_overrides.popups.insert(key);
                    self.place_popup(&xdg, reactive)
                } else {
                    self.surface_overrides.popups.remove(&key);
                    self.unplace_popup(&xdg, reactive)
                };
                if changed {
                    self.mark_surface_tree_dirty(owner);
                }
            }
        }
        self.surface_overrides.popups.retain(|key| seen.contains(key));
        self.dismiss_hidden_popup_grabs(&hidden);
    }

    /// Puts a popup at the place the shell asked for, after its positioner's adjustment. Whether anything changed.
    fn place_popup(&mut self, popup: &PopupSurface, reactive: bool) -> bool {
        let Some(geometry) = self.popup_geometry(popup) else {
            return false;
        };
        if !reactive {
            return set_shell_place(popup.wl_surface(), Some(geometry.loc));
        }
        // The client is told, so the place it knows is the one that is drawn.
        let cleared = set_shell_place(popup.wl_surface(), None);
        let before = popup.with_pending_state(|state| state.geometry);
        popup.with_pending_state(|state| state.geometry = geometry);
        if before == geometry {
            return cleared;
        }
        if let Err(error) = popup.send_pending_configure() {
            log::debug!(area = "xdg", ?error, "a placed popup could not be configured");
        }
        true
    }

    /// Puts a popup the shell stopped placing back where the protocol says. Whether anything changed.
    fn unplace_popup(&mut self, popup: &PopupSurface, reactive: bool) -> bool {
        let cleared = set_shell_place(popup.wl_surface(), None);
        if !reactive {
            return cleared;
        }
        let before = popup.with_pending_state(|state| state.geometry);
        self.unconstrain_popup(popup);
        let after = popup.with_pending_state(|state| state.geometry);
        if before == after {
            return cleared;
        }
        if let Err(error) = popup.send_pending_configure() {
            log::debug!(area = "xdg", ?error, "a popup could not be put back");
        }
        true
    }

    /// Whether the shell placed or hid a surface of this owner's tree, so its surfaces are picked from the scene.
    pub(crate) fn owner_has_surface_overrides(&self, owner: TreeOwner) -> bool {
        self.surface_overrides_apply() && !self.composer.surface_shell(owner).is_empty()
    }

    /// The surface of an owner's tree under a global position, found in the order the scene draws the owner's surfaces in. With
    /// `popups` it considers the popups and what hangs from them, otherwise everything else, the root surface among it. The
    /// answer is in the shape [`State::surface_under`] gives: the surface and its origin in global coordinates.
    pub(crate) fn shell_surface_under(
        &self,
        owner: TreeOwner,
        position: Point<f64, Logical>,
        popups: bool,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        let root = self.tree_root_surface(owner)?;
        let frame = match owner {
            TreeOwner::Window(window) => self.composer.window_frame(window)?,
            TreeOwner::Layer(node) => self.composer.layer_frame(node)?,
        };
        for candidate in self.composer.pick_order(owner) {
            if candidate.popup != popups {
                continue;
            }
            let surface = match candidate.key {
                None => root.clone(),
                Some(key) => match tree_surface(&root, key) {
                    Some(surface) => surface,
                    None => continue,
                },
            };
            let origin = Point::<i32, Logical>::from((frame.x + candidate.origin.0, frame.y + candidate.origin.1));
            // Smithay adds the surface's own view offset (a subsurface's place in its parent) to the location it is
            // given, so it is taken off here: the origin is where the scene draws the surface, not where it was put.
            let location = origin - view_offset(&surface);
            if let Some((surface, found)) =
                under_from_surface_tree(&surface, position, location, WindowSurfaceType::TOPLEVEL)
            {
                return Some((surface, found.to_f64()));
            }
        }
        None
    }
}

/// The offset Smithay's tree walk adds for a surface: its place in its parent when it is a subsurface, and nothing
/// otherwise.
fn view_offset(surface: &WlSurface) -> Point<i32, Logical> {
    with_states(surface, |states| {
        states
            .data_map
            .get::<RendererSurfaceStateUserData>()
            .and_then(|data| data.lock().ok()?.view())
            .map(|view| view.offset)
            .unwrap_or_default()
    })
}

/// The surface of a window's tree with this key: the window's own, a subsurface or a popup, or a subsurface of a popup.
fn tree_surface(root: &WlSurface, key: u64) -> Option<WlSurface> {
    let found = RefCell::new(None);
    let roots = std::iter::once(root.clone())
        .chain(PopupManager::popups_for_surface(root).map(|(popup, _)| popup.wl_surface().clone()));
    for surface in roots {
        with_surface_tree_downward(
            &surface,
            (),
            |_, _, ()| TraversalAction::DoChildren(()),
            |candidate, _, ()| {
                if key_of(candidate) == key {
                    *found.borrow_mut() = Some(candidate.clone());
                }
            },
            |_, _, ()| found.borrow().is_none(),
        );
        if found.borrow().is_some() {
            break;
        }
    }
    found.into_inner()
}
