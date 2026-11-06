//! Compat adapters the session serves, registered on its Wayland configuration.
//!
//! The session binary builds each adapter and hands the compositor the `Extension` values.
//! This module names no adapter crate: it only appends what it is given, so the compositor
//! stays free of compat crates (see `xtask/src/deps.rs`).

use std::sync::Arc;

use crate::{Extension, SessionConfig};

/// Adds the adapters to the Wayland session's extensions, after any already configured.
///
/// The server registers each extension on its display at startup, so an adapter that is
/// not in this list is never advertised.
pub fn configure_compat_extensions(config: &mut SessionConfig, extensions: Vec<Arc<dyn Extension>>) {
    config.wayland.extensions.extend(extensions);
}
