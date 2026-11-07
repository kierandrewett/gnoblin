//! The portal dialog models (design/rust-compositor.md, D19).
//!
//! - [`model`]: interface names, typed payloads, and the dialog models. Plain data,
//!   no drawing, no clock, no D-Bus. The compositor links this module for its
//!   negotiation types, and the dialog UI crates build their screens on it.
//!
//! The crate names no engine crate: no egui and no renderer. It names the primitive
//! and schema crates only. The egui shell and the `DialogUi` trait live in
//! `gnoblin-portal-ui`, which depends on this crate; the dialog screens depend on that.

/// The dialog models and their state, independent of any renderer.
pub mod model;
