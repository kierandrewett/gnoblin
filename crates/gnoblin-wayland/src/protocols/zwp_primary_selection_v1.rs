//! XML protocol `wp_primary_selection_unstable_v1` (`<protocol name="wp_primary_selection_unstable_v1">`); interfaces served: `zwp_primary_selection_device_manager_v1`.
//!
//! `zwp_primary_selection_device_manager_v1` (XML `primary-selection-unstable-v1`).
//!
//! The primary selection is what a middle click pastes. Its state lives on `State`
//! beside the clipboard; the handler here answers Smithay's trait for it.
//!
//! # Refusals and failure modes
//!
//! The primary selection is unprivileged, so every client may bind it. Its owner-gone
//! rule is the clipboard's: a selection is cleared when its owner goes. Guarded by
//! `tests/selection.rs` (`a_selection_does_not_outlive_its_owner`).

use smithay::wayland::selection::primary_selection::{PrimarySelectionHandler, PrimarySelectionState};

use crate::State;

impl PrimarySelectionHandler for State {
    fn primary_selection_state(&mut self) -> &mut PrimarySelectionState {
        &mut self.primary_selection_state
    }
}
