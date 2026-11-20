//! `gnoblin-wayland-zwp-pointer-constraints-v1`: the constraints follow the state's pointer and keyboard.

use gnoblin_wayland_protocols::zwp_pointer_constraints_v1::{
    self as constraints, Host, PointerConstraintsExt as _, ZwpPointerConstraintsV1Kind, ZwpPointerConstraintsV1Tables,
};
use smithay::input::pointer::PointerHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::pointer_constraints::{ConstraintRemove, PointerConstraintsHandler};

use crate::State;

impl Host for State {
    fn pointer(&self) -> &PointerHandle<Self> {
        &self.pointer
    }

    fn keyboard_focus(&self) -> Option<WlSurface> {
        self.keyboard.current_focus()
    }

    fn pointer_origin(&self) -> Option<Point<f64, Logical>> {
        self.pointer_origin
    }

    fn pointer_position(&self) -> Point<f64, Logical> {
        self.pointer_position
    }

    fn session_unlocked(&self) -> bool {
        self.lock.state() == gnoblin_lock::State::Unlocked
    }

    fn constraint_tables(&mut self) -> &mut ZwpPointerConstraintsV1Tables<Self> {
        &mut self.constraint_tables
    }
}

impl PointerConstraintsHandler for State {
    fn new_constraint(&mut self, _surface: &WlSurface, _pointer: &PointerHandle<Self>) {
        constraints::new_constraint(self);
    }

    fn remove_constraint(&mut self, surface: &WlSurface, _pointer: &PointerHandle<Self>, _reason: ConstraintRemove) {
        constraints::remove_constraint(self, surface);
    }

    fn cursor_position_hint(
        &mut self,
        surface: &WlSurface,
        _pointer: &PointerHandle<Self>,
        location: Point<f64, Logical>,
    ) {
        constraints::cursor_position_hint(self, surface, location);
    }
}

impl State {
    /// What the session's clients hold the pointer to now, for a backend whose pointer is another compositor's.
    ///
    /// It is read per turn from the same record `locked_surface` reads, so a constraint the client destroyed,
    /// or one that has not started because its window lacks the keyboard, is not asked of the host.
    #[must_use]
    pub(crate) fn backend_pointer_constraint(&self) -> gnoblin_backend::PointerConstraint {
        let Some(focus) = self.pointer.current_focus() else {
            return gnoblin_backend::PointerConstraint::Free;
        };
        match self.active_constraint_of(&focus) {
            Some(ZwpPointerConstraintsV1Kind::Locked) => gnoblin_backend::PointerConstraint::Locked,
            Some(ZwpPointerConstraintsV1Kind::Confined) => gnoblin_backend::PointerConstraint::Confined,
            None => gnoblin_backend::PointerConstraint::Free,
        }
    }
}
