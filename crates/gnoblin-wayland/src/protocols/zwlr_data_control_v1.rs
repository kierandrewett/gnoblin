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

use smithay::wayland::selection::wlr_data_control::{
    DataControlHandler as WlrDataControlHandler, DataControlState as WlrDataControlState,
};

use crate::State;

impl WlrDataControlHandler for State {
    fn data_control_state(&mut self) -> &mut WlrDataControlState {
        &mut self.wlr_data_control_state
    }
}
