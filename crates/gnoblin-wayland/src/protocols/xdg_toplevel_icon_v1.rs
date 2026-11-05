//! XML protocol `xdg_toplevel_icon_v1` (`<protocol name="xdg_toplevel_icon_v1">`); interfaces served: `xdg_toplevel_icon_manager_v1`.
//!
//! `xdg_toplevel_icon_manager_v1`: a client gives a toplevel an icon, by name or by pixels.
//!
//! An icon is a client's own picture of what its window is. The protocol is advisory.
//! Smithay validates the icon when the client commits it and keeps it in the
//! `wl_surface`'s cached state. The compositor reads it on each toplevel commit in
//! `State::toplevel_committed`, stores the icon's name and buffer count on the window, and
//! the session snapshot exposes both as `icon_name` and `icon_buffer_count`. A shell reads
//! the icon there; the compositor draws nothing from it.
//!
//! # Failure modes
//!
//! 1. **An icon buffer that is not `wl_shm`, or not square.** Smithay posts
//!    `xdg_toplevel_icon_v1.invalid_buffer`; the icon is not set and the window keeps the
//!    icon it had.
//! 2. **A buffer destroyed before the icon is set.** Smithay posts
//!    `xdg_toplevel_icon_v1.no_buffer`; no icon is set.
//! 3. **A request after the icon was assigned with `set_icon`.** Smithay posts
//!    `xdg_toplevel_icon_v1.immutable`; the assigned icon is not changed.
//! 4. **An icon set but not committed.** The icon is pending until the surface commits,
//!    so the snapshot shows the previous icon until then.
//! 5. **The icon drawn in a taskbar.** Not done: the compositor exposes the icon and does
//!    not draw it.

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::xdg_toplevel_icon::{XdgToplevelIconHandler, XdgToplevelIconManager};
use tracing::debug;

use crate::State;

/// Creates the `xdg_toplevel_icon_manager_v1` global.
pub(crate) fn global(display: &DisplayHandle) -> XdgToplevelIconManager {
    XdgToplevelIconManager::new::<State>(display)
}

impl XdgToplevelIconHandler for State {
    fn set_icon(&mut self, toplevel: XdgToplevel, wl_surface: WlSurface) {
        debug!(
            toplevel = %toplevel.id(),
            surface = %wl_surface.id(),
            "[xdg-toplevel-icon] icon assigned; read on the next toplevel commit"
        );
    }
}
