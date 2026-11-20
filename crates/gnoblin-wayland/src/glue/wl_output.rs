//! `gnoblin-wayland-wl-output`: a client bound an output global.

use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::wayland::output::OutputHandler;

use crate::State;

impl OutputHandler for State {
    fn output_bound(&mut self, output: Output, wl_output: WlOutput) {
        gnoblin_wayland_protocols::wl_output::output_bound(&output, &wl_output);
    }
}
