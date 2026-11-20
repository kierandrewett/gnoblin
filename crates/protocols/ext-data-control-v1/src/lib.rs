//! XML protocol `ext_data_control_v1` (`<protocol name="ext_data_control_v1">`); interfaces served: `ext_data_control_manager_v1`.
//!
//! `ext_data_control_manager_v1` and `ext_data_control_device_v1` (XML
//! `ext-data-control-v1`).
//!
//! A privileged clipboard manager reads and writes every selection through this
//! protocol. The global is filtered by `gnoblin_wayland_api::access`; see `wl_data_device_manager`
//! for the selection state it shares.
//!
//! # Refusals and failure modes
//!
//! 1. **An unprivileged client.** The manager global is filtered at bind time by
//!    [`AccessPolicy`](gnoblin_wayland_api::access::AccessPolicy), so an ordinary or sandboxed client
//!    never sees it; it is not offered and then refused.
//! 2. **Data control switched off.** The configuration hides both manager globals
//!    (`ext` and `wlr`) together, so a disabled session advertises neither.
//! 3. **A selection whose owner has gone.** The selection is cleared with its owner; a
//!    manager never reads a stale source.
//!
//! # Guarding tests
//!
//! `tests/selection.rs`: `a_data_control_manager_needs_a_privileged_client`,
//! `turning_data_control_off_hides_both_managers`, `a_selection_does_not_outlive_its_owner`.

use smithay::reexports::wayland_protocols::ext::data_control::v1::server::ext_data_control_manager_v1::ExtDataControlManagerV1;
use smithay::reexports::wayland_server::GlobalDispatch;
use smithay::wayland::selection::ext_data_control::{
    DataControlState as ExtDataControlState, ExtDataControlManagerGlobalData,
};
use smithay::wayland::selection::primary_selection::PrimarySelectionState;

use gnoblin_wayland_api::access::Global;
use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "ext-data-control-v1",
    interfaces: &[ServedInterface::new("ext_data_control_manager_v1", 1)],
    privilege: Privilege::Roster(Global::DataControl),
};

/// Creates the `ext_data_control_manager_v1` global, visible to the clients the roster calls privileged. The primary
/// selection is offered through it too.
pub fn register<D>(registrar: &Registrar<'_>, primary_selection: &PrimarySelectionState) -> ExtDataControlState
where
    D: GlobalDispatch<ExtDataControlManagerV1, ExtDataControlManagerGlobalData> + 'static,
{
    ExtDataControlState::new::<D, _>(
        registrar.display(),
        Some(primary_selection),
        registrar.filter(Global::DataControl),
    )
}
