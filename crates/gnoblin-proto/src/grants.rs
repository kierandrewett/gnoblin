//! The grants frame: which extension namespaces a connection may use.
//!
//! The compositor sends this frame immediately after every accept and before any other
//! frame, so a client always knows its extension grants before it sends a request
//! (design/rust-compositor.md, "IPC extensions"). The accept frame itself is unchanged; the
//! grants travel in their own length-prefixed frame so the fixed 20-byte handshake never
//! grows.
//!
//! Layout: a four-byte big-endian length of the body, then the body. The body is a
//! big-endian `u16` entry count, then per entry the namespace as a `u8`-length-prefixed
//! UTF-8 string followed by the grant's major, minor and patch as three big-endian `u32`s.
//! A connection with no grants sends a count of zero: the frame is still sent, so a
//! reader never has to guess whether it is coming.
//!
//! The limits are checked on both sides. The writer refuses to build a frame a reader would
//! refuse, and the reader refuses before it allocates, so a hostile length cannot reserve
//! memory.

use std::collections::BTreeSet;
use std::fmt;
use std::io::{self, Read, Write};

use gnoblin_types::Version;

use crate::error::CodecError;

/// Most grants one connection may hold.
pub const MAX_GRANTS: usize = 64;

/// Longest namespace, in bytes.
pub const MAX_NAMESPACE_LEN: usize = 128;

/// Bytes one grant's version takes: three big-endian `u32`s.
const VERSION_LEN: usize = 12;

/// Largest body a grants frame may carry: the count, then every entry at its longest.
///
/// A body longer than this is refused before it is read.
pub const MAX_GRANTS_BODY_LEN: usize = 2 + MAX_GRANTS * (1 + MAX_NAMESPACE_LEN + VERSION_LEN);

/// Size of the big-endian length prefix on a grants frame.
const LENGTH_PREFIX_LEN: usize = 4;

/// One namespace a connection may use, and the version it may use it at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    /// The extension namespace, such as `example.ext`.
    pub namespace: String,
    /// The version the namespace is granted at.
    pub version: Version,
}

/// Why a grants frame was refused, as a named error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrantsError {
    /// The body is longer than [`MAX_GRANTS_BODY_LEN`].
    TooLarge { len: usize },
    /// The frame names more entries than [`MAX_GRANTS`].
    TooManyEntries { count: usize },
    /// An entry's namespace is empty.
    EmptyNamespace,
    /// An entry's namespace is longer than [`MAX_NAMESPACE_LEN`].
    NamespaceTooLong { len: usize },
    /// An entry's namespace is not UTF-8.
    NamespaceNotUtf8,
    /// An entry's namespace holds a control character or a space.
    MalformedNamespace { namespace: String },
    /// Two entries name the same namespace.
    DuplicateNamespace { namespace: String },
    /// The body ends inside an entry, or before its count.
    Truncated,
    /// The body has bytes after its last entry.
    TrailingBytes { extra: usize },
}

impl fmt::Display for GrantsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GrantsError::TooLarge { len } => write!(
                formatter,
                "grants frame body of {len} bytes exceeds the {MAX_GRANTS_BODY_LEN} byte limit"
            ),
            GrantsError::TooManyEntries { count } => {
                write!(
                    formatter,
                    "grants frame names {count} entries, the limit is {MAX_GRANTS}"
                )
            }
            GrantsError::EmptyNamespace => formatter.write_str("grants frame names an empty namespace"),
            GrantsError::NamespaceTooLong { len } => write!(
                formatter,
                "grants frame names a {len} byte namespace, the limit is {MAX_NAMESPACE_LEN}"
            ),
            GrantsError::NamespaceNotUtf8 => formatter.write_str("grants frame names a namespace that is not UTF-8"),
            GrantsError::MalformedNamespace { namespace } => {
                write!(formatter, "grants frame names a malformed namespace {namespace:?}")
            }
            GrantsError::DuplicateNamespace { namespace } => {
                write!(formatter, "grants frame grants namespace {namespace:?} twice")
            }
            GrantsError::Truncated => formatter.write_str("grants frame ends inside an entry"),
            GrantsError::TrailingBytes { extra } => {
                write!(formatter, "grants frame has {extra} bytes after its last entry")
            }
        }
    }
}

impl std::error::Error for GrantsError {}

/// Check the rules every grant list must meet, on either side of the wire.
fn validate(grants: &[Grant]) -> Result<(), GrantsError> {
    if grants.len() > MAX_GRANTS {
        return Err(GrantsError::TooManyEntries { count: grants.len() });
    }
    let mut seen = BTreeSet::new();
    for grant in grants {
        validate_namespace(&grant.namespace)?;
        if !seen.insert(grant.namespace.as_str()) {
            return Err(GrantsError::DuplicateNamespace {
                namespace: grant.namespace.clone(),
            });
        }
    }
    Ok(())
}

/// A namespace is non-empty, at most [`MAX_NAMESPACE_LEN`] bytes, and free of control
/// characters and spaces.
fn validate_namespace(namespace: &str) -> Result<(), GrantsError> {
    if namespace.is_empty() {
        return Err(GrantsError::EmptyNamespace);
    }
    if namespace.len() > MAX_NAMESPACE_LEN {
        return Err(GrantsError::NamespaceTooLong { len: namespace.len() });
    }
    if namespace
        .chars()
        .any(|character| character.is_control() || character == ' ')
    {
        return Err(GrantsError::MalformedNamespace {
            namespace: namespace.to_owned(),
        });
    }
    Ok(())
}

/// Build the whole grants frame: the length prefix, then the body.
///
/// # Errors
///
/// A grant list the reader would refuse: too many entries, an empty, oversized or
/// malformed namespace, or the same namespace twice.
pub fn encode_grants(grants: &[Grant]) -> Result<Vec<u8>, GrantsError> {
    validate(grants)?;
    let mut body = Vec::with_capacity(MAX_GRANTS_BODY_LEN.min(2 + grants.len() * 32));
    // Checked by `validate`, so the count fits a u16 with room to spare.
    body.extend_from_slice(&(grants.len() as u16).to_be_bytes());
    for grant in grants {
        // Checked by `validate`: a namespace is at most 128 bytes, so its length fits a u8.
        body.push(grant.namespace.len() as u8);
        body.extend_from_slice(grant.namespace.as_bytes());
        body.extend_from_slice(&grant.version.major.to_be_bytes());
        body.extend_from_slice(&grant.version.minor.to_be_bytes());
        body.extend_from_slice(&grant.version.patch.to_be_bytes());
    }
    let len = u32::try_from(body.len()).map_err(|_| GrantsError::TooLarge { len: body.len() })?;
    let mut frame = Vec::with_capacity(LENGTH_PREFIX_LEN + body.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Read one grants body, the bytes after the length prefix.
///
/// # Errors
///
/// Every way a body can be malformed, each as a named [`GrantsError`].
pub fn decode_grants(body: &[u8]) -> Result<Vec<Grant>, GrantsError> {
    if body.len() > MAX_GRANTS_BODY_LEN {
        return Err(GrantsError::TooLarge { len: body.len() });
    }
    let mut cursor = Cursor { body, at: 0 };
    let count = usize::from(cursor.u16()?);
    if count > MAX_GRANTS {
        return Err(GrantsError::TooManyEntries { count });
    }
    let mut grants = Vec::with_capacity(count);
    let mut seen = BTreeSet::new();
    for _ in 0..count {
        let len = usize::from(cursor.u8()?);
        if len == 0 {
            return Err(GrantsError::EmptyNamespace);
        }
        let bytes = cursor.take(len)?;
        let namespace = std::str::from_utf8(bytes).map_err(|_| GrantsError::NamespaceNotUtf8)?;
        validate_namespace(namespace)?;
        if !seen.insert(namespace) {
            return Err(GrantsError::DuplicateNamespace {
                namespace: namespace.to_owned(),
            });
        }
        let version = Version::new(cursor.u32()?, cursor.u32()?, cursor.u32()?);
        grants.push(Grant {
            namespace: namespace.to_owned(),
            version,
        });
    }
    let extra = body.len() - cursor.at;
    if extra != 0 {
        return Err(GrantsError::TrailingBytes { extra });
    }
    Ok(grants)
}

/// Write one grants frame, length prefix included.
///
/// # Errors
///
/// [`CodecError::Grants`] for a grant list the reader would refuse, and
/// [`CodecError::Io`] when the socket fails.
pub fn write_grants<W: Write>(writer: &mut W, grants: &[Grant]) -> Result<(), CodecError> {
    let frame = encode_grants(grants)?;
    writer.write_all(&frame)?;
    writer.flush()?;
    Ok(())
}

/// Read one grants frame.
///
/// The length is checked before the body is allocated, so a hostile prefix cannot reserve
/// more than [`MAX_GRANTS_BODY_LEN`] bytes.
///
/// # Errors
///
/// [`CodecError::Grants`] for a malformed body, [`CodecError::Truncated`] when the stream
/// ends inside the frame, and [`CodecError::Io`] when the socket fails.
pub fn read_grants<R: Read>(reader: &mut R) -> Result<Vec<Grant>, CodecError> {
    let mut prefix = [0u8; LENGTH_PREFIX_LEN];
    reader.read_exact(&mut prefix).map_err(|error| match error.kind() {
        io::ErrorKind::UnexpectedEof => CodecError::Truncated,
        _ => CodecError::Io(error),
    })?;
    let len = u32::from_be_bytes(prefix) as usize;
    if len > MAX_GRANTS_BODY_LEN {
        return Err(CodecError::Grants(GrantsError::TooLarge { len }));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).map_err(|error| match error.kind() {
        io::ErrorKind::UnexpectedEof => CodecError::Truncated,
        _ => CodecError::Io(error),
    })?;
    Ok(decode_grants(&body)?)
}

/// A forward-only reader over a grants body that names the truncation it hits.
struct Cursor<'a> {
    body: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], GrantsError> {
        let end = self.at.checked_add(len).ok_or(GrantsError::Truncated)?;
        let bytes = self.body.get(self.at..end).ok_or(GrantsError::Truncated)?;
        self.at = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8, GrantsError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, GrantsError> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, GrantsError> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor as IoCursor;

    use gnoblin_types::Version;

    use super::{
        Grant, GrantsError, MAX_GRANTS, MAX_GRANTS_BODY_LEN, MAX_NAMESPACE_LEN, decode_grants, encode_grants,
        read_grants, write_grants,
    };
    use crate::CodecError;

    fn grant(namespace: &str) -> Grant {
        Grant {
            namespace: namespace.to_owned(),
            version: Version::new(0, 1, 0),
        }
    }

    /// The body of an encoded frame, without its four-byte prefix.
    fn body_of(frame: &[u8]) -> &[u8] {
        &frame[4..]
    }

    #[test]
    fn an_empty_list_is_a_frame_with_a_zero_count() {
        let frame = encode_grants(&[]).expect("encode");
        assert_eq!(frame, [0, 0, 0, 2, 0, 0]);
        assert_eq!(read_grants(&mut IoCursor::new(frame)).expect("read"), Vec::new());
    }

    #[test]
    fn round_trips_several_grants_with_their_versions() {
        let grants = vec![
            Grant {
                namespace: "example.ext".to_owned(),
                version: Version::new(0, 1, 0),
            },
            Grant {
                namespace: "vendor-x".to_owned(),
                version: Version::new(2, 30, 4),
            },
        ];
        let mut wire = Vec::new();
        write_grants(&mut wire, &grants).expect("write");
        assert_eq!(read_grants(&mut IoCursor::new(wire)).expect("read"), grants);
    }

    #[test]
    fn the_body_layout_is_count_then_length_prefixed_namespace_then_version() {
        let frame = encode_grants(&[grant("ab")]).expect("encode");
        assert_eq!(
            body_of(&frame),
            [
                0, 1, // count
                2, b'a', b'b', // namespace
                0, 0, 0, 0, // major
                0, 0, 0, 1, // minor
                0, 0, 0, 0, // patch
            ]
        );
        assert_eq!(&frame[..4], &((body_of(&frame).len()) as u32).to_be_bytes());
    }

    #[test]
    fn refuses_more_entries_than_the_limit_on_encode_and_decode() {
        let grants: Vec<Grant> = (0..=MAX_GRANTS).map(|index| grant(&format!("ns{index}"))).collect();
        assert_eq!(
            encode_grants(&grants),
            Err(GrantsError::TooManyEntries { count: MAX_GRANTS + 1 })
        );
        let mut body = vec![0, (MAX_GRANTS + 1) as u8];
        body.resize(MAX_GRANTS_BODY_LEN, 0);
        assert_eq!(
            decode_grants(&body),
            Err(GrantsError::TooManyEntries { count: MAX_GRANTS + 1 })
        );
    }

    #[test]
    fn accepts_a_namespace_at_the_limit_and_refuses_one_byte_more() {
        let at_limit = "a".repeat(MAX_NAMESPACE_LEN);
        let frame = encode_grants(&[grant(&at_limit)]).expect("a namespace at the limit");
        assert_eq!(
            read_grants(&mut IoCursor::new(frame)).expect("read")[0].namespace,
            at_limit
        );

        let over = "a".repeat(MAX_NAMESPACE_LEN + 1);
        assert_eq!(
            encode_grants(&[grant(&over)]),
            Err(GrantsError::NamespaceTooLong {
                len: MAX_NAMESPACE_LEN + 1
            })
        );
    }

    #[test]
    fn refuses_an_empty_namespace_on_both_sides() {
        assert_eq!(encode_grants(&[grant("")]), Err(GrantsError::EmptyNamespace));
        assert_eq!(decode_grants(&[0, 1, 0]), Err(GrantsError::EmptyNamespace));
    }

    #[test]
    fn refuses_a_namespace_with_a_space_or_control_character() {
        assert_eq!(
            encode_grants(&[grant("a b")]),
            Err(GrantsError::MalformedNamespace {
                namespace: "a b".to_owned()
            })
        );
        let body = [0, 1, 2, b'a', 0x07];
        assert_eq!(
            decode_grants(&body),
            Err(GrantsError::MalformedNamespace {
                namespace: "a\u{7}".to_owned()
            })
        );
    }

    #[test]
    fn refuses_a_namespace_that_is_not_utf8() {
        let body = [0, 1, 2, 0xff, 0xfe, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(decode_grants(&body), Err(GrantsError::NamespaceNotUtf8));
    }

    #[test]
    fn refuses_a_duplicate_namespace_on_both_sides() {
        assert_eq!(
            encode_grants(&[grant("dup"), grant("dup")]),
            Err(GrantsError::DuplicateNamespace {
                namespace: "dup".to_owned()
            })
        );
        let mut body = vec![0, 2];
        for _ in 0..2 {
            body.extend_from_slice(&[3, b'd', b'u', b'p']);
            body.extend_from_slice(&[0; 12]);
        }
        assert_eq!(
            decode_grants(&body),
            Err(GrantsError::DuplicateNamespace {
                namespace: "dup".to_owned()
            })
        );
    }

    #[test]
    fn refuses_a_body_that_ends_inside_an_entry_or_before_its_count() {
        assert_eq!(decode_grants(&[]), Err(GrantsError::Truncated));
        assert_eq!(decode_grants(&[0, 1]), Err(GrantsError::Truncated));
        // The name is declared as three bytes but only two follow.
        assert_eq!(decode_grants(&[0, 1, 3, b'a', b'b']), Err(GrantsError::Truncated));
        // The name is complete, the version is not.
        assert_eq!(decode_grants(&[0, 1, 1, b'a', 0, 0]), Err(GrantsError::Truncated));
    }

    #[test]
    fn refuses_bytes_after_the_last_entry() {
        let mut body = body_of(&encode_grants(&[grant("ab")]).expect("encode")).to_vec();
        body.extend_from_slice(&[9, 9]);
        assert_eq!(decode_grants(&body), Err(GrantsError::TrailingBytes { extra: 2 }));
    }

    #[test]
    fn refuses_an_oversized_body_before_reading_it() {
        // A prefix far past the limit: refused from the prefix alone, with nothing after it.
        let prefix = (MAX_GRANTS_BODY_LEN as u32 + 1).to_be_bytes();
        let error = read_grants(&mut IoCursor::new(prefix.to_vec())).expect_err("oversized");
        assert!(matches!(
            error,
            CodecError::Grants(GrantsError::TooLarge { len }) if len == MAX_GRANTS_BODY_LEN + 1
        ));
        let body = vec![0; MAX_GRANTS_BODY_LEN + 1];
        assert_eq!(
            decode_grants(&body),
            Err(GrantsError::TooLarge {
                len: MAX_GRANTS_BODY_LEN + 1
            })
        );
    }

    #[test]
    fn a_stream_that_ends_inside_the_frame_is_truncated() {
        let frame = encode_grants(&[grant("ab")]).expect("encode");
        let cut = &frame[..frame.len() - 1];
        assert!(matches!(
            read_grants(&mut IoCursor::new(cut.to_vec())),
            Err(CodecError::Truncated)
        ));
        assert!(matches!(
            read_grants(&mut IoCursor::new(vec![0, 0])),
            Err(CodecError::Truncated)
        ));
    }
}
