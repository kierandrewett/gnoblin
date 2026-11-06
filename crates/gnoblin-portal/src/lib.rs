//! The portal dialogs drawn by egui: one screen per dialog, each a thin wrapper
//! over the dialog kind's UI crate.

mod screen;

/// The dialog colour scheme and the frame size bounds, re-exported from the UI crate.
pub use gnoblin_portal_core::ui::{DialogScheme, Setup};
/// The screen that hosts one dialog, and the mapping from dialog kind to its UI.
pub use screen::{PortalScreen, ui_for};
