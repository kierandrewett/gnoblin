//! Where a subsurface is: a position is part of its parent's state, applied when the parent's commit is applied.
//!
//! `wl_subsurface.set_position` says the next `wl_surface.commit` of the *parent* applies the position, whether the
//! subsurface is synchronized or not, and a parent that is itself a synchronized subsurface applies it only when
//! its own parent's commit applies that. Smithay keeps the position in the subsurface's own double-buffered state,
//! so it lands with the subsurface's commit, and that is wrong twice over: a desynchronized subsurface moves on its
//! own commit, and a subsurface that never commits never moves when its parent does.
//!
//! The request is therefore kept here ([`request`]) and never reaches Smithay's handler, and the position travels
//! the way the protocol says it does:
//!
//! 1. `set_position` puts the position in the parent's [`AskedPositions`] list, which is not double-buffered state.
//! 2. The parent's own `wl_surface.commit` moves the list into the parent's pending [`Placements`] before Smithay
//!    snapshots it (a pre-commit hook, [`stage`]). Smithay's commit of a tree with synchronized subsurfaces also
//!    snapshots the pending state of every synchronized surface below it, committed by its client or not, so a
//!    position kept in the pending state from the request would be applied by a commit of the grandparent. Only
//!    the hook, which runs for the surface the client committed, may move it there.
//! 3. The snapshot is cached while the parent is synchronized and applied with the commit that applies the parent,
//!    which is Smithay's own rule for every other piece of state ([`applied`], called from the commit handler).
//! 4. Applying writes the position into the subsurface's current state, where the surface tree and the hit test
//!    read it, and remembers it in [`AppliedPosition`]. The subsurface's own cached state is applied before or
//!    after the parent's in a transaction, depending on how the commits interleaved, and carries no position of
//!    its own any more, so each of its applications puts the remembered one back.
//!
//! # Failure modes
//!
//! 1. **A desynchronized subsurface moves with its own commit.** Tests: the wlcs `SubsurfaceTest` cases
//!    `desync_subsurface_moves_when_only_parent_committed` and `subsurface_does_not_move_when_parent_not_committed`,
//!    and `tests/surface_tree.rs`.
//! 2. **A subsurface moves when only its grandparent commits.** Tests: the wlcs `SubsurfaceMultilevelTest`
//!    `*_does_not_move_when_only_grandparent_committed` and `..._grandparent_commit_is_before_sync_parent_commit`.
//! 3. **A recreated subsurface starts where its predecessor ended.** The position is forgotten when the
//!    `wl_subsurface` is destroyed, as Smithay forgets its own copy.

use std::mem;
use std::sync::{Mutex, MutexGuard, PoisonError};

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{DisplayHandle, Resource, Weak};
use smithay::utils::{Logical, Point};
use smithay::wayland::compositor::{
    Cacheable, SubsurfaceCachedState, SurfaceData, add_pre_commit_hook, get_parent, with_states,
};

use crate::State;

/// A subsurface and the position its parent was asked to give it.
type Placement = (Weak<WlSurface>, Point<i32, Logical>);

/// The positions asked of a parent's subsurfaces since its last commit. It is data of the parent surface, not
/// double-buffered state: see the module docs for why that matters.
#[derive(Default)]
struct AskedPositions(Mutex<Vec<Placement>>);

/// The parent's double-buffered share of its subsurfaces' positions: what its commits carry to the commit that
/// applies them.
#[derive(Default)]
struct Placements(Vec<Placement>);

/// The position last applied to a subsurface, data of the subsurface itself.
#[derive(Default)]
struct AppliedPosition(Mutex<Option<Point<i32, Logical>>>);

impl Cacheable for Placements {
    fn commit(&mut self, _dh: &DisplayHandle) -> Self {
        // Taken, so a commit that Smithay makes for a synchronized surface its client did not commit carries nothing.
        Self(mem::take(&mut self.0))
    }

    fn merge_into(self, into: &mut Self, _dh: &DisplayHandle) {
        for placement in self.0 {
            upsert(&mut into.0, placement);
        }
    }
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A guard that panicked held a list of plain values, which stays usable.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Sets the position of `subsurface` in `list`, replacing an earlier one.
fn upsert(list: &mut Vec<Placement>, (subsurface, at): Placement) {
    let id = subsurface.id();
    match list.iter_mut().find(|(known, _)| known.id() == id) {
        Some(entry) => entry.1 = at,
        None => list.push((subsurface, at)),
    }
}

/// Keeps `at` as the position of `subsurface` for its parent's next commit.
pub(crate) fn request(subsurface: &WlSurface, at: Point<i32, Logical>) {
    let Some(parent) = get_parent(subsurface) else {
        // A wl_subsurface exists only while its surface has a parent, so this is a request after the parent went.
        return;
    };
    let first = with_states(&parent, |states| {
        let first = states.data_map.insert_if_missing_threadsafe(AskedPositions::default);
        if let Some(requested) = states.data_map.get::<AskedPositions>() {
            upsert(&mut locked(&requested.0), (subsurface.downgrade(), at));
        }
        first
    });
    if first {
        add_pre_commit_hook::<State, _>(&parent, |_, _, parent| stage(parent));
    }
}

/// Moves the positions asked of `parent` into the state its commit is about to snapshot.
fn stage(parent: &WlSurface) {
    with_states(parent, |states| {
        let asked = match states.data_map.get::<AskedPositions>() {
            Some(requested) => mem::take(&mut *locked(&requested.0)),
            None => return,
        };
        if asked.is_empty() {
            return;
        }
        let mut cached = states.cached_state.get::<Placements>();
        for placement in asked {
            upsert(&mut cached.pending().0, placement);
        }
    });
}

/// A `wl_subsurface` request to forget what this module holds for `subsurface`.
pub(crate) fn forget(subsurface: &WlSurface) {
    if let Some(parent) = get_parent(subsurface) {
        let id = subsurface.id();
        with_states(&parent, |states| {
            if let Some(requested) = states.data_map.get::<AskedPositions>() {
                locked(&requested.0).retain(|(known, _)| known.id() != id);
            }
        });
    }
    with_states(subsurface, |states| {
        if let Some(held) = states.data_map.get::<AppliedPosition>() {
            *locked(&held.0) = None;
        }
    });
}

/// Called when `surface`'s state has just been applied, which is where a position becomes the subsurface's own.
pub(crate) fn applied(surface: &WlSurface) {
    let moved = with_states(surface, |states| {
        // A surface that was never anyone's parent has no list and no cache to open.
        states.data_map.get::<AskedPositions>()?;
        Some(mem::take(&mut states.cached_state.get::<Placements>().current().0))
    });
    for (subsurface, at) in moved.into_iter().flatten() {
        if let Ok(subsurface) = subsurface.upgrade() {
            hold(&subsurface, at);
        }
    }
    // The surface's own state was applied first, and with it the location Smithay no longer keeps.
    with_states(surface, |states| {
        if let Some(at) = held(states) {
            states.cached_state.get::<SubsurfaceCachedState>().current().location = at;
        }
    });
}

fn held(states: &SurfaceData) -> Option<Point<i32, Logical>> {
    states
        .data_map
        .get::<AppliedPosition>()
        .and_then(|held| *locked(&held.0))
}

fn hold(subsurface: &WlSurface, at: Point<i32, Logical>) {
    with_states(subsurface, |states| {
        states.data_map.insert_if_missing_threadsafe(AppliedPosition::default);
        if let Some(held) = states.data_map.get::<AppliedPosition>() {
            *locked(&held.0) = Some(at);
        }
        states.cached_state.get::<SubsurfaceCachedState>().current().location = at;
    });
}
