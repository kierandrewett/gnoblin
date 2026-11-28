//! Failures of a std call: an `E_*` code of the ABI, the script API's code and the text that goes with them.
//!
//! Every table function returns the code and leaves the text in a per-thread slot that `gnoblin_std_last_error` reads, the way the
//! core's `gnoblin_last_error` does. The text starts with the symbolic name of the code (`E_REFUSED: ...`) because a function that
//! returns a handle carries no code of its own. A failure that reaches a script through the object protocol is converted to
//! `gnoblin_object::Fail` and keeps the [`ErrorCode`] it was made with.

use std::cell::RefCell;

use gnoblin_core::host::{CoreError, ErrorKind};
use gnoblin_core_abi as abi;
use gnoblin_object::ErrorCode;

/// A failed std call.
#[derive(Debug)]
pub(crate) struct Fail {
    pub(crate) code: i32,
    pub(crate) message: String,
    /// The script API's code. None is a bad argument (`E_TYPE`).
    reason: Option<ErrorCode>,
}

pub(crate) type Res<T, E = Fail> = Result<T, E>;

impl Fail {
    /// A failure with the ABI code `code`. Its [`ErrorCode`] is the one `code` says; a producer that knows better follows with
    /// [`Fail::with_code`].
    pub(crate) fn new(code: i32, message: impl Into<String>) -> Fail {
        Fail {
            code,
            message: message.into(),
            reason: ErrorCode::of_abi(code),
        }
    }

    /// A failure with no more specific code (`internal`).
    pub(crate) fn err(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_ERR, message)
    }

    /// A request the library refuses, for the reason `code` says (a workspace that is not added, a collection that cannot change).
    pub(crate) fn refused(code: ErrorCode, message: impl Into<String>) -> Fail {
        Fail::new(abi::E_REFUSED, message).with_code(code)
    }

    /// A handle the library does not know (`not_found`).
    pub(crate) fn stale(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_STALE, message)
    }

    /// An argument of the wrong shape: the script's mistake, which carries no code.
    pub(crate) fn type_error(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_TYPE, message)
    }

    /// A name that is not a member of what it was looked up in (`not_found`).
    pub(crate) fn no_such(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_NO_SUCH_MEMBER, message)
    }

    /// A single-holder name that is taken (`busy`).
    pub(crate) fn busy(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_BUSY, message)
    }

    /// This failure with the code `code`.
    #[must_use]
    pub(crate) fn with_code(mut self, code: ErrorCode) -> Fail {
        self.reason = Some(code);
        self
    }

    /// This failure with the words `message`, and the ABI code and the script API's code it had.
    #[must_use]
    pub(crate) fn with_message(mut self, message: impl Into<String>) -> Fail {
        self.message = message.into();
        self
    }
}

impl From<Fail> for gnoblin_object::Fail {
    fn from(fail: Fail) -> gnoblin_object::Fail {
        let converted = gnoblin_object::Fail::new(fail.code, fail.message);
        match fail.reason {
            Some(code) => converted.with_code(code),
            None => converted,
        }
    }
}

impl From<gnoblin_object::Fail> for Fail {
    fn from(fail: gnoblin_object::Fail) -> Fail {
        let reason = fail.reason();
        let converted = Fail::new(fail.code, fail.message);
        match reason {
            Some(code) => converted.with_code(code),
            None => converted,
        }
    }
}

impl From<CoreError> for Fail {
    fn from(error: CoreError) -> Fail {
        Fail::new(error.kind().code(), error.to_string())
    }
}

/// The symbolic name of an error code, for the start of the last-error text.
pub(crate) fn code_name(code: i32) -> &'static str {
    ErrorKind::from_code(code).symbol()
}

thread_local! {
    static LAST: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Records `fail` as this thread's last error and returns its code.
pub(crate) fn record(fail: &Fail) -> i32 {
    set_last(format!("{}: {}", code_name(fail.code), fail.message));
    fail.code
}

/// Sets this thread's last-error text.
pub(crate) fn set_last(text: String) {
    LAST.with(|last| *last.borrow_mut() = text);
}

/// This thread's last-error text.
pub(crate) fn last_text() -> String {
    LAST.with(|last| last.borrow().clone())
}
