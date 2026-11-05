//! `wl_seat`: keyboard and pointer focus over surfaces.
//!
//! The seat owns focus; this module only states what a focus target is. A focus
//! target is a `WlSurface` rather than a window wrapper, because the window manager
//! already answers "which window", and a second identity between the two would have
//! to be kept in step with this one.
//!
//! Focus itself moves in [`crate::State`], driven by the window manager's
//! `Effect::Focus`, so a protocol handler never decides who has focus.

use smithay::input::SeatHandler;
use smithay::input::SeatState;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::pointer_constraints::PointerConstraintsHandler;

use crate::State;

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }
}

/// Pointer constraints are refused by not creating the constraints: Smithay needs
/// this handler because a `WlSurface` focus target requires it, and a slice with no
/// pointer lock answers nothing rather than half-locking the pointer.
impl PointerConstraintsHandler for State {}
