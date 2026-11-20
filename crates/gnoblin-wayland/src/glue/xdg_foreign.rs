//! `gnoblin-wayland-xdg-foreign`: the handle table lives on the state.

use gnoblin_wayland_protocols::xdg_foreign::{Host, XdgForeign};

use crate::State;

impl Host for State {
    fn xdg_foreign(&mut self) -> &mut XdgForeign {
        &mut self.xdg_foreign
    }
}
