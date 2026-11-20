//! `gnoblin-wayland-zwlr-output-management-v1`: the manager's bookkeeping lives on the state; the engine that answers a
//! configuration is `output_config`.

use gnoblin_wayland_protocols::zwlr_output_management_v1::{Host, ZwlrOutputManagementV1State};

use crate::State;

impl Host for State {
    fn output_manager(&mut self) -> &mut ZwlrOutputManagementV1State {
        &mut self.output_manager
    }
}
