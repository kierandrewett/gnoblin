//! Why a read or a write could not be answered.
//!
//! Every variant is a refusal the adapter makes after the gate admitted the call, so
//! each one is a statement about the graph rather than about the caller: the objects
//! the adapter reads are published by the compositor, and a value it cannot read is a
//! disagreement between the schema and the publisher, not a caller's mistake. The
//! adapter reports it instead of guessing, because a monitor reported with a zero
//! width is a monitor a caller will try to configure.
//!
//! [`DisplayError`] also carries the two failures that belong to serving the name, so
//! a host has one error type to report rather than three.

use std::fmt;

use zbus::fdo;

/// A `DisplayConfig` call that could not be answered.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DisplayError {
    /// A property the schema declares is absent or is not the value the schema
    /// promises.
    Unreadable {
        /// The object the property belongs to, as the adapter names it.
        object: String,
        /// The property's schema name.
        property: String,
        /// What was wrong with it.
        reason: String,
    },
    /// Two outputs published the same connector name, so a caller's
    /// `ApplyMonitorsConfig` could not say which one it meant.
    DuplicateConnector {
        /// The name both outputs claimed.
        name: String,
    },
    /// The current mode names a mode the output does not list.
    UnknownMode {
        /// The connector.
        connector: String,
        /// The mode id that is not in the output's list.
        mode: String,
    },
    /// The configuration's serial is not the one the adapter last published.
    StaleSerial {
        /// The serial the caller sent.
        sent: u32,
        /// The serial the adapter holds.
        current: u32,
    },
    /// The configuration names a connector the graph does not publish.
    UnknownConnector {
        /// The connector the caller named.
        name: String,
    },
    /// The configuration names a mode the output does not support.
    UnsupportedMode {
        /// The connector.
        connector: String,
        /// The mode the caller named.
        mode: String,
    },
    /// The configuration names one connector twice.
    RepeatedConnector {
        /// The connector.
        name: String,
    },
    /// The configuration's method is not one of `verify`, `temporary` or
    /// `persistent`.
    UnknownMethod {
        /// The value the caller sent.
        method: u32,
    },
    /// The output object refused the apply, or the IPC failed before it answered.
    ApplyFailed {
        /// The connector whose apply failed.
        connector: String,
        /// What the compositor or the transport said.
        reason: String,
    },
    /// The IPC connection ended, so nothing more can be read.
    Link {
        /// What the transport reported.
        reason: String,
    },
    /// This adapter is already exported in this process.
    Exported,
    /// Another process owns the bus name.
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

impl fmt::Display for DisplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DisplayError::Unreadable {
                object,
                property,
                reason,
            } => write!(formatter, "`{object}.{property}` is unreadable: {reason}"),
            DisplayError::DuplicateConnector { name } => write!(
                formatter,
                "two outputs claim the connector `{name}`, so no caller could name one of them"
            ),
            DisplayError::UnknownMode { connector, mode } => {
                write!(
                    formatter,
                    "`{connector}` reports `{mode}` as its current mode but does not list it"
                )
            }
            DisplayError::StaleSerial { sent, current } => write!(
                formatter,
                "the configuration's serial {sent} is not the current serial {current}; read the state again"
            ),
            DisplayError::UnknownConnector { name } => {
                write!(formatter, "no output is called `{name}`")
            }
            DisplayError::UnsupportedMode { connector, mode } => {
                write!(formatter, "`{connector}` does not support the mode `{mode}`")
            }
            DisplayError::RepeatedConnector { name } => {
                write!(
                    formatter,
                    "`{name}` appears in more than one place in the configuration"
                )
            }
            DisplayError::UnknownMethod { method } => write!(
                formatter,
                "{method} is not a configuration method; 0 verifies, 1 is temporary and 2 is persistent"
            ),
            DisplayError::ApplyFailed { connector, reason } => {
                write!(formatter, "`{connector}` refused the configuration: {reason}")
            }
            DisplayError::Link { reason } => write!(formatter, "the IPC connection ended: {reason}"),
            DisplayError::Exported => write!(formatter, "this adapter is already exported"),
            DisplayError::NameTaken { name } => write!(formatter, "`{name}` is owned by another process"),
            DisplayError::Bus { reason } => write!(formatter, "the bus refused the export: {reason}"),
        }
    }
}

impl std::error::Error for DisplayError {}

impl From<gnoblin_client::ClientError> for DisplayError {
    fn from(error: gnoblin_client::ClientError) -> DisplayError {
        DisplayError::Link {
            reason: error.to_string(),
        }
    }
}

impl From<DisplayError> for fdo::Error {
    fn from(error: DisplayError) -> fdo::Error {
        // `Failed` rather than a custom name: the message carries the adapter's rule
        // ids, which is what a caller greps, and a private error name would be a
        // second vocabulary beside the support matrix.
        fdo::Error::Failed(error.to_string())
    }
}
