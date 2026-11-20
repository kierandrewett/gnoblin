//! XML protocol `wayland` (`<protocol name="wayland">`); interfaces served: `wl_data_device_manager`.
//!
//! The clipboard: `wl_data_device_manager` (XML `wayland.xml`) and the selection state every
//! selection protocol shares. The primary selection, `ext_data_control_v1` and
//! `zwlr_data_control_v1` are in their own modules.
//!
//! ## What the compositor's part is
//!
//! Smithay owns the protocol objects, the offers and the seat's record of which
//! selection is current. Two things stay here:
//!
//! - the *source* of the current selection, because a `receive` request from another
//!   client is answered by handing the destination a pipe and asking the source's
//!   client to write into it — the compositor never copies the bytes itself;
//! - the two privileged globals, which a clipboard manager uses to read and write the
//!   selection without being the focused application. Those are decided per client
//!   (`gnoblin_wayland_api::access`): a sandboxed or unprivileged application does not see them at
//!   all, so a manager cannot be smuggled into a sandbox.
//!
//! ## Focus is the gate for the ordinary clipboard
//!
//! `wl_data_device` only delivers a selection to the client that has keyboard focus,
//! so the keyboard focus and the data-device focus are kept in step here: a client
//! that is focused may paste, and one that is not has no offer to paste from. That is
//! the protocol's rule, not an extra one.
//!
//! ## The host's clipboard
//!
//! A nested session shares its clipboard and primary selection with the compositor that hosts it
//! (`gnoblin_backend::selection`, which states what crosses and how much). The host's selection is a compositor-owned
//! source here, exactly as an X11 client's is (the compositor's selection bridge), and a paste from it asks the backend to read
//! the host's bytes into the client's pipe. A copy a client of the session makes is told to the backend, which offers it
//! to the host while the window has the host's keyboard. Nothing is read until somebody pastes.
//!
//! ## Failure modes first
//!
//! Each is a test in `tests/selection.rs` unless it says otherwise.
//!
//! 1. **A client pastes a selection whose owner has gone.** The offer is cleared when
//!    the source's client disconnects, so the destination is told the selection is
//!    empty instead of holding an offer that can only be answered with nothing —
//!    `a_selection_does_not_outlive_its_owner`.
//! 2. **A data-control manager is visible to an application.** Both data-control
//!    globals are filtered: an unprivileged connection does not see them, a privileged
//!    one does (`a_data_control_manager_needs_a_privileged_client`), and a sandboxed
//!    one does not either (`a_sandboxed_client_sees_no_privileged_global` in
//!    `tests/access.rs`).
//! 3. **A paste is served from a copy the compositor kept.** The bytes come from the
//!    source client through the pipe, so a source that writes nothing yields nothing:
//!    the compositor holds a mime-type list and a handle, never the data —
//!    `a_copy_between_two_clients_carries_the_source_bytes`.
//! 4. **A privileged paste request after the manager's own protocol is off.** Both
//!    data-control globals answer to the single `ext_data_control` configuration name,
//!    so turning the family off hides both: `turning_data_control_off_hides_both_managers`.

use smithay::reexports::wayland_server::GlobalDispatch;
use smithay::reexports::wayland_server::protocol::wl_data_device_manager::WlDataDeviceManager;
use smithay::wayland::GlobalData;
use smithay::wayland::selection::SelectionTarget;
use smithay::wayland::selection::data_device::{DataDeviceHandler, DataDeviceState};

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "wl-data-device-manager",
    interfaces: &[ServedInterface::new("wl_data_device_manager", 3)],
    privilege: Privilege::Core,
};

/// Creates the `wl_data_device_manager` global.
pub fn register<D>(registrar: &Registrar<'_>) -> DataDeviceState
where
    D: GlobalDispatch<WlDataDeviceManager, GlobalData> + DataDeviceHandler + 'static,
{
    DataDeviceState::new::<D>(registrar.display())
}

/// The session's selections.
///
/// Only their mime types are recorded, and only for the observation: the bytes never
/// pass through the compositor. Smithay forwards a `receive` on a client-set offer
/// straight to the source's own client, so nothing here is in the data path — which is
/// also why a source whose owner has gone cannot serve stale bytes: the offer itself is
/// withdrawn when the source resource is destroyed.
#[derive(Default)]
pub struct WlDataDeviceManagerSelections {
    clipboard: Option<Vec<String>>,
    primary: Option<Vec<String>>,
}

impl WlDataDeviceManagerSelections {
    /// The mime types of a target, for the observation.
    #[must_use]
    pub fn mime_types(&self, target: SelectionTarget) -> Option<Vec<String>> {
        match target {
            SelectionTarget::Clipboard => self.clipboard.clone(),
            SelectionTarget::Primary => self.primary.clone(),
        }
    }

    /// Replaces a target's selection.
    pub fn set(&mut self, target: SelectionTarget, mime_types: Option<Vec<String>>) {
        let slot = match target {
            SelectionTarget::Clipboard => &mut self.clipboard,
            SelectionTarget::Primary => &mut self.primary,
        };
        *slot = mime_types;
    }
}
