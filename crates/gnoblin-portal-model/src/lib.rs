//! Portal interface names, typed payloads and dialog models (design/rust-compositor.md, D19).
//!
//! This crate owns the interface names the compositor serves, the typed payload and
//! result of each interface a dialog renders and fills, and the dialog models
//! themselves. The models do not draw, read a clock, touch D-Bus or spawn a process.
//!
//! It must not know egui, any renderer or any engine crate: the compositor links it
//! for its negotiation types, and a UI crate draws the models. Its only dependencies
//! are `gnoblin-types` and `gnoblin-schema`.

mod dialog;
mod interface;
mod screen;

pub use dialog::{
    ARM_DELAY_MS, AppChoice, AppChooser, Confirmation, Consent, ConsentDecision, ConsentFocus, ConsentInput,
    ConsentTarget, DialogError, FileChooser, FileChooserMode, FileEntry, FileFilter, Key,
};
pub use interface::{Grant, Interface, UnknownInterface};
pub use screen::{Dialog, DirectoryListing, ScreenKey, Settled};
