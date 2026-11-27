//! A failed call: an `E_*` code of the ABI, the script API's string code, and the words that go with it.
//!
//! The ABI's integer says what kind of failure it was and is what every shell has always read. `E_REFUSED` and `E_ERR` stand for several
//! things the script API tells apart, so a failure also carries an [`ErrorCode`], the one set of `docs/shell-api/errors.md`, which a
//! library reports through `GnoblinObjectApi::last_error_code` and a client reads beside the integer.

use gnoblin_core_abi as abi;

/// Why a call failed, in the words of the script API. `docs/shell-api/errors.md` says what each one means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    /// An argument is invalid.
    InvalidArgument,
    /// The object or resource does not exist, or has ended.
    NotFound,
    /// The id, name or trigger is taken.
    AlreadyExists,
    /// The object is not in a state that allows the call, such as a detached one.
    InvalidState,
    /// The property or collection cannot be written.
    ReadOnly,
    /// The compositor or the machine cannot perform the operation.
    Unsupported,
    /// Session policy, logind, polkit or the compositor rejected the operation.
    Denied,
    /// Something exclusive is already running or in use.
    Busy,
    /// The caller stopped the operation.
    Cancelled,
    /// The operation reached its deadline.
    TimedOut,
    /// The compositor or a session service is not ready, or there is no seat.
    Unavailable,
    /// An unexpected error occurred.
    Internal,
    /// The configuration document refused a value, or a reload was refused.
    ConfigRejected,
}

impl ErrorCode {
    /// Every code, in the order `docs/shell-api/errors.md` lists them.
    pub const ALL: [ErrorCode; 13] = [
        ErrorCode::InvalidArgument,
        ErrorCode::NotFound,
        ErrorCode::AlreadyExists,
        ErrorCode::InvalidState,
        ErrorCode::ReadOnly,
        ErrorCode::Unsupported,
        ErrorCode::Denied,
        ErrorCode::Busy,
        ErrorCode::Cancelled,
        ErrorCode::TimedOut,
        ErrorCode::Unavailable,
        ErrorCode::Internal,
        ErrorCode::ConfigRejected,
    ];

    /// The snake_case spelling every script sees (`not_found`).
    pub const fn as_str(self) -> &'static str {
        match self {
            ErrorCode::InvalidArgument => "invalid_argument",
            ErrorCode::NotFound => "not_found",
            ErrorCode::AlreadyExists => "already_exists",
            ErrorCode::InvalidState => "invalid_state",
            ErrorCode::ReadOnly => "read_only",
            ErrorCode::Unsupported => "unsupported",
            ErrorCode::Denied => "denied",
            ErrorCode::Busy => "busy",
            ErrorCode::Cancelled => "cancelled",
            ErrorCode::TimedOut => "timed_out",
            ErrorCode::Unavailable => "unavailable",
            ErrorCode::Internal => "internal",
            ErrorCode::ConfigRejected => "config_rejected",
        }
    }

    /// The code that spells `text`, or none for a word the script API does not have.
    pub fn parse(text: &str) -> Option<ErrorCode> {
        ErrorCode::ALL.into_iter().find(|code| code.as_str() == text)
    }

    /// The code an `E_*` integer says by itself: what a library that reports no code of its own means. A bad argument (`E_TYPE`) has none,
    /// because the script API tells it apart by its class (`TypeError`) and not by a code. `E_REFUSED` is a refusal nothing more is known
    /// of, which is `denied` (the same reading the script API has always given a word it does not know).
    pub const fn of_abi(code: i32) -> Option<ErrorCode> {
        match code {
            abi::E_TYPE => None,
            abi::E_STALE | abi::E_NO_SUCH_NODE | abi::E_NO_SUCH_MEMBER => Some(ErrorCode::NotFound),
            abi::E_UNSUPPORTED | abi::E_ABI => Some(ErrorCode::Unsupported),
            abi::E_RETIRED | abi::E_THREAD => Some(ErrorCode::InvalidState),
            abi::E_REFUSED => Some(ErrorCode::Denied),
            abi::E_BUSY => Some(ErrorCode::Busy),
            abi::E_TIMEOUT => Some(ErrorCode::TimedOut),
            _ => Some(ErrorCode::Internal),
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A failure of an object-protocol call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fail {
    /// The negative `E_*` code the ABI returns for it. [`Fail::code`] is the script API's code for the same failure.
    pub code: i32,
    /// What went wrong, in words, for the last-error text and the shell's log.
    pub message: String,
    /// The script API's code. None is a bad argument (`E_TYPE`), which the script API raises as a `TypeError` with no code.
    reason: Option<ErrorCode>,
    /// The object and member the failed call named (`Client#7.x`), set by the caller that knows them.
    target: Option<String>,
}

/// A result of an object-protocol call.
pub type Res<T> = Result<T, Fail>;

impl Fail {
    /// A failure with the ABI code `code` and `message`. Its [`ErrorCode`] is the one `code` says ([`ErrorCode::of_abi`]); a producer
    /// that knows better follows with [`Fail::with_code`].
    pub fn new(code: i32, message: impl Into<String>) -> Fail {
        Fail {
            code,
            message: message.into(),
            reason: ErrorCode::of_abi(code),
            target: None,
        }
    }

    /// A failure of ours, with no more specific code: an unexpected state or a null pointer (`internal`).
    pub fn err(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_ERR, message)
    }

    /// A value of the wrong kind, or a call the class does not take: the script's mistake, which carries no code.
    pub fn type_error(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_TYPE, message)
    }

    /// A handle of an object that is gone, or never was (`not_found`).
    pub fn stale(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_STALE, message)
    }

    /// A class, a member or a name the catalog or the object does not have (`not_found`).
    pub fn no_such(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_NO_SUCH_MEMBER, message)
    }

    /// A request the object refuses, for the reason `code` says.
    pub fn refused(code: ErrorCode, message: impl Into<String>) -> Fail {
        Fail::new(abi::E_REFUSED, message).with_code(code)
    }

    /// A member the library names in its catalog and does not implement yet (`unsupported`).
    pub fn unsupported(message: impl Into<String>) -> Fail {
        Fail::new(abi::E_UNSUPPORTED, message)
    }

    /// A write to a property or a collection that cannot be written (`read_only`). The ABI code is `E_TYPE`, as `set` has always said it
    /// for a property no one may write, and a shell built before the codes still reads it so.
    pub fn read_only(message: impl Into<String>) -> Fail {
        Fail::type_error(message).with_code(ErrorCode::ReadOnly)
    }

    /// This failure with the code `code`.
    #[must_use]
    pub fn with_code(mut self, code: ErrorCode) -> Fail {
        self.reason = Some(code);
        self
    }

    /// This failure with the words `message`, and the ABI code, the script API's code and the target it had.
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Fail {
        self.message = message.into();
        self
    }

    /// This failure for the call that named `target` (`Client#7.x`): the object and member, built where they are known. The words lose
    /// the symbolic ABI code a last-error text starts with, and the target when the library's own words begin with it, because
    /// [`Display`](std::fmt::Display) puts the target first.
    #[must_use]
    pub fn at(mut self, target: impl Into<String>) -> Fail {
        let target = target.into();
        let words = self.text();
        let words = words
            .strip_prefix(&format!("{target}: "))
            .or_else(|| words.strip_prefix(&format!("{target}(): ")))
            .unwrap_or(words)
            .to_owned();
        self.message = words;
        self.target = Some(target);
        self
    }

    /// This failure with the script API's code a library reported, or the one its ABI code says when it reported none and the call
    /// was not a bad argument.
    #[must_use]
    pub(crate) fn with_reported(mut self, reported: Option<ErrorCode>) -> Fail {
        self.reason = reported.or(ErrorCode::of_abi(self.code));
        self
    }

    /// The words of this failure without the symbolic name of the ABI code (`E_STALE: `) a library's last-error text starts with.
    pub fn text(&self) -> &str {
        match self.message.split_once(": ") {
            Some((symbol, rest))
                if symbol.starts_with("E_") && symbol.bytes().all(|b| b.is_ascii_uppercase() || b == b'_') =>
            {
                rest
            }
            _ => &self.message,
        }
    }

    /// The script API's code for this failure. A bad argument has none in the script API (`TypeError`), and is `invalid_argument` here.
    pub fn code(&self) -> ErrorCode {
        self.reason.unwrap_or(ErrorCode::InvalidArgument)
    }

    /// The code the library reported for this failure; none for a bad argument, which a script runtime raises as its own `TypeError`.
    pub fn reason(&self) -> Option<ErrorCode> {
        self.reason
    }

    /// The object and member of the call that failed, when the caller named them.
    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    /// The log level for the line a library says when a callback of a shell failed with this failure: a notice for the shell's
    /// diagnostics (`LOG_NOTICE`, once per site), except when the callee ran out its budget (`E_TIMEOUT`). A runtime that stops a call
    /// reports it itself and counts it, so the library says it as a plain warning.
    pub fn callback_level(&self) -> u32 {
        if self.code == abi::E_TIMEOUT {
            abi::LOG_WARN
        } else {
            abi::LOG_NOTICE
        }
    }
}

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.target {
            Some(target) => write!(f, "{target}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for Fail {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_round_trips_through_its_spelling() {
        for code in ErrorCode::ALL {
            assert_eq!(ErrorCode::parse(code.as_str()), Some(code));
        }
        assert_eq!(ErrorCode::parse("kebab-case"), None);
        assert_eq!(ErrorCode::ALL.len(), 13);
    }

    #[test]
    fn an_abi_code_says_the_code_unless_it_is_a_bad_argument() {
        assert_eq!(ErrorCode::of_abi(abi::E_TYPE), None);
        assert_eq!(ErrorCode::of_abi(abi::E_STALE), Some(ErrorCode::NotFound));
        assert_eq!(ErrorCode::of_abi(abi::E_RETIRED), Some(ErrorCode::InvalidState));
        assert_eq!(ErrorCode::of_abi(abi::E_BUSY), Some(ErrorCode::Busy));
        assert_eq!(ErrorCode::of_abi(abi::E_ERR), Some(ErrorCode::Internal));
        assert_eq!(ErrorCode::of_abi(abi::E_REFUSED), Some(ErrorCode::Denied));
    }

    #[test]
    fn the_constructors_set_the_code_their_name_says() {
        assert_eq!(Fail::type_error("x").reason(), None);
        assert_eq!(Fail::type_error("x").code(), ErrorCode::InvalidArgument);
        assert_eq!(Fail::stale("x").code(), ErrorCode::NotFound);
        assert_eq!(Fail::no_such("x").code(), ErrorCode::NotFound);
        assert_eq!(Fail::unsupported("x").code(), ErrorCode::Unsupported);
        assert_eq!(Fail::err("x").code(), ErrorCode::Internal);
        let refused = Fail::refused(ErrorCode::AlreadyExists, "x");
        assert_eq!(
            (refused.code, refused.code()),
            (abi::E_REFUSED, ErrorCode::AlreadyExists)
        );
        let read_only = Fail::read_only("x");
        assert_eq!(
            (read_only.code, read_only.reason()),
            (abi::E_TYPE, Some(ErrorCode::ReadOnly))
        );
    }

    #[test]
    fn a_target_leads_the_text_and_a_replaced_message_keeps_the_code() {
        let fail = Fail::stale("the client is gone").at("Client#7.x");
        assert_eq!(fail.to_string(), "Client#7.x: the client is gone");
        assert_eq!(fail.target(), Some("Client#7.x"));
        let wrapped = fail.with_message("wrapped");
        assert_eq!(
            (wrapped.code, wrapped.code(), wrapped.target()),
            (abi::E_STALE, ErrorCode::NotFound, Some("Client#7.x"))
        );
    }
}
