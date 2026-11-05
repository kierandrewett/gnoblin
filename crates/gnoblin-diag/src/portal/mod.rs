//! Built-in portal UI state for the egui dialogs (design/rust-compositor.md, D19).
//!
//! Nothing here negotiates, queues or decides outcomes: that is the `portal` module
//! of `gnoblin-compositor`. This module holds the interface names, the typed
//! payload and result of each interface the dialogs render and fill, and the
//! dialog models themselves. The models do not draw, read a clock, touch D-Bus or
//! spawn a process. [`PortalScreen`] draws one model with egui, headlessly; the overlay
//! feeds input back in.
//!
//! The models depend only on `gnoblin-types` and `gnoblin-schema` (D19); only
//! [`screen`] touches egui.

mod dialog;
mod interface;
mod screen;

pub use dialog::{
    ARM_DELAY_MS, AppChoice, AppChooser, Confirmation, Consent, ConsentDecision, ConsentFocus, ConsentInput,
    ConsentTarget, DialogError, FileChooser, FileChooserMode, FileEntry, FileFilter, Key,
};
pub use interface::{Grant, Interface, UnknownInterface};
pub use screen::{Dialog, DirectoryListing, Label, PortalScreen, ScreenKey, Settled, Setup, Theme};
