//! The kebab-case error code the schema declares and the wire carries.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

/// A kebab-case error code.
///
/// Codes are open-ended on purpose: an older peer reports one it has never heard
/// of rather than dropping the failure, so this type validates the shape only,
/// not membership in a schema. The one place that decides what a valid code
/// looks like is [`is_error_code`], which the schema's own parser calls, so a
/// declaration and a wire value can never disagree about the spelling.
///
/// A code that a crate builds itself is a literal, and a literal gets
/// [`ErrorCode::from_static`]: it borrows and it refuses an invalid spelling at
/// compile time, so `const OBJECT_GONE: ErrorCode = ErrorCode::from_static("object-gone");`
/// needs no unwrap and no allocation. Only a code decoded from a peer allocates.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ErrorCode(Cow<'static, str>);

impl ErrorCode {
    /// A validated code from a string literal.
    ///
    /// Const-evaluable and borrowing. An invalid spelling is a compile-time
    /// error at a constant initialiser and a panic at run time, because a
    /// literal that is not a code is a programming error rather than a value
    /// that can be handled.
    ///
    /// # Panics
    ///
    /// Panics when `text` is not a kebab-case code.
    #[must_use]
    pub const fn from_static(text: &'static str) -> Self {
        if !is_error_code(text) {
            panic!(
                "an error code starts with a lowercase ASCII letter and continues with lowercase ASCII letters, digits or `-`"
            );
        }
        Self(Cow::Borrowed(text))
    }

    /// Validate a kebab-case code received as text.
    ///
    /// Returns [`None`] for a string that is not one, rather than repairing it:
    /// a code arrives from a peer or from a schema, and a repaired code would
    /// name a failure the other side does not know.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        if is_error_code(text) {
            Some(Self(Cow::Owned(text.to_owned())))
        } else {
            None
        }
    }

    /// The code as text, which is what the envelope carries.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ErrorCode {
    type Err = InvalidErrorCode;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text).ok_or_else(|| InvalidErrorCode { text: text.to_owned() })
    }
}

/// A string that is not a kebab-case error code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidErrorCode {
    text: String,
}

impl InvalidErrorCode {
    /// The text that was rejected, so a caller can quote it in its own message.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for InvalidErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "`{}` is not a kebab-case error code: it starts with a lowercase ASCII letter and continues with lowercase ASCII letters, digits or `-`",
            self.text
        )
    }
}

impl std::error::Error for InvalidErrorCode {}

/// Whether `text` is a kebab-case error code.
///
/// The rule is the schema's: one lowercase ASCII letter, then lowercase ASCII
/// letters, digits and hyphens. It deliberately does not require each hyphen to
/// separate a word, because the existing schema codes are what they are and a
/// stricter rule would refuse a code a current namespace declares.
///
/// Const-evaluable, so [`ErrorCode::from_static`] can check a literal before the
/// program runs.
#[must_use]
pub const fn is_error_code(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_lowercase() {
        return false;
    }
    let mut index = 1;
    while index < bytes.len() {
        let byte = bytes[index];
        if !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-') {
            return false;
        }
        index += 1;
    }
    true
}
