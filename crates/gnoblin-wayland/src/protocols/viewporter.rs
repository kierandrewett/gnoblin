//! `wp_viewporter`.
//!
//! Smithay needs no handler trait for this protocol: the global is created once and
//! a viewport is validated on commit by `on_commit_buffer_handler`, which
//! [`crate::protocols::compositor`] already calls for every surface. A client that
//! sets a viewport outside its buffer gets a protocol error on that client, not a
//! session failure.
//!
//! This module exists because the crate keeps one module per protocol
//! (`design/rust-compositor.md`, "Project structure"). It is where the renderer will
//! read `ViewportCachedState` for the source rectangle once it draws, and it owns
//! the global's creation so the protocol's initialisation sits with the protocol.

use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::viewporter::ViewporterState;

use crate::State;

/// Creates the `wp_viewporter` global.
pub(crate) fn global(display: &DisplayHandle) -> ViewporterState {
    ViewporterState::new::<State>(display)
}
