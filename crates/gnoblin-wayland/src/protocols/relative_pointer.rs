//! `zwp_relative_pointer_manager_v1`: the motion a pointer-locked client is given.
//!
//! A relative pointer reports the motion vector instead of a position, which is the only
//! motion a locked pointer can be said to have: the cursor does not move, so there is no
//! position to report. The global is therefore half of the pointer-lock story —
//! [`crate::protocols::pointer_constraints`] holds the pointer still, and this protocol is
//! where the movement that would have happened goes.
//!
//! The compositor sends `relative_motion` from one place, [`crate::State::send_relative_motion`],
//! called on both of the motions the session has: while a lock is active the whole motion
//! becomes a relative event, and otherwise a motion still carries its delta so a client that
//! only wants the vector (a game reading the mouse, a remote-desktop session) does not have
//! to difference two positions.
//!
//! # Failure modes
//!
//! 1. **A locked client receives nothing while the mouse moves.** The lock turns each
//!    motion into a relative event addressed to the focused surface, so the client sees the
//!    movement: `a_locked_pointer_reports_motion_as_relative_events()` in `tests/input.rs`.
//! 2. **A relative event reaches a client that is not under the pointer.** The event is
//!    addressed to the surface the pointer is on, so a client that has no focus gets
//!    nothing and cannot observe another client's motion.

use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::relative_pointer::RelativePointerManagerState;

use crate::State;

/// Creates the `zwp_relative_pointer_manager_v1` global.
pub(crate) fn global(display: &DisplayHandle) -> RelativePointerManagerState {
    RelativePointerManagerState::new::<State>(display)
}
