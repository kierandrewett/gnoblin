//! `gnoblin-wayland-ext-foreign-toplevel-list-v1`: the list state lives on the state.

use smithay::wayland::foreign_toplevel_list::{ForeignToplevelListHandler, ForeignToplevelListState};

use crate::State;

impl ForeignToplevelListHandler for State {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.foreign_toplevel_list_state
    }
}
