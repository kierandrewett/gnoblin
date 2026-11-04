//! Length-prefixed framing.
//!
//! The prefix is four bytes, big-endian, and counts the body only. A fixed width
//! keeps the reader trivial and lets a reader refuse an implausible length before
//! it allocates; the body itself is a prost message (design/rust-compositor.md,
//! D16). Descriptors that belong to a frame travel beside it, not inside it: see
//! [`crate::transport`].

use std::io::{self, Read, Write};

use prost::Message as _;

use crate::Envelope;
use crate::error::CodecError;

/// Largest accepted body, in bytes.
///
/// A frame bigger than this is a bug or a hostile peer, never a real message: the
/// biggest thing the IPC carries is a frame export, and that travels as a
/// descriptor, not as bytes.
pub const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

/// Size of the big-endian length prefix.
pub const LENGTH_PREFIX_LEN: usize = 4;

/// Write one frame: length prefix, then the encoded envelope.
pub fn write_frame<W: Write>(writer: &mut W, envelope: &Envelope) -> Result<(), CodecError> {
    let body = envelope.encode_to_vec();
    if body.len() > MAX_FRAME_LEN {
        return Err(CodecError::TooLarge { len: body.len() });
    }
    // Checked above, so this conversion cannot fail in practice; reported rather
    // than unwrapped so a future limit change cannot panic.
    let len = u32::try_from(body.len()).map_err(|_| CodecError::TooLarge { len: body.len() })?;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

/// Read one frame, or `None` when the peer closed the connection cleanly between
/// frames.
///
/// End of stream in the middle of a frame is an error: the peer thought it sent
/// something and did not, which is a bug worth reporting rather than hiding.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Option<Envelope>, CodecError> {
    let mut prefix = [0u8; LENGTH_PREFIX_LEN];
    let mut filled = 0;
    while filled < LENGTH_PREFIX_LEN {
        match reader.read(&mut prefix[filled..]) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => return Err(CodecError::Truncated),
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(CodecError::Io(error)),
        }
    }

    let len = u32::from_be_bytes(prefix) as usize;
    if len > MAX_FRAME_LEN {
        return Err(CodecError::TooLarge { len });
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).map_err(|error| match error.kind() {
        io::ErrorKind::UnexpectedEof => CodecError::Truncated,
        _ => CodecError::Io(error),
    })?;
    let mut envelope = Envelope::decode(&body[..])?;
    // A peer cannot make a secret member look plain by sending zero flags.
    envelope.flag_schema_sensitivity();
    Ok(Some(envelope))
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use super::{MAX_FRAME_LEN, read_frame, write_frame};
    use crate::{Envelope, Kind};

    fn round_trip(envelope: Envelope) -> Envelope {
        let mut buffer = Vec::new();
        write_frame(&mut buffer, &envelope).expect("write");
        read_frame(&mut Cursor::new(buffer)).expect("read").expect("a frame")
    }

    #[test]
    fn round_trips_every_kind() {
        let frames = [
            Envelope::request("bsm.greeter", "auth_response", 9, b"payload".to_vec()),
            Envelope::reply("gnoblin", "focus", 9, b"ok".to_vec()),
            Envelope::event("gnoblin", "ready", b"ready".to_vec()),
            Envelope::error("gnoblin", "window", 9, "unknown-member", "no such member"),
        ];
        for frame in frames {
            assert_eq!(round_trip(frame.clone()), frame);
        }
    }

    #[test]
    fn a_peer_cannot_clear_the_sensitive_flag() {
        // A frame for a schema-secret member, built by hand with zero flags, as a
        // hostile or outdated peer would send it.
        let forged = Envelope {
            namespace: "bsm.greeter".to_owned(),
            member: "auth_response".to_owned(),
            request_id: 1,
            flags: 0,
            kind: Some(Kind::Request(crate::Request {
                payload: b"hunter2".to_vec(),
            })),
        };
        assert!(!forged.is_sensitive());
        let decoded = round_trip(forged);
        assert!(
            decoded.is_sensitive(),
            "the decoder re-applies the schema's flag, so nothing downstream sees a plain secret"
        );
    }

    #[test]
    fn round_trips_a_frame_with_no_kind() {
        let frame = Envelope::default();
        assert_eq!(round_trip(frame.clone()), frame);
    }

    #[test]
    fn writes_a_big_endian_length_prefix() {
        let envelope = Envelope::event("gnoblin", "session", Vec::new());
        let mut buffer = Vec::new();
        write_frame(&mut buffer, &envelope).expect("write");
        let declared = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;
        assert_eq!(declared, buffer.len() - 4);
    }

    #[test]
    fn a_clean_close_is_not_an_error() {
        assert!(read_frame(&mut Cursor::new(Vec::new())).expect("clean close").is_none());
    }

    #[test]
    fn a_truncated_body_is_rejected() {
        let mut buffer = Vec::new();
        write_frame(&mut buffer, &Envelope::event("gnoblin", "session", b"ready".to_vec())).expect("write");
        buffer.truncate(buffer.len() - 2);
        let error = read_frame(&mut Cursor::new(buffer)).expect_err("truncated");
        assert!(error.to_string().contains("middle of a frame"), "{error}");
    }

    #[test]
    fn a_truncated_prefix_is_rejected() {
        let error = read_frame(&mut Cursor::new(vec![0u8, 0, 1])).expect_err("truncated prefix");
        assert!(error.to_string().contains("middle of a frame"), "{error}");
    }

    #[test]
    fn an_oversize_length_is_refused_before_allocating() {
        let mut prefix = Vec::new();
        prefix.extend_from_slice(&u32::try_from(MAX_FRAME_LEN + 1).expect("length").to_be_bytes());
        let error = read_frame(&mut Cursor::new(prefix)).expect_err("too large");
        assert!(error.to_string().contains("exceeds"), "{error}");
        assert!(matches!(error, crate::CodecError::TooLarge { .. }));
    }

    #[test]
    fn a_body_that_is_not_an_envelope_is_rejected() {
        let mut buffer = Vec::new();
        buffer.extend_from_slice(&2u32.to_be_bytes());
        buffer.write_all(&[0xff, 0xff]).expect("write");
        let error = read_frame(&mut Cursor::new(buffer)).expect_err("bad body");
        assert!(matches!(error, crate::CodecError::Decode(_)), "{error}");
    }

    #[test]
    fn a_written_frame_has_no_side_effects_on_the_envelope() {
        let envelope = Envelope::event("gnoblin", "session", b"ready".to_vec());
        let before = envelope.clone();
        let mut buffer = Vec::new();
        write_frame(&mut buffer, &envelope).expect("write");
        assert_eq!(envelope, before);
        assert!(matches!(envelope.kind, Some(Kind::Event(_))));
    }
}
