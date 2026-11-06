//! WASM module host: loads extension modules, delivers them graph events and
//! enforces their capabilities and deadlines.
//!
//! A module is a WebAssembly component built against the WIT world generated from
//! the schema in [`bindings`]. [`ModuleHost`] owns the set of loaded modules and
//! drives their lifecycle: load, start, deliver events, stop and reload. Each
//! module runs in its own store with its own [`mirror`] of the object graph and its
//! own IPC connection, so a trap or hang in one module cannot corrupt another.
//! [`Manifest`] declares what a module may ask for ([`Grants`]) and how much work
//! each call may do ([`Budget`], [`ModuleLimits`]). A deadline is fuel: a call that
//! exhausts its fuel traps, the module is unloaded, and the compositor keeps running.

pub mod abi;
mod bindings;
mod error;
mod host;
mod log;
mod manifest;
mod mirror;

pub use error::{CallError, ModuleError};
pub use host::{ModuleHost, ModuleState};
pub use log::{Level, LogLine, LogSink, Sink};
pub use manifest::{Budget, Grants, Manifest, ModuleLimits};
