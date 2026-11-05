//! XML protocol `pointer_constraints_unstable_v1` (`<protocol name="pointer_constraints_unstable_v1">`); interfaces served: `zwp_pointer_constraints_v1`.
//!
//! `zwp_pointer_constraints_v1`: pointer lock and pointer confinement.
//!
//! The protocol lets a client ask for one of two things from the pointer while its
//! surface has focus: to stop moving (a lock, which a game uses to look around by
//! relative motion) or to stay inside a rectangle of the surface (a confine, which a
//! remote-desktop viewer uses so the client's cursor cannot escape its window). Both
//! are *constraints on the compositor's own state*, not on the client, so the whole of
//! the decision is here and in [`crate::State`]'s motion path.
//!
//! ## Lifetimes
//!
//! A constraint asked for with `oneshot` lives until it is deactivated, and one asked
//! for with `persistent` survives a loss of focus so it can activate again when the
//! pointer returns. Smithay removes the constraint object itself on a `oneshot`
//! deactivation and keeps the persistent one, and it already tells the client with
//! `unlocked`/`unconfined`; what this module adds is the record of which surface holds
//! a constraint, so the compositor can release them all for reasons of its own — the
//! session being covered is the one this slice acts on.
//!
//! ## When the compositor activates one
//!
//! A constraint is activated when the surface already has the pointer's focus and the
//! session is not covered. Activating a lock for a surface the pointer is not on would
//! freeze the pointer in a window the user is not using, which is exactly the kind of
//! stuck session a lock must never produce.
//!
//! # Failure modes
//!
//! 1. **A pointer lock survives a focus change.** The pointer leaving the surface
//!    deactivates the constraint, and the compositor drops its record in
//!    `remove_constraint`, so the next motion on another window moves normally:
//!    `a_pointer_lock_is_released_when_focus_moves` in `tests/input.rs`.
//! 2. **A pointer lock survives the session lock.** Engaging the lock releases every
//!    held constraint before the cover is drawn, so a covered session has no client
//!    holding the pointer: `a_session_lock_releases_a_pointer_lock`.
//! 3. **A lock freezes a pointer the user is not pointing with.** A constraint created
//!    while the surface has no pointer focus is not activated; the client hears
//!    `unlocked` only when the lock later activates and is released, exactly as the
//!    protocol describes.
//! 4. **A confined pointer escapes its region.** Motion under a confine is clamped into
//!    the region's bounding box, and a motion that would leave it is held at the edge:
//!    `a_confined_pointer_stays_inside_its_region`.

use smithay::input::pointer::PointerHandle;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::pointer_constraints::{
    ConstraintRemove, PointerConstraintRef, PointerConstraintsHandler, PointerConstraintsState, with_pointer_constraint,
};
use tracing::debug;

use crate::State;

/// Creates the `zwp_pointer_constraints_v1` global.
pub(crate) fn global(display: &DisplayHandle) -> PointerConstraintsState {
    PointerConstraintsState::new::<State>(display)
}

/// What a client asked the pointer to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ZwpPointerConstraintsV1Kind {
    /// The pointer stays where it is and reports relative motion.
    Locked,
    /// The pointer stays inside a region of the surface.
    Confined,
}

/// One constraint the compositor holds for a surface.
#[derive(Clone, Debug)]
pub(crate) struct ZwpPointerConstraintsV1HeldConstraint {
    /// The surface the client asked for.
    pub(crate) surface: WlSurface,
    /// The pointer the constraint applies to.
    pub(crate) pointer: PointerHandle<State>,
}

/// The constraints the session holds.
///
/// It is a list rather than a map because at most a few surfaces hold one at a time
/// and the list is read on every motion: a vector scan of two entries is cheaper than a
/// hash, and the order is the order the clients asked, which is what the release order
/// reports.
#[derive(Default, Debug)]
pub(crate) struct ZwpPointerConstraintsV1Tables {
    /// Every constraint that is currently held.
    pub(crate) held: Vec<ZwpPointerConstraintsV1HeldConstraint>,
    /// Where the client says it draws the cursor while locked, per pointer.
    pub(crate) hints: Vec<(WlSurface, Point<f64, Logical>)>,
}

impl State {
    /// The surface whose client holds an active pointer lock, if any.
    ///
    /// The answer is read per motion from Smithay's own record, so a lock the client
    /// destroyed or the pointer left cannot be seen as held.
    #[must_use]
    pub(crate) fn locked_surface(&self) -> Option<WlSurface> {
        let focus = self.pointer.current_focus()?;
        (self.constraint_of(&focus) == Some(ZwpPointerConstraintsV1Kind::Locked)).then_some(focus)
    }

    /// Holds a position inside the region of the surface under it, if that surface's client
    /// confined the pointer.
    ///
    /// The region is surface-local and the position is global, so the surface's origin is
    /// what turns one into the other. A position outside every surface is returned as it
    /// came: there is no constraint to honour where there is no surface.
    #[must_use]
    pub(crate) fn confine(&self, position: Point<f64, Logical>) -> Point<f64, Logical> {
        let Some((surface, origin)) = self.surface_under(position) else {
            return position;
        };
        if self.constraint_of(&surface) != Some(ZwpPointerConstraintsV1Kind::Confined) {
            return position;
        }
        let Some(region) = self.constraint_region(&surface) else {
            return position;
        };
        let local = position - origin;
        let left = f64::from(region.loc.x);
        let top = f64::from(region.loc.y);
        // The right and bottom edges are exclusive, so the clamped position stays strictly
        // inside the region the way a rectangle's own `contains` defines it.
        let right = f64::from(region.loc.x + region.size.w);
        let bottom = f64::from(region.loc.y + region.size.h);
        let clamped = Point::from((local.x.clamp(left, right), local.y.clamp(top, bottom)));
        origin + clamped
    }

    /// The constraint a surface holds for the session's pointer, if any.
    ///
    /// The kind comes from Smithay's own record of the constraint, so a constraint the
    /// client destroyed cannot be seen as held here: the two cannot disagree.
    pub(crate) fn constraint_of(&self, surface: &WlSurface) -> Option<ZwpPointerConstraintsV1Kind> {
        let pointer = self.pointer.clone();
        with_pointer_constraint(surface, &pointer, |constraint| {
            constraint.map(|constraint: PointerConstraintRef<'_, State>| {
                if matches!(
                    &*constraint,
                    smithay::wayland::pointer_constraints::PointerConstraint::Locked(_)
                ) {
                    ZwpPointerConstraintsV1Kind::Locked
                } else {
                    ZwpPointerConstraintsV1Kind::Confined
                }
            })
        })
    }

    /// The region a constraint confines the pointer to, in surface-local logical
    /// coordinates, as the bounding box of the region's added rectangles.
    ///
    /// A region with subtractions is treated as its bounding box: the protocol's own
    /// confinement is "stay inside the region", and a pointer held inside a bounding
    /// box never escapes the region itself. Refusing the subtraction is the safe
    /// direction of that approximation.
    pub(crate) fn constraint_region(&self, surface: &WlSurface) -> Option<smithay::utils::Rectangle<i32, Logical>> {
        let pointer = self.pointer.clone();
        with_pointer_constraint(surface, &pointer, |constraint| {
            let region = constraint?.region()?.clone();
            let mut bounds: Option<smithay::utils::Rectangle<i32, Logical>> = None;
            for (kind, rect) in &region.rects {
                if !matches!(kind, smithay::wayland::compositor::RectangleKind::Add) {
                    continue;
                }
                bounds = Some(match bounds {
                    Some(current) => current.merge(*rect),
                    None => *rect,
                });
            }
            bounds
        })
    }

    /// Releases every held constraint, telling each client its pointer is free.
    ///
    /// The session lock calls this before the cover is drawn: a covered client must not
    /// be able to keep the pointer captured, and a persistent constrain that survived
    /// the cover would resume on the window underneath it when the user unlocked.
    pub(crate) fn release_constraints(&mut self) {
        let held = std::mem::take(&mut self.constraint_tables.held);
        for entry in held {
            with_pointer_constraint(&entry.surface, &entry.pointer, |constraint| {
                if let Some(constraint) = constraint {
                    debug!(
                        surface = ?entry.surface.id(),
                        "[pointer-constraints] released while the session is covered"
                    );
                    constraint.deactivate();
                }
            });
        }
        self.constraint_tables.hints.clear();
    }
}

impl PointerConstraintsHandler for State {
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        // A covered session has no client surface under the pointer, so nothing may be
        // locked or confined: the constraint is held by the client and answered when
        // the session is revealed and the pointer arrives.
        if self.lock.state() != gnoblin_lock::State::Unlocked {
            return;
        }
        if !self.pointer_is_over(surface) {
            return;
        }
        with_pointer_constraint(surface, pointer, |constraint| {
            if let Some(constraint) = constraint {
                constraint.activate();
            }
        });
        self.constraint_tables.held.push(ZwpPointerConstraintsV1HeldConstraint {
            surface: surface.clone(),
            pointer: pointer.clone(),
        });
        debug!(surface = ?surface.id(), "[pointer-constraints] activated");
    }

    fn remove_constraint(&mut self, surface: &WlSurface, _pointer: &PointerHandle<Self>, _reason: ConstraintRemove) {
        self.constraint_tables.held.retain(|entry| entry.surface != *surface);
        self.constraint_tables.hints.retain(|(held, _)| held != surface);
    }

    fn cursor_position_hint(
        &mut self,
        surface: &WlSurface,
        _pointer: &PointerHandle<Self>,
        location: Point<f64, Logical>,
    ) {
        self.constraint_tables.hints.retain(|(held, _)| held != surface);
        self.constraint_tables.hints.push((surface.clone(), location));
    }
}
