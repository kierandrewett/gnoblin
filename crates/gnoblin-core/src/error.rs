//! Errors from the core, as a Rust type.
//!
//! Every table function returns an `i32`: zero is success and a negative code is a failure, with the text in the core's
//! per-thread last-error slot. [`CoreError`] is that pair, read once at the point of failure so a later call cannot
//! overwrite the text.

use std::fmt;

use gnoblin_core_abi as abi;

use crate::sys;

pub use crate::generated::ErrorKind;

impl ErrorKind {
    /// The short text `Display` puts before the core's own. Exhaustive on purpose: an `E_*` code added to the ABI adds a
    /// variant to the generated enum (`cargo xtask gen-sdks`), and this match is where the SDK has to say what it means.
    fn describe(self) -> &'static str {
        match self {
            ErrorKind::Failed => "failed",
            ErrorKind::Panic => "panic contained",
            ErrorKind::Unsupported => "not supported by the installed core",
            ErrorKind::Abi => "incompatible ABI",
            ErrorKind::Load => "load failed",
            ErrorKind::Stale => "stale handle",
            ErrorKind::Retired => "generation retired",
            ErrorKind::NoSuchNode => "no such node",
            ErrorKind::NoSuchMember => "no such member",
            ErrorKind::Type => "wrong value type",
            ErrorKind::Refused => "refused",
            ErrorKind::Thread => "called from the wrong thread",
            ErrorKind::Busy => "busy",
            ErrorKind::Timeout => "timed out",
        }
    }
}

/// A failed call: what kind of failure, and the core's text for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreError {
    kind: ErrorKind,
    message: String,
}

impl CoreError {
    pub(crate) fn new(kind: ErrorKind, message: impl Into<String>) -> CoreError {
        CoreError {
            kind,
            message: message.into(),
        }
    }

    /// Reads the core's last-error text for `code`.
    pub(crate) fn from_code(code: i32) -> CoreError {
        CoreError::new(ErrorKind::from_code(code), last_error_text())
    }

    /// The kind of failure.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The core's text, which may be empty.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.message.is_empty() {
            f.write_str(self.kind.describe())
        } else {
            write!(f, "{}: {}", self.kind.describe(), self.message)
        }
    }
}

impl std::error::Error for CoreError {}

/// The result of a call into the core.
pub type Result<T, E = CoreError> = std::result::Result<T, E>;

/// Turns a table function's return code into a `Result`.
pub(crate) fn check(code: i32) -> Result<()> {
    if code >= abi::E_OK {
        Ok(())
    } else {
        Err(CoreError::from_code(code))
    }
}

/// The text of the last failure on this thread, empty when there is none.
#[allow(unsafe_code)]
pub(crate) fn last_error_text() -> String {
    let mut buffer = [0u8; 512];
    // SAFETY: `gnoblin_last_error` writes at most `cap` bytes, including the terminating NUL, into the buffer it is
    // given and returns the full length. The buffer is a live local array of exactly `cap` bytes.
    let full = unsafe { sys::gnoblin_last_error(buffer.as_mut_ptr(), buffer.len()) };
    let kept = full.min(buffer.len() - 1);
    String::from_utf8_lossy(&buffer[..kept]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_known_code_has_a_kind_and_unknown_codes_are_failures() {
        assert_eq!(ErrorKind::from_code(abi::E_RETIRED), ErrorKind::Retired);
        assert_eq!(ErrorKind::from_code(abi::E_ABI), ErrorKind::Abi);
        assert_eq!(ErrorKind::from_code(abi::E_ERR), ErrorKind::Failed);
        assert_eq!(ErrorKind::from_code(-999), ErrorKind::Failed);
    }

    #[test]
    fn display_joins_the_kind_and_the_core_text() {
        let error = CoreError::new(ErrorKind::Retired, "generation 3 has been retired");
        assert_eq!(error.to_string(), "generation retired: generation 3 has been retired");
        assert_eq!(CoreError::new(ErrorKind::Busy, "").to_string(), "busy");
    }
}
