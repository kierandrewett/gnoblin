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

pub mod clock;
pub mod recorder;
pub mod runtime;
pub mod window;

pub use clock::Clock;
pub use recorder::Recorder;
pub use runtime::RuntimeDir;
pub use window::{object_fields, window_fields};
