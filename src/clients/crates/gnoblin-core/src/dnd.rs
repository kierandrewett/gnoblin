//! Do-Not-Disturb: a tiny cross-process flag (presence of a runtime file). The
//! topbar's control-centre toggles it; the notification daemon checks it before
//! showing a popup. Lightweight on purpose — no extra D-Bus surface.

use crate::FileFlag;

static FLAG: FileFlag = FileFlag::new("gnoblin-dnd");

/// Is Do-Not-Disturb currently on?
pub fn is_on() -> bool {
    FLAG.is_on()
}

/// Turn DND on/off.
pub fn set(on: bool) {
    FLAG.set(on);
}

/// Flip DND, returning the new state.
pub fn toggle() -> bool {
    FLAG.toggle()
}
