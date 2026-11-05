//! `ext_data_control_manager_v1` and `ext_data_control_device_v1` (XML
//! `ext-data-control-v1`).
//!
//! A privileged clipboard manager reads and writes every selection through this
//! protocol. The global is filtered by [`crate::access`]; see `wl_data_device_manager`
//! for the selection state it shares.

use smithay::wayland::selection::ext_data_control::{
    DataControlHandler as ExtDataControlHandler, DataControlState as ExtDataControlState,
};

use crate::State;

impl ExtDataControlHandler for State {
    fn data_control_state(&mut self) -> &mut ExtDataControlState {
        &mut self.ext_data_control_state
    }
}
