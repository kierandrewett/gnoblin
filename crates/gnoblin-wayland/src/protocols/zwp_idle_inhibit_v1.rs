//! XML protocol `idle_inhibit_unstable_v1` (`<protocol name="idle_inhibit_unstable_v1">`); interfaces served: `zwp_idle_inhibit_manager_v1`.
//!
//! `zwp_idle_inhibit_manager_v1`: a client asks that the session never idle.
//!
//! A video player and a presentation tool both need it: the screen must not blank while
//! the user is watching something and not touching the input devices. The protocol's
//! object is a surface, so the inhibitor's lifetime is the surface's, and the answer —
//! "is this session idle?" — is decided in [`crate::protocols::ext_idle_notify_v1`], because a
//! second place deciding it would be a second answer to one question.
//!
//! # Failure modes
//!
//! 1. **An inhibitor leaks past its client.** The surface is checked for liveness on
//!    every idle evaluation, so a killed client's inhibitor stops counting even though
//!    its `destroy` request never arrived:
//!    `an_idle_inhibitor_suppresses_idle_and_never_leaks` in `tests/protocols.rs`.
//! 2. **An inhibitor outlives the object that asked for it.** Destroying the inhibitor
//!    object calls `uninhibit` for that surface through Smithay's request handler, so
//!    the session idles again on the next advance.

use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::idle_inhibit::{IdleInhibitHandler, IdleInhibitManagerState};

use crate::State;

/// Creates the `zwp_idle_inhibit_manager_v1` global.
pub(crate) fn global(display: &DisplayHandle) -> IdleInhibitManagerState {
    IdleInhibitManagerState::new::<State>(display)
}

impl IdleInhibitHandler for State {
    fn inhibit(&mut self, surface: WlSurface) {
        self.set_idle_inhibited(&surface, true);
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.set_idle_inhibited(&surface, false);
    }
}
