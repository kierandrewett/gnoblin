//! XML protocol `pointer_gestures_unstable_v1` (`<protocol name="pointer_gestures_unstable_v1">`); interfaces served: `zwp_pointer_gestures_v1`.
//!
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

use smithay::input::SeatHandler;
use smithay::reexports::wayland_protocols::wp::pointer_gestures::zv1::server::{
    zwp_pointer_gesture_hold_v1::ZwpPointerGestureHoldV1, zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1,
    zwp_pointer_gesture_swipe_v1::ZwpPointerGestureSwipeV1, zwp_pointer_gestures_v1::ZwpPointerGesturesV1,
};
use smithay::reexports::wayland_server::{Dispatch, GlobalDispatch};
use smithay::wayland::GlobalData;
use smithay::wayland::pointer_gestures::{PointerGestureUserData, PointerGesturesState};

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "zwp-pointer-gestures-v1",
    interfaces: &[ServedInterface::new("zwp_pointer_gestures_v1", 3)],
    privilege: Privilege::Core,
};

/// Creates the `zwp_pointer_gestures_v1` global.
pub fn register<D>(registrar: &Registrar<'_>) -> PointerGesturesState
where
    D: GlobalDispatch<ZwpPointerGesturesV1, GlobalData>
        + Dispatch<ZwpPointerGesturesV1, GlobalData>
        + Dispatch<ZwpPointerGestureSwipeV1, PointerGestureUserData<D>>
        + Dispatch<ZwpPointerGesturePinchV1, PointerGestureUserData<D>>
        + Dispatch<ZwpPointerGestureHoldV1, PointerGestureUserData<D>>
        + SeatHandler
        + 'static,
{
    PointerGesturesState::new::<D>(registrar.display())
}
