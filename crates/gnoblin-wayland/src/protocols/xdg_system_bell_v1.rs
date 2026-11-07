//! XML protocol `xdg_system_bell_v1` (`<protocol name="xdg_system_bell_v1">`); interfaces served: `xdg_system_bell_v1`.
//!
//! `xdg_system_bell_v1`: a client asks the session to ring its system bell, optionally naming the
//! surface that rang it.
//!
//! The request carries intent, not a sound. The compositor counts every ring and, when the
//! session's visual bell is configured, flashes the screen (see [`State::ring_bell`]). The surface
//! is optional: when a client names one, the ring is attributed to the window that owns it, which
//! the debug log records. The published snapshot carries only the count, so a shell reads how
//! many times the bell rang, not which window rang it. A surface that owns no window, or no surface
//! at all, still rings: the bell is a session-wide event.
//!
//! Smithay keeps the global and parses the request; this module is the handler that maps the named
//! surface to its window and forwards the ring. Nothing here decides placement or focus.
//!
//! # Failure modes
//!
//! 1. **A surface that names no window.** A client may name a surface that is not a mapped
//!    toplevel (a popup, a subsurface, or one already destroyed). The bell still rings, attributed
//!    to no window, rather than being dropped.
//! 2. **A ring from a locked session.** The count still rises, because the protocol has no notion
//!    of a lock, but the visual flash is suppressed while locked so the bell cannot reveal what the
//!    lock hides. [`State::ring_bell`] owns that rule; this module does not repeat it.
//! 3. **A ring storm.** Each request is counted and flashed once. The protocol has no rate limit,
//!    so a misbehaving client can flash the screen repeatedly; a limit is not added here because
//!    the visual bell is opt-in and the configured flash is already the user's choice.

use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::xdg_system_bell::{XdgSystemBellHandler, XdgSystemBellState};

use crate::State;

/// Creates the `xdg_system_bell_v1` global.
pub(crate) fn global(display: &DisplayHandle) -> XdgSystemBellState {
    XdgSystemBellState::new::<State>(display)
}

impl XdgSystemBellHandler for State {
    /// A client rang the bell. The named surface, when it owns a window, names that window;
    /// otherwise the ring is session-wide and carries no window.
    fn ring(&mut self, surface: Option<WlSurface>) {
        let window = surface.as_ref().and_then(|surface| self.window_of(surface));
        self.ring_bell(window);
    }
}
