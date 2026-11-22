//! Shared test helpers for Gnoblin's layers above the pure crates.
//!
//! The pieces here exist so that an in-process fixture and a crate test describe a run the same way: a private runtime
//! directory, one fake compositor behind the tree link, and the oracle fixtures. Keeping them in one crate means a change
//! to how a run is shaped is a change in one place rather than a difference between two harnesses.
//!
//! This crate is a development dependency only; `cargo xtask deps` fails if a
//! shipped crate declares it outside `[dev-dependencies]`.

/// The per-backend status table in `crates/gnoblin-backend/parity.md`, read so the conformance tests can hold it to the code.
pub mod backend_status;
/// An in-memory compositor behind the tree link, which every shell runtime's tests share.
pub mod compositor;
/// Golden fixtures recorded from the TypeScript std, for the native std port to match.
pub mod oracle;
/// Private runtime directories for a test, removed when the test ends.
pub mod runtime;
/// Builds schema-checked property sets for the `gnoblin` window and other objects.
pub mod window;

/// The private runtime directory, re-exported at the crate root.
pub use runtime::RuntimeDir;
/// Fixture builders for object and window property sets, re-exported at the crate root.
pub use window::{object_fields, window_fields};
