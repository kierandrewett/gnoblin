//! Why a frame could not be written, read or decoded.

use std::fmt;
use std::io;

/// A frame the codec refused.
///
/// These are coding errors or a broken peer, never a version disagreement: a
/// version disagreement is a [`crate::Mismatch`] reported from the handshake.
#[derive(Debug)]
pub enum CodecError {
    /// The socket failed.
    Io(io::Error),
    /// The sender declared a body longer than [`crate::MAX_FRAME_LEN`].
    ///
    /// Refused before allocating, so a corrupt length prefix cannot make the
    /// reader reserve gigabytes.
    TooLarge { len: usize },
    /// The stream ended in the middle of a frame.
    Truncated,
    /// The first frame was not [`crate::HANDSHAKE_LEN`] bytes long.
    BadHandshakeLength { got: usize },
    /// The first frame did not start with [`crate::MAGIC`].
    BadMagic { got: [u8; 4] },
    /// The first frame named a handshake kind this version does not define.
    UnknownHandshakeKind { kind: u8 },
    /// The body was not a valid message of the expected type.
    Decode(prost::DecodeError),
    /// A grants frame whose body the grants rules refuse.
    Grants(crate::grants::GrantsError),
}

impl fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodecError::Io(error) => write!(formatter, "socket error: {error}"),
            CodecError::TooLarge { len } => write!(
                formatter,
                "frame body of {len} bytes exceeds the {} byte limit",
                crate::MAX_FRAME_LEN
            ),
            CodecError::Truncated => formatter.write_str("the stream ended in the middle of a frame"),
            CodecError::BadHandshakeLength { got } => write!(
                formatter,
                "handshake frame is {got} bytes, expected {}",
                crate::HANDSHAKE_LEN
            ),
            CodecError::BadMagic { got } => write!(
                formatter,
                "handshake frame starts with {got:02x?}, expected {:02x?}",
                crate::MAGIC
            ),
            CodecError::UnknownHandshakeKind { kind } => {
                write!(formatter, "handshake frame names unknown kind {kind}")
            }
            CodecError::Decode(error) => write!(formatter, "message did not decode: {error}"),
            CodecError::Grants(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for CodecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CodecError::Io(error) => Some(error),
            CodecError::Decode(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for CodecError {
    fn from(error: io::Error) -> CodecError {
        CodecError::Io(error)
    }
}

impl From<prost::DecodeError> for CodecError {
    fn from(error: prost::DecodeError) -> CodecError {
        CodecError::Decode(error)
    }
}
impl From<crate::grants::GrantsError> for CodecError {
    fn from(error: crate::grants::GrantsError) -> CodecError {
        CodecError::Grants(error)
    }
}
