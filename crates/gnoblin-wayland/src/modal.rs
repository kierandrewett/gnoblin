//! A modal overlay: an overlay drawn above one output's layers that takes the user's input
//! instead of the clients underneath it.
//!
//! While a modal stands, no key or pointer button reaches a client on its output. Each one
//! is kept as a [`ModalEvent`] for the compositor to take, and the compositor decides what
//! it means. While the session is locked the lock cover owns the output, so a stored modal
//! is neither drawn nor routed until the session unlocks. Screen captures leave the overlay
//! out, so a capture shows what the modal covers.

use std::fmt;

use gnoblin_render_frame::frame::Overlay;
use gnoblin_types::{KeyState, OutputId};

/// A modal overlay on one output.
#[derive(Clone, Debug)]
pub struct Modal {
    output: OutputId,
    overlay: Overlay,
    text: bool,
}

impl Modal {
    /// A modal drawn on `output`.
    #[must_use]
    pub fn new(output: OutputId, overlay: Overlay) -> Self {
        Self {
            output,
            overlay,
            text: false,
        }
    }

    /// This modal also takes the text each key press types, as [`ModalEvent::Text`], for a dialog that has an entry.
    /// Other modals never get it, so a key that is not a dialog key is not read as a character.
    #[must_use]
    pub fn with_text(mut self) -> Self {
        self.text = true;
        self
    }

    /// Whether the modal takes typed text.
    #[must_use]
    pub fn takes_text(&self) -> bool {
        self.text
    }

    /// The output the modal is drawn on.
    #[must_use]
    pub fn output(&self) -> OutputId {
        self.output
    }

    /// The overlay the modal draws.
    #[must_use]
    pub fn overlay(&self) -> &Overlay {
        &self.overlay
    }
}

/// One input event a modal took instead of the clients.
///
/// `keycode` is the evdev code the backend delivered, and `button` the evdev button
/// code, so a consumer names keys the way the configuration does.
#[derive(Clone, Copy, PartialEq)]
pub enum ModalEvent {
    /// A key edge.
    Key { keycode: u32, state: KeyState },
    /// The character a key press typed, after the layout and the modifiers. Sent only to a modal that
    /// [takes text](Modal::with_text), right after the press it belongs to.
    Text { character: char },
    /// A pointer button edge, at the pointer's position in layout coordinates.
    PointerButton {
        x: f64,
        y: f64,
        button: u32,
        state: KeyState,
    },
}

impl ModalEvent {
    /// Overwrites queued events with a key release that carries nothing: a queue that held typed characters must not
    /// keep them once they are routed.
    pub fn wipe_all(events: &mut [ModalEvent]) {
        events.fill(ModalEvent::Key {
            keycode: 0,
            state: KeyState::Released,
        });
    }
}

/// The typed character is what a passphrase is made of, so a log or a panic that formats the queue never shows it.
impl fmt::Debug for ModalEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModalEvent::Key { keycode, state } => f
                .debug_struct("Key")
                .field("keycode", keycode)
                .field("state", state)
                .finish(),
            ModalEvent::Text { .. } => f.write_str("Text { character: <redacted> }"),
            ModalEvent::PointerButton { x, y, button, state } => f
                .debug_struct("PointerButton")
                .field("x", x)
                .field("y", y)
                .field("button", button)
                .field("state", state)
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_character_never_shows_in_debug() {
        let shown = format!("{:?}", ModalEvent::Text { character: 'x' });
        assert!(shown.contains("redacted"), "{shown}");
        assert!(!shown.contains("'x'"), "{shown}");
    }
}
