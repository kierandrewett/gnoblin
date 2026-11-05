//! Why an introspection call could not be answered.
//!
//! Every variant is a statement about the graph: the window list is what the compositor
//! published, and a value this adapter cannot read is a disagreement between the schema and
//! its publisher rather than a caller's mistake. The adapter reports it instead of guessing,
//! because a window reported with no title is a window a picker shows as blank.
//!
//! [`IntrospectError`] also carries the two failures that belong to serving the name, so a
//! host has one error type to report.

use std::fmt;

use zbus::fdo;

/// An `Introspect` call that could not be answered.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum IntrospectError {
    /// A property the schema declares is absent or is not the value the schema promises.
    Unreadable {
        /// The object the property belongs to.
        object: String,
        /// The property's schema name.
        property: String,
        /// What was wrong with it.
        reason: String,
    },
    /// Two windows published the same identity, so a caller could not tell them apart.
    DuplicateWindow {
        /// The identity both windows claimed.
        id: u64,
    },
    /// The graph publishes no enabled output, so the screen has no size to report.
    NoScreen,
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

impl fmt::Display for IntrospectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntrospectError::Unreadable {
                object,
                property,
                reason,
            } => write!(formatter, "`{object}.{property}` is unreadable: {reason}"),
            IntrospectError::DuplicateWindow { id } => write!(
                formatter,
                "two windows claim the identity {id}, so a caller could not tell them apart"
            ),
            IntrospectError::NoScreen => {
                formatter.write_str("the graph publishes no enabled output, so the screen has no size to report")
            }
            IntrospectError::Link { reason } => write!(formatter, "the IPC connection ended: {reason}"),
            IntrospectError::Exported => write!(formatter, "this adapter is already exported"),
            IntrospectError::NameTaken { name } => write!(formatter, "`{name}` is owned by another process"),
            IntrospectError::Bus { reason } => write!(formatter, "the bus refused the export: {reason}"),
        }
    }
}

impl std::error::Error for IntrospectError {}

impl From<gnoblin_client::ClientError> for IntrospectError {
    fn from(error: gnoblin_client::ClientError) -> IntrospectError {
        IntrospectError::Link {
            reason: error.to_string(),
        }
    }
}

impl From<IntrospectError> for fdo::Error {
    fn from(error: IntrospectError) -> fdo::Error {
        // `Failed` rather than a custom name: the message carries the adapter's rule ids,
        // which is what a caller greps, and a private error name would be a second
        // vocabulary beside the support matrix.
        fdo::Error::Failed(error.to_string())
    }
}
