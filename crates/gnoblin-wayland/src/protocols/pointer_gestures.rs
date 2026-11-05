//! `zwp_pointer_gestures_v1`: swipe, pinch and hold gestures reported to a client.
//!
//! The global is all this module creates, and that is the whole of the compositor's part
//! in the protocol: Smithay keeps a gesture object with the pointer it belongs to, and the
//! pointer's own calls — `gesture_swipe_*`, `gesture_pinch_*` and `gesture_hold_*` — reach
//! every gesture object of the client that holds pointer focus, in the order they were
//! sent.
//!
//! A gesture's *recognition* is not here either: a touchpad's swipe is decided by
//! `gnoblin-input`'s recogniser, whose `GestureEvent` is a plain value the input path turns
//! into those calls. Keeping the recogniser pure is what makes a gesture test a test of the
//! sequence rather than of a libinput device.
//!
//! # Failure modes
//!
//! 1. **A gesture reaches a client that does not hold the pointer.** The calls are
//!    addressed to the pointer's focus, so a client without focus receives nothing and
//!    cannot observe another client's touchpad.
//! 2. **A gesture object outlives its pointer.** The object is stored with the pointer by
//!    Smithay and dropped with it, so a second pointer (a nested session's, a
//!    remote-desktop client's) cannot inherit an object built for the first.

use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::pointer_gestures::PointerGesturesState;

use crate::State;

/// Creates the `zwp_pointer_gestures_v1` global.
pub(crate) fn global(display: &DisplayHandle) -> PointerGesturesState {
    PointerGesturesState::new::<State>(display)
}
