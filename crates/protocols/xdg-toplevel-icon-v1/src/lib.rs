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
use smithay::reexports::wayland_protocols::xdg::toplevel_icon::v1::server::xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{GlobalDispatch, Resource};
use smithay::wayland::xdg_toplevel_icon::{
    XdgToplevelIconHandler, XdgToplevelIconManager, XdgToplevelIconManagerUserData,
};

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

gnoblin_log::logger!(log = "wayland");

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "xdg-toplevel-icon-v1",
    interfaces: &[ServedInterface::new("xdg_toplevel_icon_manager_v1", 1)],
    privilege: Privilege::Core,
};

/// Creates the `xdg_toplevel_icon_manager_v1` global.
pub fn register<D>(registrar: &Registrar<'_>) -> XdgToplevelIconManager
where
    D: XdgToplevelIconHandler + GlobalDispatch<XdgToplevelIconManagerV1, XdgToplevelIconManagerUserData> + 'static,
{
    XdgToplevelIconManager::new::<D>(registrar.display())
}

/// A client assigned an icon to `toplevel`. Nothing is stored here: the icon is read on the next toplevel commit.
pub fn set_icon(toplevel: &XdgToplevel, surface: &WlSurface) {
    log::debug!(area = "xdg-toplevel-icon", toplevel = %toplevel.id(),
        surface = %surface.id(),
        "icon assigned; read on the next toplevel commit"
    );
}
