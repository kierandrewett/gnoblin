//! egui screens for the built-in portal dialogs (design/rust-compositor.md, D19).
//!
//! The dialog models and interface names live in `gnoblin-portal-core` (its `model` module); this module
//! draws them. [`PortalScreen`] draws one model with egui, headlessly; the overlay
//! feeds input back in.

mod screen;

pub use screen::{Label, PortalScreen, Setup, Theme};
