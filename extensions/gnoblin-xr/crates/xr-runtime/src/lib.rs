//! Finds the active OpenXR runtime and supervises its service process for
//! `gnoblin-xr`.
//!
//! The crate has three parts:
//!
//! - [`lookup`]: resolves the active runtime manifest using the OpenXR loader's
//!   discovery rules (see the module docs for the sources).
//! - [`service`]: detects the runtime service binaries (`monado-service`,
//!   `wivrn-server`) on `PATH`.
//! - [`supervisor`]: starts, watches, restarts and stops the runtime service
//!   child process behind a [`supervisor::Spawner`] trait, so tests use a fake
//!   child and never start a real process.

#[cfg(test)]
mod testing;

pub mod lookup;
pub mod service;
pub mod supervisor;

pub use lookup::{Environment, LookupError, ManifestError, Runtime, Source, find_active_runtime};
pub use service::{Service, ServiceKind, detect_service};
pub use supervisor::{ChildProcess, ChildState, ExitStatus, RuntimeManager, Spawner, Status, SystemSpawner};
