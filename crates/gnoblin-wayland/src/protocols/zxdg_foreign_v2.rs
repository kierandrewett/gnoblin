//! XML protocol `xdg_foreign_unstable_v2` (`<protocol name="xdg_foreign_unstable_v2">`); interfaces served: `zxdg_exporter_v2`, `zxdg_importer_v2`.
//!
//! `zxdg_exporter_v2` and `zxdg_importer_v2` (xdg-foreign, unstable v2): one client
//! names a toplevel by a handle, and another client makes that toplevel its parent.
//!
//! Smithay mints the handle, holds the exported surface while it is exported, and checks
//! the parent relation an importer asks for. When `set_parent_of` changes a child's
//! parent, Smithay records it on the child's role and calls
//! `XdgShellHandler::parent_changed`. That hook feeds the window manager through
//! [`crate::State::sync_window_role`], so the relation reaches the window manager as a
//! `WindowRoleChanged` event and the session snapshot exposes it as `transient_for`.
//!
//! The window manager acts on the relation: the child becomes a floating transient and is
//! stacked above its parent, so an imported dialog never tiles over the window it belongs to.
//!
//! # Failure modes
//!
//! 1. **Export of a surface that is not a toplevel.** Smithay posts
//!    `zxdg_exporter_v2.invalid_surface`; nothing is exported.
//! 2. **Import of a handle no live export holds** (never minted, or revoked). Smithay
//!    sends the new `zxdg_imported_v2` a `destroyed` event; no relation is recorded.
//! 3. **`set_parent_of` naming a child the parent cannot take.** Smithay posts
//!    `zxdg_imported_v2.invalid_surface` and leaves the child's previous parent in place.
//! 4. **The export object is destroyed.** Smithay revokes every relation made through
//!    its handle and calls `parent_changed` for each affected child, so the window
//!    manager sees the parent cleared.
//! 5. **A handle outlives its export.** Handles are minted per export and stop resolving
//!    once revoked, so an old handle cannot name a new surface.

use smithay::reexports::wayland_server::DisplayHandle;
use smithay::wayland::xdg_foreign::{XdgForeignHandler, XdgForeignState};

use crate::State;

/// Creates the `zxdg_exporter_v2` and `zxdg_importer_v2` globals.
pub(crate) fn global(display: &DisplayHandle) -> XdgForeignState {
    XdgForeignState::new::<State>(display)
}

impl XdgForeignHandler for State {
    fn xdg_foreign_state(&mut self) -> &mut XdgForeignState {
        &mut self.xdg_foreign_state
    }
}
