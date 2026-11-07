//! The egui layer of the portal dialogs (design/rust-compositor.md, D19).
//!
//! - [`ui`]: the [`ui::DialogUi`] trait each dialog kind implements, the
//!   [`ui::DialogShell`] that runs egui frames for it, and the translation from
//!   compositor keys and pointers into egui input.
//! - [`paint`]: the headless CPU painter for egui output.
//!
//! The dialog models come from `gnoblin-portal-core`. Every dialog screen crate
//! (`gnoblin-portal-consent`, `-filechooser`, `-appchooser`) depends on this crate
//! and implements [`ui::DialogUi`]; the compositor depends on it only for
//! [`paint::Frame`]. This crate names egui and the core and no dialog crate.

/// Painting helpers that turn egui output into frame pixels.
pub mod paint;
/// The egui dialog shell and the trait every dialog kind implements.
pub mod ui;
