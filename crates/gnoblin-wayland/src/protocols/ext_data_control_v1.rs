//! XML protocol `ext_data_control_v1` (`<protocol name="ext_data_control_v1">`); interfaces served: `ext_data_control_manager_v1`.
//!
//! `ext_data_control_manager_v1` and `ext_data_control_device_v1` (XML
//! `ext-data-control-v1`).
//!
//! A privileged clipboard manager reads and writes every selection through this
//! protocol. The global is filtered by [`crate::access`]; see `wl_data_device_manager`
//! for the selection state it shares.
//!
//! # Refusals and failure modes
//!
//! 1. **An unprivileged client.** The manager global is filtered at bind time by
//!    [`AccessPolicy`](crate::access::AccessPolicy), so an ordinary or sandboxed client
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

use smithay::wayland::selection::ext_data_control::{
    DataControlHandler as ExtDataControlHandler, DataControlState as ExtDataControlState,
};

use crate::State;

impl ExtDataControlHandler for State {
    fn data_control_state(&mut self) -> &mut ExtDataControlState {
        &mut self.ext_data_control_state
    }
}
