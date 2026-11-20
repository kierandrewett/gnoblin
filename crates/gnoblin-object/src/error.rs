//! A failed call: an `E_*` code of the ABI and the words that go with it.

use gnoblin_core_abi as abi;

/// A failure of an object-protocol call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fail {
    /// The negative `E_*` code the ABI returns for it.
    pub code: i32,
    /// What went wrong, in words, for the last-error text and the shell's log.
    pub message: String,
}

/// A result of an object-protocol call.
pub type Res<T> = Result<T, Fail>;

impl Fail {
    /// A failure with `code` and `message`.
    pub fn new(code: i32, message: impl Into<String>) -> Fail {
        Fail {
            code,
            message: message.into(),
        }
    }

    /// A failure with no more specific code.
    pub fn err(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_ERR, message)
    }

    /// A value of the wrong kind, or a call the class does not take.
    pub fn type_error(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_TYPE, message)
    }

    /// A handle of an object that is gone, or never was.
    pub fn stale(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_STALE, message)
    }

    /// A class or member the catalog does not have.
    pub fn no_such(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_NO_SUCH_MEMBER, message)
    }

    /// A request the object refuses.
    pub fn refused(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_REFUSED, message)
    }

    /// A member the library names in its catalog and does not implement yet.
    pub fn unsupported(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_UNSUPPORTED, message)
    }
}

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Fail {}
