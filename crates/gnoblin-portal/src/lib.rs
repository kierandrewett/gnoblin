//! The portal dialogs drawn by egui: one screen per dialog, each a thin wrapper
//! over the dialog kind's UI crate.

mod screen;

/// The screen that hosts one dialog, and the mapping from dialog kind to its UI.
pub use screen::{PortalScreen, ui_for};
