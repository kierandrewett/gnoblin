//! `gnoblin-wayland-wp-presentation-time`: the sequence counter lives on the state.

use gnoblin_wayland_protocols::wp_presentation_time::{Host, WpPresentationTimeTables};

use crate::State;

impl Host for State {
    fn presentation_tables(&mut self) -> &mut WpPresentationTimeTables {
        &mut self.presentation_tables
    }
}
