//! `gnoblin-wayland-zwlr-gamma-control-v1`: who holds each output's gamma is kept with the output controls.

use gnoblin_types::OutputId;
use gnoblin_wayland_protocols::zwlr_gamma_control_v1::Host;
use smithay::reexports::wayland_protocols_wlr::gamma_control::v1::server::zwlr_gamma_control_v1::ZwlrGammaControlV1;

use crate::State;

impl Host for State {
    fn gamma_size(&self, output: OutputId) -> Option<u32> {
        self.output_controls.gamma_size(output)
    }

    fn gamma_free(&self, output: OutputId) -> bool {
        self.output_controls.gamma_free(output)
    }

    fn gamma_holder_is(&self, output: OutputId, object: &ZwlrGammaControlV1) -> bool {
        self.output_controls.gamma_holder_is(output, object)
    }

    fn hold_gamma(&mut self, output: OutputId, object: ZwlrGammaControlV1) {
        self.output_controls.hold_gamma(output, object);
    }

    fn queue_gamma(&mut self, output: OutputId, tables: [Vec<u16>; 3]) {
        self.output_controls.queue_gamma(output, tables);
    }

    fn release_gamma(&mut self, object: &ZwlrGammaControlV1) {
        self.output_controls.release_gamma(object);
    }
}
