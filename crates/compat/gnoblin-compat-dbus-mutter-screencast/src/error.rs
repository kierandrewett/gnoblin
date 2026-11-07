//! Why serving the name or opening a session failed.
//!
//! The refusals a caller sees for a gate or matrix decision are `zbus::fdo::Error`s
//! built by the service, because they are the frame's rules and their text is the
//! matrix's. This type carries the two failures that belong to serving the name, so a
//! host has one error type to report rather than three.

use std::fmt;

/// A `ScreenCast` adapter that could not be served.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ScreenCastError {
    /// This adapter is already exported in this process.
    Exported,
    /// Another process owns the bus name, which is the ordinary case while Mutter runs
    /// beside this adapter.
    NameTaken {
        /// The name.
        name: String,
    },
    /// The bus refused the export.
    Bus {
        /// What zbus reported.
        reason: String,
    },
}

impl fmt::Display for ScreenCastError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScreenCastError::Exported => {
                formatter.write_str("this ScreenCast adapter is already exported in this process")
            }
            ScreenCastError::NameTaken { name } => write!(
                formatter,
                "another process owns `{name}`; the adapter does not queue behind it"
            ),
            ScreenCastError::Bus { reason } => write!(formatter, "the bus refused the export: {reason}"),
        }
    }
}

impl std::error::Error for ScreenCastError {}
