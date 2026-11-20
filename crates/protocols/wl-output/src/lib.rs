//! XML protocol `wayland` (`<protocol name="wayland">`); interfaces served: `wl_output`.
//!
//! `wl_output`.
//!
//! The globals are created by `crate::State` when the output layout places an
//! output, because an output exists there before any client can see it. This module
//! is the client-facing half: it counts the bindings so a hotplug can log who is
//! watching, and it is where an output's per-client state would live.

use smithay::output::Output;
use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::{GlobalDispatch, Resource};
use smithay::wayland::output::WlOutputData;

use gnoblin_wayland_api::{Privilege, ProtocolInfo, Registrar, ServedInterface};

gnoblin_log::logger!(log = "wayland");

/// What this protocol advertises.
pub const PROTOCOL: ProtocolInfo = ProtocolInfo {
    name: "wl-output",
    interfaces: &[ServedInterface::new("wl_output", 4)],
    privilege: Privilege::Core,
};

/// Creates the `wl_output` global of `output`. The layout calls it once for each output it places.
pub fn create_global<D>(registrar: &Registrar<'_>, output: &Output) -> GlobalId
where
    D: GlobalDispatch<WlOutput, WlOutputData> + 'static,
{
    output.create_global::<D>(registrar.display())
}

/// A client bound `wl_output` for `output`.
pub fn output_bound(output: &Output, wl_output: &WlOutput) {
    log::debug!(area = "output", output = %output.name(), instance = ?wl_output.id(), "bound by a client");
}
