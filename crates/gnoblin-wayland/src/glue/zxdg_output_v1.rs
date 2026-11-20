//! `gnoblin-wayland-xdg-output`: the objects clients hold live on the state, so a layout change reaches them.

use gnoblin_wayland_protocols::xdg_output::{Host, XdgOutputs};

use crate::State;

impl Host for State {
    fn xdg_outputs(&mut self) -> &mut XdgOutputs {
        &mut self.xdg_outputs
    }
}
