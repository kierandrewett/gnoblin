//! The shared pieces of the portal dialogs (design/rust-compositor.md, D19).
//!
//! - [`canvas`]: the frame, theme, list and button helpers the dialog crates draw with.
//! - [`model`]: interface names, typed payloads, and the dialog models. Plain data,
//!   no drawing, no clock, no D-Bus. The compositor links this module for its
//!   negotiation types.
//! - [`theme`]: the design tokens every dialog draws from.
//! - [`paint`]: the headless CPU painter for egui output.
//! - [`widgets`]: painter helpers shared by the dialog screens.
//!
//! The crate names no engine crate. It may name egui and the primitive and schema
//! crates only; the UI crates depend on it, never the other way round.

pub mod canvas;
pub mod model;
pub mod paint;
pub mod theme;
pub mod widgets;
