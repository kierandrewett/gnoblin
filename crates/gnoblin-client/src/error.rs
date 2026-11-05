//! Why the client, or the wire, refused something.

use std::fmt;

use gnoblin_proto::{CodecError, HandshakeKind, Mismatch};

/// Why a payload could not be decoded.
///
/// Every variant is a refusal of the whole frame rather than a partial decode: the
/// model's guarantee is that a subscriber's view converges, and a half-decoded diff
/// would silently break that.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    /// The payload ended inside a field.
    Truncated,
    /// The payload had bytes left after the member's fields.
    Trailing,
    /// A value tag this build does not know.
    Tag(u8),
    /// A length-prefixed string was not UTF-8.
    Utf8,
    /// An error code was not kebab-case.
    Code,
    /// A set of named values carried the same name twice.
    Duplicate,
    /// The frame's kind does not belong to its member: a reply where an event was
    /// expected, or the reverse.
    Kind,
}

impl fmt::Display for WireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::Truncated => formatter.write_str("the payload ended inside a field"),
            WireError::Trailing => formatter.write_str("the payload had bytes left after its fields"),
            WireError::Tag(tag) => write!(formatter, "unknown value tag {tag}"),
            WireError::Utf8 => formatter.write_str("a text field was not UTF-8"),
            WireError::Code => formatter.write_str("an error code was not kebab-case"),
            WireError::Duplicate => formatter.write_str("a set of named values repeated a name"),
            WireError::Kind => formatter.write_str("the frame kind does not belong to its member"),
        }
    }
}

impl std::error::Error for WireError {}

/// Why the handshake could not be exchanged.
#[derive(Debug)]
pub enum HandshakeError {
    /// The connection failed, or the handshake frame did not decode.
    Codec(CodecError),
    /// The peer sent an envelope where the fixed-layout handshake belonged.
    NotAHandshake,
    /// The peer sent a handshake that belongs to the other side.
    WrongKind {
        /// The kind this side had to receive.
        expected: HandshakeKind,
        /// The kind that arrived.
        got: HandshakeKind,
    },
}

impl fmt::Display for HandshakeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HandshakeError::Codec(error) => write!(formatter, "handshake failed: {error}"),
            HandshakeError::NotAHandshake => formatter.write_str("the first frame was an envelope, not a handshake"),
            HandshakeError::WrongKind { expected, got } => {
                write!(formatter, "expected handshake {expected:?}, received {got:?}")
            }
        }
    }
}

impl std::error::Error for HandshakeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HandshakeError::Codec(error) => Some(error),
            _ => None,
        }
    }
}

impl From<CodecError> for HandshakeError {
    fn from(error: CodecError) -> HandshakeError {
        HandshakeError::Codec(error)
    }
}

/// Why [`crate::Client::connect`] failed.
///
/// A version disagreement is reported as a [`Mismatch`] naming both versions, which
/// is what the fixed-layout handshake exists for: the client can always say which
/// two versions disagree.
#[derive(Debug)]
pub enum ConnectError {
    /// The handshake could not be exchanged.
    Handshake(HandshakeError),
    /// The compositor refuses this build, or answered with a version that does not
    /// serve it.
    Refused(Mismatch),
}

impl fmt::Display for ConnectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectError::Handshake(error) => write!(formatter, "{error}"),
            ConnectError::Refused(mismatch) => write!(formatter, "{mismatch}"),
        }
    }
}

impl std::error::Error for ConnectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConnectError::Handshake(error) => Some(error),
            ConnectError::Refused(mismatch) => Some(mismatch),
        }
    }
}

impl From<HandshakeError> for ConnectError {
    fn from(error: HandshakeError) -> ConnectError {
        ConnectError::Handshake(error)
    }
}

/// Why a client call failed.
#[derive(Debug)]
pub enum ClientError {
    /// The connection failed while sending or receiving.
    Codec(CodecError),
    /// The peer closed the connection between frames.
    Disconnected,
    /// The compositor refused the request with one of the schema's error codes.
    ///
    /// The code stays text: an older client reports a code this build has never
    /// heard of rather than dropping the failure.
    Refused {
        /// The member that was refused.
        member: String,
        /// The kebab-case code the compositor sent.
        code: String,
        /// Human-readable detail. Never carries a secret.
        message: String,
    },
    /// The connection had nothing to read before its read timeout.
    ///
    /// Not a failure: a caller that set a read timeout on the socket is asking whether
    /// a frame has arrived, and this is the answer "not yet". It is reported here
    /// rather than through the transport because only the client knows whether an
    /// outstanding reply makes the wait a real one.
    Idle,
    /// The peer sent a payload this build could not decode.
    Protocol {
        /// The member whose payload failed.
        member: String,
        /// What was wrong with it.
        reason: WireError,
    },
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::Codec(error) => write!(formatter, "{error}"),
            ClientError::Disconnected => formatter.write_str("the compositor closed the connection"),
            ClientError::Idle => formatter.write_str("the connection had nothing to deliver yet"),
            ClientError::Refused { member, code, message } => {
                write!(formatter, "`{member}` was refused with `{code}`: {message}")
            }
            ClientError::Protocol { member, reason } => {
                write!(
                    formatter,
                    "`{member}` carried a payload this build cannot read: {reason}"
                )
            }
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ClientError::Codec(error) => Some(error),
            _ => None,
        }
    }
}

impl From<CodecError> for ClientError {
    fn from(error: CodecError) -> ClientError {
        ClientError::Codec(error)
    }
}
