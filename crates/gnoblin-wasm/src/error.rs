//! What went wrong, in the two places it can.
//!
//! The split is the one the caller acts on: [`HostError`] means the host refused to
//! take a module at all (or could not subscribe it), so nothing about that module is
//! running; [`CallError`] means one entry into a module failed, and the host either
//! killed that module because of it or reports it and carries on. Every variant
//! names the module, because a compositor with a dozen modules and a log line that
//! says "trapped" is not diagnosable.

use std::fmt;
use std::path::PathBuf;

use gnoblin_client::{ClientError, ConnectError};

use crate::host::ModuleState;

/// The host would not (or could not) take the module.
#[derive(Debug)]
pub enum HostError {
    /// A module with that name is already loaded.
    Duplicate { module: String },
    /// No module with that name is loaded.
    Missing { module: String },
    /// The manifest names a capability or a member the generated package does not
    /// have.
    Unsupported { module: String, reason: String },
    /// The bytes are not a component this host can compile.
    Compile { module: String, reason: String },
    /// The component compiled but could not be linked or instantiated: it imports
    /// something this host does not provide, or its instantiation exhausted its
    /// budget.
    Link { module: String, reason: String },
    /// The module could not be read from disk.
    Read { path: PathBuf, reason: String },
    /// The connection to the compositor was refused.
    Connect { module: String, source: ConnectError },
    /// The subscription to the object graph failed.
    Subscribe { module: String, source: ClientError },
}

impl fmt::Display for HostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::Duplicate { module } => write!(formatter, "module `{module}` is already loaded"),
            HostError::Missing { module } => write!(formatter, "no module `{module}` is loaded"),
            HostError::Unsupported { module, reason } => write!(formatter, "module `{module}`: {reason}"),
            HostError::Compile { module, reason } => {
                write!(
                    formatter,
                    "module `{module}` is not a component this host can run: {reason}"
                )
            }
            HostError::Link { module, reason } => write!(formatter, "module `{module}` cannot be linked: {reason}"),
            HostError::Read { path, reason } => write!(formatter, "cannot read {}: {reason}", path.display()),
            HostError::Connect { module, source } => {
                write!(formatter, "module `{module}` could not connect: {source}")
            }
            HostError::Subscribe { module, source } => {
                write!(formatter, "module `{module}` could not subscribe: {source}")
            }
        }
    }
}

impl std::error::Error for HostError {}

/// One entry into a module failed.
#[derive(Debug)]
pub enum CallError {
    /// No module with that name is loaded.
    Missing { module: String },
    /// The module is loaded but not in a state that accepts this call.
    NotRunning { module: String, state: ModuleState },
    /// The module used its whole budget without returning.
    ///
    /// The module has been killed: a call that did not finish cannot be resumed, and
    /// leaving it half-run would mean the next call sees a store in an unknown
    /// state.
    Deadline { module: String, fuel: u64 },
    /// The module trapped. It has been killed for the same reason as a deadline.
    Trapped { module: String, reason: String },
    /// The IPC client failed while the module was running.
    Client { module: String, source: ClientError },
}

impl CallError {
    /// The module the failure is about.
    pub fn module(&self) -> &str {
        match self {
            CallError::Missing { module }
            | CallError::NotRunning { module, .. }
            | CallError::Deadline { module, .. }
            | CallError::Trapped { module, .. }
            | CallError::Client { module, .. } => module,
        }
    }
}

impl fmt::Display for CallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallError::Missing { module } => write!(formatter, "no module `{module}` is loaded"),
            CallError::NotRunning { module, state } => {
                write!(
                    formatter,
                    "module `{module}` is {} and accepts no calls",
                    state.as_str()
                )
            }
            CallError::Deadline { module, fuel } => {
                write!(formatter, "module `{module}` used its {fuel} fuel and did not return")
            }
            CallError::Trapped { module, reason } => write!(formatter, "module `{module}` trapped: {reason}"),
            CallError::Client { module, source } => write!(formatter, "module `{module}`: {source}"),
        }
    }
}

impl std::error::Error for CallError {}
