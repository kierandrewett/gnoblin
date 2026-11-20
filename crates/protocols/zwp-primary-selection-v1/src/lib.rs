//! XML protocol `wp_primary_selection_unstable_v1` (`<protocol name="wp_primary_selection_unstable_v1">`); interfaces served: `zwp_primary_selection_device_manager_v1`.
//!
//! `zwp_primary_selection_device_manager_v1` (XML `primary-selection-unstable-v1`).
//!
//! The primary selection is what a middle click pastes. Its state lives on `State`
//! beside the clipboard; the handler here answers Smithay's trait for it.
//!
//! # Refusals and failure modes
//!
//! The primary selection is unprivileged, so every client may bind it. Its owner-gone
//! rule is the clipboard's: a selection is cleared when its owner goes. Guarded by
//! `tests/selection.rs` (`a_selection_does_not_outlive_its_owner`).

use smithay::reexports::wayland_protocols::wp::primary_selection::zv1::server::zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1;
use smithay::reexports::wayland_server::GlobalDispatch;
use smithay::wayland::selection::primary_selection::{
    PrimaryDeviceManagerGlobalData, PrimarySelectionHandler, PrimarySelectionState,
};

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "zwp-primary-selection-v1",
    interfaces: &[ServedInterface::new("zwp_primary_selection_device_manager_v1", 1)],
    privilege: Privilege::Core,
};

/// Creates the `zwp_primary_selection_device_manager_v1` global, visible to every client.
pub fn register<D>(registrar: &Registrar<'_>) -> PrimarySelectionState
where
    D: GlobalDispatch<ZwpPrimarySelectionDeviceManagerV1, PrimaryDeviceManagerGlobalData>
        + PrimarySelectionHandler
        + 'static,
{
    PrimarySelectionState::new::<D>(registrar.display())
}
