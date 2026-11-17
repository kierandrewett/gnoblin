//! Failures of a std call: an `E_*` code of the ABI and the text that goes with it.
//!
//! Every table function returns the code and leaves the text in a per-thread slot that `gnoblin_std_last_error` reads, the way the
//! core's `gnoblin_last_error` does. The text starts with the symbolic name of the code (`E_REFUSED: ...`) because a function that
//! returns a handle carries no code of its own.

use std::cell::RefCell;

use gnoblin_core::{CoreError, ErrorKind};
use gnoblin_core_abi as abi;

/// A failed std call.
#[derive(Debug)]
pub(crate) struct Fail {
    pub(crate) code: i32,
    pub(crate) message: String,
}

pub(crate) type Res<T, E = Fail> = Result<T, E>;

impl Fail {
    pub(crate) fn new(code: i32, message: impl Into<String>) -> Fail {
        Fail {
            code,
            message: message.into(),
        }
    }

    /// A failure with no more specific code.
    pub(crate) fn err(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_ERR, message)
    }

    /// A request the library refuses (a workspace that is not added, a collection that cannot change).
    pub(crate) fn refused(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_REFUSED, message)
    }

    /// A handle the library does not know.
    pub(crate) fn stale(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_STALE, message)
    }

    /// An argument of the wrong shape.
    pub(crate) fn type_error(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_TYPE, message)
    }

    /// A name that is not a member of what it was looked up in.
    pub(crate) fn no_such(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_NO_SUCH_MEMBER, message)
    }

    /// A single-holder name that is taken.
    pub(crate) fn busy(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_BUSY, message)
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
