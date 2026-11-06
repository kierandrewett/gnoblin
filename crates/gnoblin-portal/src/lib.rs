//! The portal dialogs drawn by egui: one screen per dialog, each a thin wrapper
//! over the dialog kind's UI crate.

mod screen;

pub use gnoblin_portal_core::ui::Setup;
pub use screen::{PortalScreen, ui_for};
