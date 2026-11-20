//! `gnoblin-wayland-zwp-text-input-v3`: the seat, and what has been told and held, live on the state.

use std::collections::{HashMap, HashSet};

use gnoblin_wayland_protocols::zwp_text_input_v3 as text_input;
use smithay::input::Seat;
use smithay::reexports::wayland_protocols::wp::text_input::zv3::server::zwp_text_input_v3::{
    ContentHint, ContentPurpose,
};
use smithay::reexports::wayland_server::backend::ObjectId;
use smithay::wayland::text_input::TextInputActivation;

use crate::State;

impl text_input::Host for State {
    fn seat(&self) -> &Seat<Self> {
        &self.seat
    }

    fn announced_text_inputs(&mut self) -> &mut HashSet<ObjectId> {
        &mut self.announced_text_inputs
    }

    fn held_text_inputs(&mut self) -> &mut HashMap<ObjectId, text_input::HeldTextInput> {
        &mut self.held_text_inputs
    }
}

impl TextInputActivation for State {
    fn activated(&mut self, content_type: Option<(ContentHint, ContentPurpose)>) {
        text_input::activated(self, content_type);
    }

    fn deactivated(&mut self) {
        text_input::deactivated();
    }
}
