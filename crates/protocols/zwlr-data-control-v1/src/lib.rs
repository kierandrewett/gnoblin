//! XML protocol `wlr_data_control_unstable_v1` (`<protocol name="wlr_data_control_unstable_v1">`); interfaces served: `zwlr_data_control_manager_v1`.
//!
//! `zwlr_data_control_manager_v1` and `zwlr_data_control_device_v1` (XML
//! `wlr-data-control-unstable-v1`).
//!
//! The wlroots spelling of the privileged clipboard capability, filtered like
//! `ext_data_control_v1`.
//!
//! # Refusals and failure modes
//!
//! Filtered and switched off exactly as [`ext_data_control_v1`](super::ext_data_control_v1)
//! is: an unprivileged client never binds the wlroots manager, and turning data control off
//! hides it.
//!
//! # Guarding tests
//!
//! `tests/selection.rs`: `turning_data_control_off_hides_both_managers`,
//! `a_data_control_manager_needs_a_privileged_client`.

use smithay::reexports::wayland_protocols_wlr::data_control::v1::server::zwlr_data_control_manager_v1::ZwlrDataControlManagerV1;
use smithay::reexports::wayland_server::GlobalDispatch;
use smithay::wayland::selection::primary_selection::PrimarySelectionState;
use smithay::wayland::selection::wlr_data_control::{
    DataControlManagerGlobalData, DataControlState as WlrDataControlState,
};

use gnoblin_wayland_api::access::Global;
use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "zwlr-data-control-v1",
    interfaces: &[ServedInterface::new("zwlr_data_control_manager_v1", 2)],
    privilege: Privilege::Roster(Global::WlrDataControl),
};

/// Creates the `zwlr_data_control_manager_v1` global, visible to the clients the roster calls privileged. The primary
/// selection is offered through it too.
pub fn register<D>(registrar: &Registrar<'_>, primary_selection: &PrimarySelectionState) -> WlrDataControlState
where
    D: GlobalDispatch<ZwlrDataControlManagerV1, DataControlManagerGlobalData> + 'static,
{
    WlrDataControlState::new::<D, _>(
        registrar.display(),
        Some(primary_selection),
        registrar.filter(Global::WlrDataControl),
    )
}
