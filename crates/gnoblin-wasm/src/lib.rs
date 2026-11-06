//! WASM module host: WIT host API generated from the schema, capabilities and deadlines.

gnoblin_log::component!("wasm");

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
