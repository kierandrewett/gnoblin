//! `zwlr_data_control_manager_v1` and `zwlr_data_control_device_v1` (XML
//! `wlr-data-control-unstable-v1`).
//!
//! The wlroots spelling of the privileged clipboard capability, filtered like
//! `ext_data_control_v1`.

use smithay::wayland::selection::wlr_data_control::{
    DataControlHandler as WlrDataControlHandler, DataControlState as WlrDataControlState,
};

use crate::State;

impl WlrDataControlHandler for State {
    fn data_control_state(&mut self) -> &mut WlrDataControlState {
        &mut self.wlr_data_control_state
    }
}
