//! XML protocol `wayland` (`<protocol name="wayland">`); interfaces served: `wl_output`.
//!
//! `wl_output`.
//!
//! The globals are created by [`crate::State`] when the output layout places an
//! output, because an output exists there before any client can see it. This module
//! is the client-facing half: it counts the bindings so a hotplug can log who is
//! watching, and it is where an output's per-client state would live.

use smithay::output::Output;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::wayland::output::OutputHandler;
use tracing::debug;

use crate::State;

impl OutputHandler for State {
    fn output_bound(&mut self, output: Output, wl_output: WlOutput) {
        debug!(output = %output.name(), instance = ?wl_output.id(), "[output] bound by a client");
    }
}
