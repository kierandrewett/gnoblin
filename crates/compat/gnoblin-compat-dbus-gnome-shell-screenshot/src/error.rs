//! Why a screenshot call could not be answered.
//!
//! A refusal here is the adapter's whole answer, so the variants say *which* refusal it is: the
//! gate's (the mode, the lock) arrives as a
//! [`gnoblin_compat_core::Refusal`](gnoblin_compat_core::Refusal) before this type is reached,
//! and what is left is the graph's.
//!
//! [`ScreenshotError`] also carries the two failures that belong to serving the name, so a host
//! has one error type to report.

use std::fmt;

use zbus::fdo;

/// A `Screenshot` call that could not be answered.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ScreenshotError {
    /// The graph publishes no `lock` object, so the gate's embargo is unknown.
    NoLock,
    /// A property the schema declares is absent or is not the value the schema promises.
    Unreadable {
        /// The object the property belongs to.
        object: String,
        /// The property's schema name.
        property: String,
        /// What was wrong with it.
        reason: String,
    },
    /// The IPC connection ended, so the lock state can no longer be read.
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

impl fmt::Display for ScreenshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScreenshotError::NoLock => formatter.write_str(
                "the graph publishes no `lock` object, so the capture embargo is unknown rather than released",
            ),
            ScreenshotError::Unreadable {
                object,
                property,
                reason,
            } => write!(formatter, "`{object}.{property}` is unreadable: {reason}"),
            ScreenshotError::Link { reason } => write!(formatter, "the IPC connection ended: {reason}"),
            ScreenshotError::Exported => write!(formatter, "this adapter is already exported"),
            ScreenshotError::NameTaken { name } => write!(formatter, "`{name}` is owned by another process"),
            ScreenshotError::Bus { reason } => write!(formatter, "the bus refused the export: {reason}"),
        }
    }
}

impl std::error::Error for ScreenshotError {}

impl From<gnoblin_client::ClientError> for ScreenshotError {
    fn from(error: gnoblin_client::ClientError) -> ScreenshotError {
        ScreenshotError::Link {
            reason: error.to_string(),
        }
    }
}

impl From<ScreenshotError> for fdo::Error {
    fn from(error: ScreenshotError) -> fdo::Error {
        // `Failed` rather than a custom name: the message carries the adapter's rule ids, which
        // is what a caller greps, and a private error name would be a second vocabulary beside
        // the support matrix.
        fdo::Error::Failed(error.to_string())
    }
}
