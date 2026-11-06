//! The shared pieces of the portal dialogs (design/rust-compositor.md, D19).
//!
//! - [`model`]: interface names, typed payloads, and the dialog models. Plain data,
//!   no drawing, no clock, no D-Bus. The compositor links this module for its
//!   negotiation types.
//! - [`ui`]: the [`ui::DialogUi`] trait each dialog kind implements, and the
//!   [`ui::DialogShell`] that runs egui frames for it.
//! - [`paint`]: the headless CPU painter for egui output.
//!
//! The crate names no engine crate. It may name egui and the primitive and schema
//! crates only; the UI crates depend on it, never the other way round.

/// The dialog models and their state, independent of any renderer.
pub mod model;
/// Painting helpers that turn a model into egui draw calls.
pub mod paint;
/// The egui dialog UIs for each portal dialog kind.
pub mod ui;
