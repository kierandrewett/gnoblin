//! Built-in portal UI state for the egui dialogs (design/rust-compositor.md, D19).
//!
//! Nothing here negotiates, queues or decides outcomes: that is the `portal` module
//! of `gnoblin-compositor`. This module holds the interface names, the typed
//! payload and result of each interface the dialogs render and fill, and the
//! dialog models themselves. Nothing here draws, reads a clock, touches D-Bus or
//! spawns a process; the overlay draws the models and feeds input back in.
//!
//! Depends only on `gnoblin-types` and `gnoblin-schema` (D19).

mod dialog;
mod interface;

pub use dialog::{
    ARM_DELAY_MS, AppChoice, AppChooser, Confirmation, Consent, ConsentDecision, ConsentFocus, ConsentInput,
    ConsentTarget, DialogError, FileChooser, FileChooserMode, FileEntry, FileFilter, Key,
};
pub use interface::{Grant, Interface, UnknownInterface};
