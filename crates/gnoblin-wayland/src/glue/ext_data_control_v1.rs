//! `gnoblin-wayland-ext-data-control-v1`: the state owns Smithay's data-control state.

use smithay::wayland::selection::ext_data_control::{
    DataControlHandler as ExtDataControlHandler, DataControlState as ExtDataControlState,
};

use crate::State;

impl ExtDataControlHandler for State {
    fn data_control_state(&mut self) -> &mut ExtDataControlState {
        &mut self.ext_data_control_state
    }
}
