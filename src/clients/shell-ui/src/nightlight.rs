//! Night Light: a cross-process flag (presence of a runtime file), mirroring
//! [`crate::dnd`]. The topbar's control-centre tile toggles it; the
//! `gnoblin-night-light` daemon watches it and warms the screen gamma when on.
//! Lightweight on purpose — no extra D-Bus surface.

use crate::config::Config;
use std::path::PathBuf;

/// Default colour temperature (Kelvin) when night light is on. ~4000K is a
/// comfortable warm white; 6500K is neutral daylight (effectively off).
pub const DEFAULT_TEMP: u16 = 4000;

fn path() -> Option<PathBuf> {
    std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|d| PathBuf::from(d).join("gnoblin-nightlight"))
}

/// Is Night Light currently on?
pub fn is_on() -> bool {
    path().map(|p| p.exists()).unwrap_or(false)
}

/// Turn Night Light on/off.
pub fn set(on: bool) {
    let Some(p) = path() else { return };
    if on {
        let _ = std::fs::write(&p, b"");
    } else {
        let _ = std::fs::remove_file(&p);
    }
}

/// Flip Night Light, returning the new state.
pub fn toggle() -> bool {
    let next = !is_on();
    set(next);
    next
}

/// Configured colour temperature in Kelvin (`[appearance] night-light-temperature`),
/// clamped to a sane range; falls back to [`DEFAULT_TEMP`].
pub fn temperature() -> u16 {
    Config::load()
        .get("appearance", "night-light-temperature")
        .and_then(|s| s.trim().parse::<u16>().ok())
        .unwrap_or(DEFAULT_TEMP)
        .clamp(1000, 6500)
}
