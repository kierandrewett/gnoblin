//! Errors from the std library, as a Rust type.
//!
//! A std call returns zero or a positive value for success and a negative `E_*` code for a failure, with the text in the library's
//! per-thread last-error slot. [`Error`] is that pair, read once at the point of failure so a later call cannot overwrite the text.
//! The kinds are the core's ([`ErrorKind`]), because the codes are the same.

use std::fmt;

use gnoblin_core::CoreError;
pub use gnoblin_core::ErrorKind;
use gnoblin_object::Fail;

/// A failed call: what kind of failure, and the library's text for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    message: String,
}

/// The result of a std call.
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    pub(crate) fn new(kind: ErrorKind, message: impl Into<String>) -> Error {
        Error {
            kind,
            message: message.into(),
        }
    }

    /// The kind of failure.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The library's text, which may be empty.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.message.is_empty() {
            write!(f, "{:?}", self.kind)
        } else {
            f.write_str(&self.message)
        }
    }
}

impl std::error::Error for Error {}

impl From<CoreError> for Error {
    fn from(error: CoreError) -> Error {
        Error::new(error.kind(), error.to_string())
    }
}

impl From<Fail> for Error {
    fn from(fail: Fail) -> Error {
        Error::new(ErrorKind::from_code(fail.code), fail.message)
    }
}
