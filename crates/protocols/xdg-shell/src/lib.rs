//! XML protocol `xdg_shell` (`<protocol name="xdg_shell">`); interfaces served: `xdg_wm_base`.
//!
//! `xdg_wm_base`: the global every toplevel and popup is made through.
//!
//! The crate creates the global and says what it advertises. What a toplevel or a popup does once it exists is a decision of
//! the window manager and the popup grabs, so the handler (`XdgShellHandler`) is the compositor's and sits in its glue module
//! with the state it answers for.

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_wm_base::XdgWmBase;
use smithay::reexports::wayland_server::GlobalDispatch;
use smithay::wayland::GlobalData;
use smithay::wayland::shell::xdg::XdgShellState;

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "xdg-shell",
    interfaces: &[ServedInterface::new("xdg_wm_base", 7)],
    privilege: Privilege::Core,
};

/// Creates the `xdg_wm_base` global.
pub fn register<D>(registrar: &Registrar<'_>) -> XdgShellState
where
    D: GlobalDispatch<XdgWmBase, GlobalData> + 'static,
{
    XdgShellState::new::<D>(registrar.display())
}
