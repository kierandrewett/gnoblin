//! `gnoblin-wayland-zwp-primary-selection-v1`: the primary selection lives on the state beside the clipboard.

use smithay::wayland::selection::primary_selection::{PrimarySelectionHandler, PrimarySelectionState};

use crate::State;

impl PrimarySelectionHandler for State {
    fn primary_selection_state(&mut self) -> &mut PrimarySelectionState {
        &mut self.primary_selection_state
    }
}
