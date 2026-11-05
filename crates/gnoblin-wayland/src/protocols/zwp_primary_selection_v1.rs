//! `zwp_primary_selection_device_manager_v1` (XML `primary-selection-unstable-v1`).
//!
//! The primary selection is what a middle click pastes. Its state lives on `State`
//! beside the clipboard; the handler here answers Smithay's trait for it.

use smithay::wayland::selection::primary_selection::{PrimarySelectionHandler, PrimarySelectionState};

use crate::State;

impl PrimarySelectionHandler for State {
    fn primary_selection_state(&mut self) -> &mut PrimarySelectionState {
        &mut self.primary_selection_state
    }
}
