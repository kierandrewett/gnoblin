//! Shared test helpers for Gnoblin's layers above the pure crates.
//!
//! The pieces here exist so that a parity target, an in-process fixture and a
//! crate test all describe a run the same way: a private runtime directory, one
//! clock that never reads the wall clock, and one recorder that knows how to
//! write a trace without ever writing a secret. Keeping them in one crate means
//! a change to how observations are shaped is a change in one place rather than
//! a difference between two harnesses.
//!
//! This crate is a development dependency only; `cargo xtask deps` fails if a
//! shipped crate declares it outside `[dev-dependencies]`.

/// A clock that moves only when a test moves it.
pub mod clock;
/// Records observations as JSON with secrets removed before they are stored.
pub mod recorder;
/// Private runtime directories for a test, removed when the test ends.
pub mod runtime;
/// Builds schema-checked property sets for the `gnoblin` window and other objects.
pub mod window;

/// The test clock, re-exported at the crate root.
pub use clock::Clock;
/// The trace recorder, re-exported at the crate root.
pub use recorder::Recorder;
/// The private runtime directory, re-exported at the crate root.
pub use runtime::RuntimeDir;
/// Fixture builders for object and window property sets, re-exported at the crate root.
pub use window::{object_fields, window_fields};
