//! Portal interface names, typed payloads and dialog models (design/rust-compositor.md, D19).
//!
//! This module owns the interface names the compositor serves, the typed payload and
//! result of each interface a dialog renders and fills, and the dialog models
//! themselves. The models do not draw, read a clock, touch D-Bus or spawn a process.
//!
//! Keep egui out of this module: the compositor links it for its negotiation types,
//! and the UI crates draw the models. Its only dependencies are `gnoblin-types` and
//! `gnoblin-schema`.

mod dialog;
mod interface;
mod screen;

/// Dialog models: the file chooser, the app chooser, consent and their types.
pub use dialog::{
    ARM_DELAY_MS, AppChoice, AppChooser, Confirmation, Consent, ConsentDecision, ConsentInput, ConsentTarget,
    DialogError, FileChooser, FileChooserMode, FileEntry, FileFilter,
};
/// Grant, interface and error types for the portal interfaces.
pub use interface::{Grant, Interface, UnknownInterface};
/// The dialog screen model, its listing and its settled state.
pub use screen::{Dialog, DirectoryListing, ScreenKey, Settled};
