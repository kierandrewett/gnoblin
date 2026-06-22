//! Do-Not-Disturb: a tiny cross-process flag (presence of a runtime file). The
//! topbar's control-centre toggles it; the notification daemon checks it before
//! showing a popup. Lightweight on purpose — no extra D-Bus surface.

use std::path::PathBuf;

fn path() -> Option<PathBuf> {
    std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|d| PathBuf::from(d).join("gnoblin-dnd"))
}

/// Is Do-Not-Disturb currently on?
pub fn is_on() -> bool {
    path().map(|p| p.exists()).unwrap_or(false)
}

/// Turn DND on/off.
pub fn set(on: bool) {
    let Some(p) = path() else { return };
    if on {
        let _ = std::fs::write(&p, b"");
    } else {
        let _ = std::fs::remove_file(&p);
    }
}

/// Flip DND, returning the new state.
pub fn toggle() -> bool {
    let next = !is_on();
    set(next);
    next
}
