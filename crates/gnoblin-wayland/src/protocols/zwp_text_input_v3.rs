//! XML protocol `text_input_unstable_v3` (`<protocol name="text_input_unstable_v3">`); interfaces served: `zwp_text_input_manager_v3`.
//!
//! `zwp_text_input_manager_v3` and `zwp_text_input_v3` (XML `text-input-unstable-v3`).
//!
//! A client that wants to type in a language an input method serves creates a
//! `zwp_text_input_v3` and enables it while its surface has focus. Smithay carries the
//! conversation through `TextInputHandle`; this module owns the two things a compositor
//! must decide for it:
//!
//! - **Focus.** `enter` and `leave` are sent when the keyboard focus moves, because a text
//!   input that is not told it has focus never asks to be enabled.
//! - **The initial `enter`.** A text input created on a surface that already has focus is
//!   announced by [`announce_new_text_inputs`].
//!
//! # Failure modes
//!
//! 1. **A text input is never told it has focus.** Focus moves set the handle and send
//!    `enter`, so a client's `enable` is accepted rather than discarded:
//!    `text_input_enter_and_commit_round_trip` in `tests/protocols.rs`.
//! 2. **A committed string never reaches the client.** An input method's `commit_string`
//!    goes to the active text input of the focused surface, and only there: the same test
//!    proves the round trip end to end.

use std::collections::HashSet;

use gnoblin_log::debug;
use smithay::reexports::wayland_protocols::wp::text_input::zv3::server::zwp_text_input_v3::{
    ContentHint, ContentPurpose,
};
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::Resource;
use smithay::wayland::text_input::{TextInputActivation, TextInputManagerState};

use crate::State;

/// Creates the `zwp_text_input_manager_v3` global.
pub(crate) fn text_input_global(display: &DisplayHandle) -> TextInputManagerState {
    TextInputManagerState::new::<State>(display)
}

/// Tells a `zwp_text_input_v3` it has focus when it was created on a surface that already
/// had it.
///
/// `enter` normally follows the keyboard focus, and Smithay sends it for a newly created
/// text input only when an input method already exists. An ordinary session starts its
/// input method after the first window is focused, so a text input created then would wait
/// for a focus change that never comes and its `enable` would never be answered. This is
/// run after every dispatch, which is the only place a new text input can appear, and it
/// remembers what it told so an object hears `enter` once.
pub(crate) fn announce_new_text_inputs(state: &mut State) {
    use smithay::wayland::text_input::{TextInputHandle, TextInputSeat};
    let handle: TextInputHandle = state.seat.text_input().clone();
    let announced = std::mem::take(&mut state.announced_text_inputs);
    let mut fresh = Vec::new();
    let mut current = HashSet::new();
    handle.with_focused_text_input(|input, surface| {
        let id = input.id();
        if !announced.contains(&id) {
            fresh.push((input.clone(), surface.clone()));
        }
        current.insert(id);
    });
    // Outside the handle's lock: writing to a client is the client's own lock, and taking
    // it while holding this one is the shape of a dispatch deadlock.
    for (input, surface) in fresh {
        input.enter(&surface);
    }
    state.announced_text_inputs = current;
}

impl TextInputActivation for State {
    fn activated(&mut self, content_type: Option<(ContentHint, ContentPurpose)>) {
        debug!(
            hint = ?content_type.map(|(hint, _)| hint),
            "[text-input] a client enabled its text input"
        );
        // A client enables a text input only after it was told it has focus, and the sweep
        // after the dispatch that created it is what told it.
        announce_new_text_inputs(self);
    }

    fn deactivated(&mut self) {
        debug!("[text-input] the active text input was disabled");
    }
}
