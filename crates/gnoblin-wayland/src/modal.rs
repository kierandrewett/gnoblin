//! A modal overlay: a surface drawn above one output's layers that takes the user's
//! input instead of the clients underneath it.
//!
//! While a modal is set, no key or pointer button reaches a client. Each one is kept
//! as a [`ModalEvent`] for the compositor to take, and the compositor decides what it
//! means. Screen captures leave the overlay out, so a capture shows what the modal
//! covers.

use gnoblin_backend::render::Overlay;
use gnoblin_types::{KeyState, OutputId};

/// A modal overlay on one output.
#[derive(Clone, Debug)]
pub struct Modal {
    output: OutputId,
    overlay: Overlay,
}

impl Modal {
    /// A modal drawn on `output`.
    #[must_use]
    pub fn new(output: OutputId, overlay: Overlay) -> Self {
        Self { output, overlay }
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
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ModalEvent {
    /// A key edge.
    Key { keycode: u32, state: KeyState },
    /// A pointer button edge, at the pointer's position in layout coordinates.
    PointerButton {
        x: f64,
        y: f64,
        button: u32,
        state: KeyState,
    },
}
