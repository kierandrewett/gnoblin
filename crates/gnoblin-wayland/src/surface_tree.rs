//! The surface tree of a window: its subsurfaces and its popups, built for the composer to draw.
//!
//! A window is more than its toplevel surface. A `wl_subsurface` is part of the window it hangs from, at an offset
//! from its parent and above or below it as `place_above` and `place_below` set; an `xdg_popup` is a window of its
//! own that is placed by the compositor against its parent and drawn above it, above the subsurfaces of its parent
//! and over the edge of the window when it hangs past it. A client's menus, tooltips, combo boxes and its video
//! or GL widgets are all of these, and a compositor that draws only the toplevel surface shows none of them.
//!
//! The protocol engine knows the tree and the composer knows the scene, and `gnoblin-wayland` names neither the
//! scene nor its nodes (`gnoblin-backend`, `render/compose.rs`, is where they meet). This module is the one place
//! that reads the tree out of Smithay and hands it over as a flat list of [`TreeSurface`]: parents before their
//! children, the surfaces that share a parent in the order they paint, each with its offset in its parent and its
//! buffer. The composer makes a node of each and keeps them in step with the list.
//!
//! # When the list is built
//!
//! A change to a window's tree is a commit of one of its surfaces, a popup that goes, a surface that is destroyed or
//! a popup moved by the compositor. Each only marks the window, and the list is built once per window per frame,
//! before the composer's damage is taken, so a client that commits a video subsurface and a popup in one iteration
//! costs one build. The surfaces that committed are remembered until then, so the composer damages exactly them.
//!
//! # Failure modes
//!
//! 1. **A subsurface placed below its parent is drawn over it.** The parent's own pixels sit in the middle of its
//!    children in Smithay's order, and a child before them is `behind`:
//!    `a_subsurface_below_the_parent_is_drawn_behind_it` in `tests/surface_tree.rs`.
//! 2. **An unmapped subsurface keeps its children on screen.** A surface with no buffer is left out with everything
//!    under it, as the protocol says: `a_subsurface_without_a_buffer_hides_what_hangs_from_it`.
//! 3. **A popup of a popup is placed against the window.** Each popup's offset is taken from its parent surface, so
//!    a menu opened from a submenu follows it: `a_submenu_is_drawn_beside_its_menu`.
//! 4. **A destroyed popup stays drawn.** The composer's tree is rebuilt when a popup or surface goes, without a
//!    commit to say so: `a_destroyed_popup_leaves_the_screen`.

use std::collections::{BTreeMap, BTreeSet};

use smithay::desktop::{PopupKind, PopupManager, find_popup_root_surface};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::compositor::{
    SubsurfaceCachedState, TraversalAction, get_children, get_parent, is_sync_subsurface, with_surface_tree_upward,
};

use gnoblin_backend::render::compose::{TreeSurface, TreeSurfaceRole};
use gnoblin_render_frame::frame::SurfaceSource;
use gnoblin_types::WindowId;

use crate::State;
use crate::popup::popup_origin;
use crate::protocols::wp_alpha_modifier_v1::committed_factor;
use crate::state::committed_content;

/// The windows whose surface trees are to be built again before the next frame, and the surfaces that committed
/// since the last build.
#[derive(Debug, Default)]
pub(crate) struct SurfaceTrees {
    dirty: BTreeSet<WindowId>,
    committed: BTreeSet<u64>,
}

/// A surface's key among those of its window: its protocol id, which no two surfaces of one client share.
fn key_of(surface: &WlSurface) -> u64 {
    u64::from(surface.id().protocol_id())
}

/// Adds the key of every synchronized subsurface under `surface` to `keys`.
fn add_synchronized_keys(surface: &WlSurface, keys: &mut BTreeSet<u64>) {
    for child in get_children(surface) {
        if is_sync_subsurface(&child) {
            keys.insert(key_of(&child));
        }
        add_synchronized_keys(&child, keys);
    }
}

impl State {
    /// A surface committed: its window's tree is built again before the next frame, with this surface's pixels
    /// damaged.
    ///
    /// The commit of a surface that is not a synchronized subsurface applies, with its own, the commits the
    /// synchronized subsurfaces under it held back, so their pixels may have changed now: they are damaged too.
    pub(crate) fn surface_committed(&mut self, surface: &WlSurface) {
        self.surface_trees.committed.insert(key_of(surface));
        if !is_sync_subsurface(surface) {
            add_synchronized_keys(surface, &mut self.surface_trees.committed);
        }
        if let Some(window) = self.tree_window_of(surface) {
            self.surface_trees.dirty.insert(window);
        }
    }

    /// A window's tree changed in a way no commit says.
    pub(crate) fn mark_surface_tree_dirty(&mut self, window: WindowId) {
        self.surface_trees.dirty.insert(window);
    }

    /// A surface or a popup went, and which window it belonged to is no longer known.
    pub(crate) fn mark_all_surface_trees_dirty(&mut self) {
        self.surface_trees.dirty.extend(self.windows.keys().copied());
    }

    /// The window a surface belongs to: the window of its root through its subsurface parents, or, for a popup or a
    /// subsurface of one, the window its popup chain hangs from. `None` for a surface that is neither, such as a
    /// layer surface's.
    fn tree_window_of(&self, surface: &WlSurface) -> Option<WindowId> {
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        if let Some(window) = self.window_of(&root) {
            return Some(window);
        }
        let popup = self.popups.find_popup(&root)?;
        self.window_of(&find_popup_root_surface(&popup).ok()?)
    }

    /// Hands the composer the tree of every window marked since the last frame.
    pub(crate) fn flush_surface_trees(&mut self) {
        let trees = std::mem::take(&mut self.surface_trees);
        for window in trees.dirty {
            let Some(root) = self.windows.get(&window).and_then(|entry| entry.surface.wl_surface()) else {
                continue;
            };
            let surfaces = surfaces_of(&root, &trees.committed);
            self.composer.set_window_surfaces(window, surfaces);
        }
    }
}

/// The surfaces of the tree of the window whose toplevel surface is `root`, below that surface, for the composer.
///
/// A window that is not mapped has none. Its subsurfaces come first, each with the subsurfaces under it, then its
/// popups from the oldest to the newest, each with its own subsurfaces: a popup is above everything the window
/// shows, and a popup that hangs from another is above that one.
fn surfaces_of(root: &WlSurface, committed: &BTreeSet<u64>) -> Vec<TreeSurface> {
    let mut surfaces = Vec::new();
    if committed_content(root).is_none() {
        return surfaces;
    }
    push_subsurfaces(root, None, committed, &mut surfaces);
    // The manager lists the popups newest first, a popup's children before it: the reverse is the order they paint.
    let mut popups: Vec<_> = PopupManager::popups_for_surface(root).collect();
    popups.reverse();
    let mut origins: BTreeMap<u64, Point<i32, Logical>> = BTreeMap::new();
    for (popup, _) in &popups {
        let PopupKind::Xdg(xdg) = popup else {
            continue;
        };
        let surface = popup.wl_surface();
        let Some((buffer, content)) = committed_content(surface) else {
            continue;
        };
        // A popup is placed against the surface it hangs from, which is the window's own or another popup's. One
        // whose parent is not drawn is not drawn either.
        let (parent, parent_origin) = match xdg.get_parent_surface() {
            Some(parent) if &parent == root => (None, Point::from((0, 0))),
            Some(parent) => match origins.get(&key_of(&parent)) {
                Some(origin) => (Some(key_of(&parent)), *origin),
                None => continue,
            },
            None => continue,
        };
        let origin = popup_origin(root, popup, xdg);
        let key = key_of(surface);
        origins.insert(key, origin);
        surfaces.push(TreeSurface {
            key,
            parent,
            role: TreeSurfaceRole::Popup,
            behind: false,
            offset: gnoblin_types::Point::new(origin.x - parent_origin.x, origin.y - parent_origin.y),
            alpha: committed_factor(surface),
            content,
            source: SurfaceSource::of_buffer(buffer),
            damaged: committed.contains(&key),
        });
        push_subsurfaces(surface, Some(key), committed, &mut surfaces);
    }
    surfaces
}

/// Adds the mapped subsurfaces under `surface`, which the list names `parent`, each followed by the ones under it.
fn push_subsurfaces(surface: &WlSurface, parent: Option<u64>, committed: &BTreeSet<u64>, out: &mut Vec<TreeSurface>) {
    // Smithay keeps a surface's children bottom first with the surface itself among them, and a traversal visits them
    // in that order: a child visited before its parent is below it. The location is read here, while the traversal
    // holds the surface's data, because asking for it again from the surface would wait on the lock it holds.
    let mut painted: Vec<(WlSurface, Option<WlSurface>, Point<i32, Logical>)> = Vec::new();
    with_surface_tree_upward(
        surface,
        None::<WlSurface>,
        |surface, _, _| TraversalAction::DoChildren(Some(surface.clone())),
        |surface, states, parent| {
            let location = states.cached_state.get::<SubsurfaceCachedState>().current().location;
            painted.push((surface.clone(), parent.clone(), location));
        },
        |_, _, _| true,
    );
    push_children(&painted, surface, parent, committed, out);
}

/// Adds the children of `under` in `painted`, in paint order, each followed by its own.
fn push_children(
    painted: &[(WlSurface, Option<WlSurface>, Point<i32, Logical>)],
    under: &WlSurface,
    parent: Option<u64>,
    committed: &BTreeSet<u64>,
    out: &mut Vec<TreeSurface>,
) {
    let under_position = painted.iter().position(|(surface, _, _)| surface == under);
    for (position, (surface, _, location)) in painted
        .iter()
        .enumerate()
        .filter(|(_, (_, hangs_from, _))| hangs_from.as_ref() == Some(under))
    {
        // A surface with no buffer is not mapped, and neither is anything that hangs from it.
        let Some((buffer, content)) = committed_content(surface) else {
            continue;
        };
        let key = key_of(surface);
        out.push(TreeSurface {
            key,
            parent,
            role: TreeSurfaceRole::Subsurface,
            behind: under_position.is_some_and(|under| position < under),
            offset: gnoblin_types::Point::new(location.x, location.y),
            alpha: committed_factor(surface),
            content,
            source: SurfaceSource::of_buffer(buffer),
            damaged: committed.contains(&key),
        });
        push_children(painted, surface, Some(key), committed, out);
    }
}
