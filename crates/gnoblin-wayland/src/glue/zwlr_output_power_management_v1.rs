//! `gnoblin-wayland-zwlr-output-power-management-v1`: who holds each output's power is kept with the output controls.

use gnoblin_types::OutputId;
use gnoblin_wayland_protocols::zwlr_output_power_management_v1::Host;
use smithay::reexports::wayland_protocols_wlr::output_power_management::v1::server::zwlr_output_power_v1::ZwlrOutputPowerV1;

use crate::State;

impl Host for State {
    fn power_supported(&self) -> bool {
        self.output_controls.power_supported()
    }

    fn power_free(&self, output: OutputId) -> bool {
        self.output_controls.power_free(output)
    }

    fn power(&self, output: OutputId) -> bool {
        self.output_controls.power(output)
    }

    fn power_holder_is(&self, output: OutputId, object: &ZwlrOutputPowerV1) -> bool {
        self.output_controls.power_holder_is(output, object)
    }

    fn hold_power(&mut self, output: OutputId, object: ZwlrOutputPowerV1) {
        self.output_controls.hold_power(output, object);
    }

    fn queue_power(&mut self, output: OutputId, on: bool) {
        self.output_controls.queue_power(output, on);
    }

    fn release_power(&mut self, object: &ZwlrOutputPowerV1) {
        self.output_controls.release_power(object);
    }
}
