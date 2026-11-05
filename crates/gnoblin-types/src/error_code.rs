//! The kebab-case error code the schema declares and the wire carries.

use std::fmt;
use std::str::FromStr;

/// A kebab-case error code.
///
/// Codes are open-ended on purpose: an older peer reports one it has never heard
/// of rather than dropping the failure, so this type validates the shape only,
/// not membership in a schema. The one place that decides what a valid code
/// looks like is [`is_error_code`], which the schema's own parser calls, so a
/// declaration and a wire value can never disagree about the spelling.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ErrorCode(String);

impl ErrorCode {
    /// Validate a kebab-case code.
    ///
    /// Returns [`None`] for a string that is not one, rather than repairing it:
    /// a code arrives from a peer or from a schema, and a repaired code would
    /// name a failure the other side does not know.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        if is_error_code(text) {
            Some(Self(text.to_owned()))
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
        formatter.write_str(&self.0)
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
#[must_use]
pub fn is_error_code(text: &str) -> bool {
    let mut bytes = text.bytes();
    matches!(bytes.next(), Some(first) if first.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
