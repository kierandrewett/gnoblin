//! The shared pieces of the portal dialogs (design/rust-compositor.md, D19).
//!
//! - [`model`]: interface names, typed payloads, and the dialog models. Plain data,
//!   no drawing, no clock, no D-Bus. The compositor links this module for its
//!   negotiation types.
//! - [`ui`]: the [`ui::DialogUi`] trait each dialog kind implements, and the
//!   [`ui::Shell`] that runs egui frames for it.
//! - [`paint`]: the headless CPU painter for egui output.
//!
//! The crate names no engine crate. It may name egui and the primitive and schema
//! crates only; the UI crates depend on it, never the other way round.

pub mod model;
pub mod paint;
pub mod ui;
