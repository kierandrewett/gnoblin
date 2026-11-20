//! XML protocol `xdg_dialog_v1` (`<protocol name="xdg_dialog_v1">`); interfaces served: `xdg_wm_dialog_v1`.
//!
//! `xdg_wm_dialog_v1`: a client says a toplevel is a dialog, and whether it is modal.
//!
//! Smithay keeps the hint on the toplevel's role. Each change is forwarded to the window
//! record through `State::sync_window_role`, which records the window's kind as
//! `Modal` or `Dialog` when the window has a parent, and as `Normal` otherwise. The kind
//! is exposed on the session snapshot as `modal`, so a shell can read whether a window
//! asked to be modal.
//!
//! The hint is only honoured for a toplevel with a parent. The protocol makes the hint
//! advisory for a parentless toplevel, so a parentless dialog is a normal window.
//!
//! Modality is enforced by the no-shell fallback, which is why this global is advertised: an
//! open modal child takes focus from its parent, and a parent's focus request is redirected
//! to that child until the child is no longer modal. A dialog stays a transient above its
//! parent, so it is never tiled over the window it belongs to. While a shell holds the tree
//! the stock shell makes a dialog a child of its parent and the compositor stacks it above.
//!
//! # Failure modes
//!
//! 1. **A second dialog object for one toplevel.** Smithay posts
//!    `xdg_wm_dialog_v1.already_used`; the first dialog keeps the toplevel.
//! 2. **A hint outlives its toplevel.** The hint lives on the toplevel's role, so it is
//!    dropped with the toplevel and the window manager forgets the window's kind with it.
//! 3. **A hint with no parent.** The window manager records `Normal`, because a dialog
//!    without a parent has nothing to be modal to.
//! 4. **The parent is blocked only while the modal is open.** Unsetting modality, or
//!    closing the child, releases the parent to take focus again.

use gnoblin_wayland_api::host::WindowHost;
use smithay::reexports::wayland_protocols::xdg::dialog::v1::server::xdg_wm_dialog_v1::XdgWmDialogV1;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Dispatch, GlobalDispatch};
use smithay::wayland::GlobalData;
use smithay::wayland::shell::xdg::dialog::XdgDialogState;

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "xdg-dialog-v1",
    interfaces: &[ServedInterface::new("xdg_wm_dialog_v1", 1)],
    privilege: Privilege::Core,
};

/// Creates the `xdg_wm_dialog_v1` global.
pub fn register<D>(registrar: &Registrar<'_>) -> XdgDialogState
where
    D: GlobalDispatch<XdgWmDialogV1, GlobalData> + Dispatch<XdgWmDialogV1, GlobalData> + 'static,
{
    XdgDialogState::new::<D>(registrar.display())
}

/// The dialog hint of the toplevel owning `surface` changed: the window's kind is read again.
pub fn dialog_hint_changed<H: WindowHost>(host: &mut H, surface: &WlSurface) {
    if let Some(window) = host.window_of(surface) {
        host.sync_window_role(window);
    }
}
