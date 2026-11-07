//! Why an idle call could not be answered.
//!
//! Every variant is a statement about the graph or about the caller's own watches: the
//! adapter holds no idle clock of its own, so a missing `idle` object is a failure it
//! reports rather than a zero it invents, and a watch that belongs to somebody else is
//! refused because a caller must not be able to cancel another's idle policy.
//!
//! [`IdleError`] also carries the two failures that belong to serving the name, so a host
//! has one error type to report.

use std::fmt;

use zbus::fdo;

/// An `IdleMonitor` call that could not be answered.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum IdleError {
    /// The graph publishes no `idle` object, so nothing told the adapter how long the
    /// session has been idle.
    NoCounter,
    /// A property the schema declares is absent or is not the value the schema promises.
    Unreadable {
        /// The object the property belongs to.
        object: String,
        /// The property's schema name.
        property: String,
        /// What was wrong with it.
        reason: String,
    },
    /// The watch id is not one this adapter handed out.
    NoWatch {
        /// The id the caller sent.
        id: u32,
    },
    /// The watch belongs to another caller, so this one may not remove it.
    ForeignWatch {
        /// The id the caller sent.
        id: u32,
    },
    /// The IPC connection ended, so the counter can no longer be read.
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

impl fmt::Display for IdleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdleError::NoCounter => formatter
                .write_str("the graph publishes no `idle` object, so the idle time is unknown rather than zero"),
            IdleError::Unreadable {
                object,
                property,
                reason,
            } => write!(formatter, "`{object}.{property}` is unreadable: {reason}"),
            IdleError::NoWatch { id } => write!(formatter, "watch {id} was never added by this adapter"),
            IdleError::ForeignWatch { id } => write!(
                formatter,
                "watch {id} belongs to another caller, so this one cannot remove it"
            ),
            IdleError::Link { reason } => write!(formatter, "the IPC connection ended: {reason}"),
            IdleError::Exported => write!(formatter, "this adapter is already exported"),
            IdleError::NameTaken { name } => write!(formatter, "`{name}` is owned by another process"),
            IdleError::Bus { reason } => write!(formatter, "the bus refused the export: {reason}"),
        }
    }
}

impl std::error::Error for IdleError {}

impl From<gnoblin_client::ClientError> for IdleError {
    fn from(error: gnoblin_client::ClientError) -> IdleError {
        IdleError::Link {
            reason: error.to_string(),
        }
    }
}

impl From<IdleError> for fdo::Error {
    fn from(error: IdleError) -> fdo::Error {
        // `Failed` rather than a custom name: the message carries the adapter's rule ids,
        // which is what a caller greps, and a private error name would be a second
        // vocabulary beside the support matrix.
        fdo::Error::Failed(error.to_string())
    }
}
