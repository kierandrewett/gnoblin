//! XML protocol `pointer_constraints_unstable_v1` (`<protocol name="pointer_constraints_unstable_v1">`); interfaces served: `zwp_pointer_constraints_v1`.
//!
//! `zwp_pointer_constraints_v1`: pointer lock and pointer confinement.
//!
//! The protocol lets a client ask for one of two things from the pointer while its
//! surface has focus: to stop moving (a lock, which a game uses to look around by
//! relative motion) or to stay inside a rectangle of the surface (a confine, which a
//! remote-desktop viewer uses so the client's cursor cannot escape its window). Both
//! are *constraints on the compositor's own state*, not on the client, so the whole of
//! the decision is here and in `crate::State`'s motion path.
//!
//! ## Lifetimes
//!
//! A constraint asked for with `oneshot` lives until it is deactivated, and one asked
//! for with `persistent` survives a loss of focus so it can activate again when the
//! pointer returns. Smithay removes the constraint object itself on a `oneshot`
//! deactivation and keeps the persistent one, and it already tells the client with
//! `unlocked`/`unconfined`; what this module adds is the record of which surface holds
//! an active constraint, so the compositor can release them all for reasons of its own
//! (the session being covered is the one this slice acts on) and decide when a waiting
//! constraint may start.
//!
//! ## When the compositor activates one
//!
//! A constraint is active while all of these hold, and `State::update_pointer_constraints`
//! settles it whenever one of them can have changed (a new constraint, the pointer's focus
//! moving, the keyboard's focus moving):
//!
//! - the session is not covered;
//! - the surface has the pointer's focus and the pointer is inside the constraint's region,
//!   which the protocol guarantees a client when it hears `locked` or `confined`;
//! - the surface's window has the keyboard. A lock that started when the pointer merely
//!   passed over a window would freeze it for a user who is typing into another one, which
//!   is exactly the kind of stuck session a lock must never produce, so the window has to
//!   have been chosen first.
//!
//! A constraint stops when any of them stops. A `oneshot` one is then gone; a `persistent`
//! one starts again when all three hold again.
//!
//! # Failure modes
//!
//! 1. **A pointer lock survives a focus change.** The pointer leaving the surface
//!    deactivates the constraint, and the compositor drops its record in
//!    `remove_constraint`, so the next motion on another window moves normally:
//!    `a_pointer_lock_is_released_when_focus_moves` in `tests/protocols.rs`.
//! 2. **A pointer lock survives the session lock.** Engaging the lock releases every
//!    held constraint before the cover is drawn, so a covered session has no client
//!    holding the pointer: `a_session_lock_releases_a_pointer_lock`.
//! 3. **A lock freezes a pointer the user is not pointing with.** A constraint created
//!    on a surface that does not have both the pointer and the keyboard is not activated;
//!    the client hears `locked` when the window is chosen and `unlocked` when it is not
//!    chosen any more, exactly as the protocol describes:
//!    `a_pointer_constraint_waits_until_its_window_has_the_keyboard`.
//! 4. **A confined pointer escapes its region.** Motion under a confine is clamped into
//!    the region's bounding box (the whole surface when the client gave none), and a motion
//!    that would leave it is held at the edge. A pointer that has just left the surface is
//!    still the confined surface's pointer: the region is read for the surface that holds
//!    the pointer, not for whatever lies under the position the motion asked for.
//!    `a_confined_pointer_stays_inside_its_surface` and `a_confined_pointer_stays_inside_its_region`.
//! 5. **A mouse is not constrained.** A mouse reports motion as a delta, and that path
//!    held no lock and no confinement and sent no relative motion at all, so a game
//!    played with a mouse had neither: `a_locked_pointer_reports_a_mouse_as_relative_motion`.

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};
use smithay::backend::renderer::utils::RendererSurfaceStateUserData;
use smithay::input::SeatHandler;
use smithay::input::pointer::PointerHandle;
use smithay::reexports::wayland_protocols::wp::pointer_constraints::zv1::server::{
    zwp_confined_pointer_v1::ZwpConfinedPointerV1, zwp_locked_pointer_v1::ZwpLockedPointerV1,
    zwp_pointer_constraints_v1::ZwpPointerConstraintsV1,
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Dispatch, GlobalDispatch, Resource};
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::GlobalData;
use smithay::wayland::compositor::{RectangleKind, get_parent, with_states};
use smithay::wayland::pointer_constraints::{
    PointerConstraint, PointerConstraintRef, PointerConstraintUserData, PointerConstraintsHandler,
    PointerConstraintsState, with_pointer_constraint,
};

gnoblin_log::logger!(log = "wayland");

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "zwp-pointer-constraints-v1",
    interfaces: &[ServedInterface::new("zwp_pointer_constraints_v1", 1)],
    privilege: Privilege::Core,
};

/// What the protocol needs of the compositor: the pointer and keyboard it follows, and where its tables live.
pub trait Host: SeatHandler<PointerFocus = WlSurface> + PointerConstraintsHandler + Sized + 'static {
    /// The session's pointer.
    fn pointer(&self) -> &PointerHandle<Self>;

    /// The surface that has the keyboard, if any.
    fn keyboard_focus(&self) -> Option<WlSurface>;

    /// The origin, in the layout, of the surface the pointer is on, if it is on one.
    fn pointer_origin(&self) -> Option<Point<f64, Logical>>;

    /// The pointer's place in the layout.
    fn pointer_position(&self) -> Point<f64, Logical>;

    /// Whether the session is unlocked: a covered session holds no constraint.
    fn session_unlocked(&self) -> bool;

    /// The constraints the session holds.
    fn constraint_tables(&mut self) -> &mut ZwpPointerConstraintsV1Tables<Self>;
}

/// Creates the `zwp_pointer_constraints_v1` global.
pub fn register<D>(registrar: &Registrar<'_>) -> PointerConstraintsState
where
    D: GlobalDispatch<ZwpPointerConstraintsV1, GlobalData>
        + Dispatch<ZwpPointerConstraintsV1, GlobalData>
        + Dispatch<ZwpLockedPointerV1, PointerConstraintUserData<D>>
        + Dispatch<ZwpConfinedPointerV1, PointerConstraintUserData<D>>
        + SeatHandler
        + 'static,
{
    PointerConstraintsState::new::<D>(registrar.display())
}

/// What a client asked the pointer to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZwpPointerConstraintsV1Kind {
    /// The pointer stays where it is and reports relative motion.
    Locked,
    /// The pointer stays inside a region of the surface.
    Confined,
}

/// One constraint the compositor holds active for a surface.
pub struct ZwpPointerConstraintsV1HeldConstraint<H: SeatHandler> {
    /// The surface the client asked for.
    pub surface: WlSurface,
    /// The pointer the constraint applies to.
    pub pointer: PointerHandle<H>,
}

/// The constraints the session holds.
///
/// It is a list rather than a map because at most a few surfaces hold one at a time
/// and the list is read on every motion: a vector scan of two entries is cheaper than a
/// hash, and the order is the order the clients asked, which is what the release order
/// reports.
pub struct ZwpPointerConstraintsV1Tables<H: SeatHandler> {
    /// Every constraint that is currently active.
    pub held: Vec<ZwpPointerConstraintsV1HeldConstraint<H>>,
    /// Where the client says it draws the cursor while locked, per pointer.
    pub hints: Vec<(WlSurface, Point<f64, Logical>)>,
}

impl<H: SeatHandler> Default for ZwpPointerConstraintsV1Tables<H> {
    fn default() -> Self {
        Self {
            held: Vec::new(),
            hints: Vec::new(),
        }
    }
}

/// The surface a surface is a subsurface of, however deep: the one a window's keyboard focus names.
fn root_of(surface: &WlSurface) -> WlSurface {
    let mut root = surface.clone();
    while let Some(parent) = get_parent(&root) {
        root = parent;
    }
    root
}

/// The rectangle of a surface, in its own coordinates, from the buffer it shows.
fn surface_extent(surface: &WlSurface) -> Option<Rectangle<i32, Logical>> {
    with_states(surface, |states| {
        let renderer = states.data_map.get::<RendererSurfaceStateUserData>()?;
        let size = renderer.lock().ok()?.surface_size()?;
        Some(Rectangle::new((0, 0).into(), size))
    })
}

pub trait PointerConstraintsExt: Host {
    /// The surface whose client holds an active pointer lock, if any.
    ///
    /// The answer is read per motion from Smithay's own record, so a lock the client
    /// destroyed or the pointer left cannot be seen as held, and one that has not started
    /// yet (its window does not have the keyboard) does not hold the pointer.
    #[must_use]
    fn locked_surface(&self) -> Option<WlSurface> {
        let focus = self.pointer().current_focus()?;
        (self.active_constraint_of(&focus) == Some(ZwpPointerConstraintsV1Kind::Locked)).then_some(focus)
    }

    /// Holds a position inside the region of the surface that holds the pointer, if that
    /// surface's client confined it.
    ///
    /// The region is surface-local and the position is global, so the surface's origin is
    /// what turns one into the other. The surface is the one that holds the pointer now,
    /// not the one under `position`: a motion that would carry the pointer off the surface
    /// is exactly the one that has to be held, and nothing lies under where it would land.
    /// A position is returned as it came when nothing is confined.
    #[must_use]
    fn confine(&self, position: Point<f64, Logical>) -> Point<f64, Logical> {
        let Some(surface) = self.pointer().current_focus() else {
            return position;
        };
        if self.active_constraint_of(&surface) != Some(ZwpPointerConstraintsV1Kind::Confined) {
            return position;
        }
        let (Some(origin), Some(bounds)) = (self.pointer_origin(), self.constraint_bounds(&surface)) else {
            return position;
        };
        let local = position - origin;
        let left = f64::from(bounds.loc.x);
        let top = f64::from(bounds.loc.y);
        // The right and bottom edges are exclusive, so the last pixel the pointer can stand on is the one
        // before them: a pointer on the edge itself is outside the surface and would lose its focus.
        let right = (f64::from(bounds.loc.x + bounds.size.w) - 1.0).max(left);
        let bottom = (f64::from(bounds.loc.y + bounds.size.h) - 1.0).max(top);
        origin + Point::from((local.x.clamp(left, right), local.y.clamp(top, bottom)))
    }

    /// The kind of the constraint a surface holds for the session's pointer while it is active.
    ///
    /// The kind comes from Smithay's own record of the constraint, so a constraint the
    /// client destroyed cannot be seen as held here: the two cannot disagree.
    fn active_constraint_of(&self, surface: &WlSurface) -> Option<ZwpPointerConstraintsV1Kind> {
        let pointer = self.pointer().clone();
        with_pointer_constraint(surface, &pointer, |constraint| {
            constraint
                .filter(|constraint: &PointerConstraintRef<'_, Self>| constraint.is_active())
                .map(|constraint| match &*constraint {
                    PointerConstraint::Locked(_) => ZwpPointerConstraintsV1Kind::Locked,
                    PointerConstraint::Confined(_) => ZwpPointerConstraintsV1Kind::Confined,
                })
        })
    }

    /// The region a surface's constraint holds the pointer to, in surface-local logical
    /// coordinates, or `None` when the client's region is empty.
    ///
    /// It is the bounding box of the region's added rectangles, clipped to the surface; a
    /// client that gave no region means the whole surface. A region with subtractions is
    /// treated as its bounding box: the protocol's own confinement is "stay inside the
    /// region", and a pointer held inside a bounding box never escapes the region itself.
    /// Refusing the subtraction is the safe direction of that approximation.
    fn constraint_bounds(&self, surface: &WlSurface) -> Option<Rectangle<i32, Logical>> {
        let extent = surface_extent(surface)?;
        let pointer = self.pointer().clone();
        let region = with_pointer_constraint(surface, &pointer, |constraint| constraint?.region().cloned());
        let Some(region) = region else {
            return Some(extent);
        };
        let mut bounds: Option<Rectangle<i32, Logical>> = None;
        for (kind, rect) in &region.rects {
            if !matches!(kind, RectangleKind::Add) {
                continue;
            }
            bounds = Some(bounds.map_or(*rect, |current| current.merge(*rect)));
        }
        bounds?.intersection(extent)
    }

    /// Whether the pointer is inside the region a surface's constraint holds it to.
    ///
    /// The protocol promises a client that the pointer is inside its region when the
    /// constraint starts, so a constraint whose region does not hold the pointer waits.
    fn pointer_in_constraint_region(&self, surface: &WlSurface) -> bool {
        let Some(origin) = self.pointer_origin() else {
            return false;
        };
        let local = self.pointer_position() - origin;
        let pointer = self.pointer().clone();
        let region = with_pointer_constraint(surface, &pointer, |constraint| constraint?.region().cloned());
        match region {
            // No region: the surface's own input region, which the pointer has focus through.
            None => true,
            Some(region) => region.contains((local.x.floor() as i32, local.y.floor() as i32)),
        }
    }

    /// The surface whose constraint may be active now, if it has one.
    ///
    /// It has to hold the pointer's focus and its window the keyboard, and the session must not be covered.
    fn constraint_target(&self) -> Option<WlSurface> {
        if !self.session_unlocked() {
            return None;
        }
        let surface = self.pointer().current_focus()?;
        let keyboard = self.keyboard_focus()?;
        (root_of(&surface) == root_of(&keyboard)).then_some(surface)
    }

    /// Brings the constraints in line with where the pointer and the keyboard are.
    ///
    /// An active constraint on any other surface ends, and the one on the surface that
    /// qualifies starts. It is called wherever the pointer's focus or the keyboard's moves,
    /// and when a constraint is made, so a client hears `locked` and `unlocked` as the
    /// window it asked for is chosen and left, however many times that happens.
    fn update_pointer_constraints(&mut self) {
        let target = self.constraint_target();
        let held = std::mem::take(&mut self.constraint_tables().held);
        for entry in held {
            if target.as_ref() == Some(&entry.surface) {
                self.constraint_tables().held.push(entry);
                continue;
            }
            with_pointer_constraint(&entry.surface, &entry.pointer, |constraint| {
                if let Some(constraint) = constraint {
                    log::debug!(area = "pointer-constraints", surface = ?entry.surface.id(), "deactivated");
                    constraint.deactivate();
                }
            });
        }
        let Some(surface) = target else {
            return;
        };
        if self
            .constraint_tables()
            .held
            .iter()
            .any(|entry| entry.surface == surface)
        {
            return;
        }
        let pointer = self.pointer().clone();
        let startable = self.pointer_in_constraint_region(&surface);
        let started = with_pointer_constraint(&surface, &pointer, |constraint| match constraint {
            Some(constraint) if startable => {
                constraint.activate();
                true
            }
            _ => false,
        });
        if started {
            log::debug!(area = "pointer-constraints", surface = ?surface.id(), "activated");
            self.constraint_tables()
                .held
                .push(ZwpPointerConstraintsV1HeldConstraint { surface, pointer });
        }
    }

    /// Releases every held constraint, telling each client its pointer is free.
    ///
    /// The session lock calls this before the cover is drawn: a covered client must not
    /// be able to keep the pointer captured. A persistent constraint stays asked for and
    /// starts again by the rules above once the cover is gone and the pointer is back on
    /// a window that has the keyboard.
    fn release_constraints(&mut self) {
        let held = std::mem::take(&mut self.constraint_tables().held);
        for entry in held {
            with_pointer_constraint(&entry.surface, &entry.pointer, |constraint| {
                if let Some(constraint) = constraint {
                    log::debug!(area = "pointer-constraints", surface = ?entry.surface.id(),
                        "released while the session is covered"
                    );
                    constraint.deactivate();
                }
            });
        }
        self.constraint_tables().hints.clear();
    }
}

impl<T: Host> PointerConstraintsExt for T {}

/// A constraint was made for `surface`: the constraints are brought in line with where the pointer and keyboard are.
pub fn new_constraint<H: Host>(host: &mut H) {
    // A covered session has no client surface under the pointer, so nothing may be
    // locked or confined: the constraint is held by the client and answered when
    // the session is revealed and the pointer arrives. A surface that has the
    // pointer but not the keyboard waits for the keyboard in the same way.
    host.update_pointer_constraints();
}

/// A constraint of `surface` ended.
pub fn remove_constraint<H: Host>(host: &mut H, surface: &WlSurface) {
    host.constraint_tables().held.retain(|entry| entry.surface != *surface);
    host.constraint_tables().hints.retain(|(held, _)| held != surface);
}

/// The client told where it draws the cursor while `surface` is locked.
pub fn cursor_position_hint<H: Host>(host: &mut H, surface: &WlSurface, location: Point<f64, Logical>) {
    host.constraint_tables().hints.retain(|(held, _)| held != surface);
    host.constraint_tables().hints.push((surface.clone(), location));
}
