//! Length-prefixed framing.
//!
//! The prefix is four bytes, big-endian, and counts the body only. A fixed width
//! keeps the reader trivial and lets a reader refuse an implausible length before
//! it allocates; the body itself is a prost message (design/rust-compositor.md,
//! D16). Descriptors that belong to a frame travel beside it, not inside it: see
//! [`crate::transport`].

use std::io::{self, Read, Write};

use prost::Message as _;
use zeroize::Zeroizing;

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
    // The encoded body carries the secret, so it is wiped once it has been written.
    let body = Zeroizing::new(envelope.encode_to_vec());
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
///
/// A reader with a read timeout must not use this: a timeout in the middle of a frame drops
/// the bytes already read and the next call starts inside the frame. Keep one [`FrameAssembler`]
/// for the stream instead.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Option<Envelope>, CodecError> {
    FrameAssembler::default().read(reader)
}

/// Reads the frames of one stream and keeps a partial frame across a read that fails.
///
/// A role pumps its connection with a read timeout, and a frame is written as its prefix and
/// then its body, so a timeout can fall between the two or inside the body. The bytes read so
/// far stay here, and the next [`FrameAssembler::read`] goes on from them, so a timeout loses
/// nothing and the stream never loses its framing.
#[derive(Default)]
pub struct FrameAssembler {
    prefix: [u8; LENGTH_PREFIX_LEN],
    /// How many bytes of the prefix have arrived.
    prefix_filled: usize,
    /// The body being read, once its prefix is complete, and how many bytes of it have arrived.
    body: Option<(Zeroizing<Vec<u8>>, usize)>,
}

/// Shows how far a frame has come and never its bytes: a partial frame may be a secret.
impl std::fmt::Debug for FrameAssembler {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FrameAssembler")
            .field("prefix_filled", &self.prefix_filled)
            .field("body_filled", &self.body.as_ref().map(|(_, filled)| *filled))
            .finish_non_exhaustive()
    }
}

impl FrameAssembler {
    /// Read the next frame, or `None` when the peer closed the connection cleanly between frames.
    ///
    /// # Errors
    ///
    /// Any error `reader` gives, a timeout included, leaves the bytes already read here: call
    /// again to go on. A length over [`MAX_FRAME_LEN`], a frame cut short by the end of the
    /// stream and a body that does not decode are refusals of the stream itself.
    pub fn read<R: Read>(&mut self, reader: &mut R) -> Result<Option<Envelope>, CodecError> {
        while self.body.is_none() {
            match reader.read(&mut self.prefix[self.prefix_filled..]) {
                Ok(0) if self.prefix_filled == 0 => return Ok(None),
                Ok(0) => return Err(CodecError::Truncated),
                Ok(read) => self.prefix_filled += read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(CodecError::Io(error)),
            }
            if self.prefix_filled == LENGTH_PREFIX_LEN {
                let len = u32::from_be_bytes(self.prefix) as usize;
                if len > MAX_FRAME_LEN {
                    return Err(CodecError::TooLarge { len });
                }
                self.body = Some((Zeroizing::new(vec![0u8; len]), 0));
            }
        }
        while let Some((body, filled)) = self.body.as_mut() {
            if *filled == body.len() {
                break;
            }
            match reader.read(&mut body[*filled..]) {
                Ok(0) => return Err(CodecError::Truncated),
                Ok(read) => *filled += read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(CodecError::Io(error)),
            }
        }
        let Some((body, _)) = self.body.take() else {
            return Ok(None);
        };
        self.prefix_filled = 0;
        // The body is wiped when this function returns, so the decoded copy is the only one
        // that outlives it, and `Envelope::drop` wipes that one for a sensitive frame.
        let mut envelope = Envelope::decode(body.as_slice())?;
        // A peer cannot make a secret member look plain by sending zero flags.
        envelope.flag_schema_sensitivity();
        Ok(Some(envelope))
    }
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
            Envelope::request("gnoblin", "request.answer", 9, b"payload".to_vec()),
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
            namespace: "gnoblin".to_owned(),
            member: "request.answer".to_owned(),
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

/// What one read from a *privileged* stream produced.
///
/// The runtime and the services role are restartable, and a restarted child reaches the
/// compositor on the descriptor the guardian gave its role. That descriptor is one stream for
/// the session's life, so a new child's first frame is a handshake again. A handshake and an
/// envelope are told apart by their first four bytes — [`MAGIC`] against a length prefix — so
/// the two can share one stream without a length change to either.
#[derive(Debug)]
pub enum StreamFrame {
    /// A new session starts here.
    Handshake(crate::Handshake),
    /// One envelope of the session in progress.
    Envelope(crate::Envelope),
}

/// Read the next thing a privileged stream carries: a handshake or an envelope.
///
/// `Ok(None)` is a clean end: the peer closed between frames.
///
/// # Errors
///
/// The same refusals as [`read_frame`], plus a handshake of the wrong length.
pub fn read_any(reader: &mut impl std::io::BufRead) -> Result<Option<StreamFrame>, CodecError> {
    let mut prefix = [0u8; LENGTH_PREFIX_LEN];
    let mut filled = 0;
    while filled < prefix.len() {
        match std::io::Read::read(reader, &mut prefix[filled..]) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => return Err(CodecError::Truncated),
            Ok(read) => filled += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(CodecError::Io(error)),
        }
    }
    if prefix == crate::MAGIC {
        let mut rest = [0u8; crate::HANDSHAKE_LEN - LENGTH_PREFIX_LEN];
        match std::io::Read::read_exact(reader, &mut rest) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Err(CodecError::Truncated),
            Err(error) => return Err(CodecError::Io(error)),
        }
        let mut frame = [0u8; crate::HANDSHAKE_LEN];
        frame[..LENGTH_PREFIX_LEN].copy_from_slice(&prefix);
        frame[LENGTH_PREFIX_LEN..].copy_from_slice(&rest);
        return crate::Handshake::decode(&frame).map(|handshake| Some(StreamFrame::Handshake(handshake)));
    }
    let length = u32::from_be_bytes(prefix) as usize;
    if length > MAX_FRAME_LEN {
        return Err(CodecError::TooLarge { len: length });
    }
    let mut body = vec![0u8; length];
    std::io::Read::read_exact(reader, &mut body).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            CodecError::Truncated
        } else {
            CodecError::Io(error)
        }
    })?;
    use prost::Message as _;
    Ok(Some(StreamFrame::Envelope(crate::Envelope::decode(body.as_slice())?)))
}

#[cfg(test)]
mod any_tests {
    use super::{StreamFrame, read_any};
    use crate::{Envelope, Handshake, Version, write_frame};

    fn version() -> Version {
        Version::new(0, 2, 0)
    }

    #[test]
    fn a_privileged_stream_carries_a_second_handshake_between_sessions() {
        // The failure this prevents: a restarted child writes its hello and the compositor
        // reads it as a frame body, so the connection it needs is dropped instead of served.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&Handshake::hello(version()).encode());
        write_frame(
            &mut bytes,
            &Envelope::request("gnoblin", "object.subscribe", 1, Vec::new()),
        )
        .expect("a frame");
        bytes.extend_from_slice(&Handshake::hello(version()).encode());

        let mut reader = std::io::BufReader::new(&bytes[..]);
        let mut kinds = Vec::new();
        while let Some(frame) = read_any(&mut reader).expect("decodes") {
            kinds.push(match frame {
                StreamFrame::Handshake(handshake) => format!("{:?}", handshake.kind),
                StreamFrame::Envelope(envelope) => envelope.member.clone(),
            });
        }
        assert_eq!(kinds, vec!["Hello", "object.subscribe", "Hello"]);
    }

    #[test]
    fn a_closed_stream_ends_without_a_frame() {
        let mut reader = std::io::BufReader::new(&b""[..]);
        assert!(read_any(&mut reader).expect("a clean end").is_none());
    }
}

#[cfg(test)]
mod resume_tests {
    use std::io::{self, Read};

    use super::{FrameAssembler, write_frame};
    use crate::error::CodecError;
    use crate::{Envelope, Kind};

    /// A stream that hands out a few bytes, then times out, then a few more: what a socket with a read
    /// timeout does when a frame's prefix and body are written apart.
    struct Stalling {
        bytes: Vec<u8>,
        at: usize,
        /// Bytes served between two timeouts.
        burst: usize,
        served_in_burst: usize,
    }

    impl Read for Stalling {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.at == self.bytes.len() {
                return Ok(0);
            }
            if self.served_in_burst == self.burst {
                self.served_in_burst = 0;
                return Err(io::Error::new(io::ErrorKind::TimedOut, "the read timed out"));
            }
            let room = (self.burst - self.served_in_burst)
                .min(buffer.len())
                .min(self.bytes.len() - self.at);
            buffer[..room].copy_from_slice(&self.bytes[self.at..self.at + room]);
            self.at += room;
            self.served_in_burst += room;
            Ok(room)
        }
    }

    fn frames() -> Vec<Envelope> {
        vec![
            Envelope::event("gnoblin", "changes", vec![7u8; 300]),
            Envelope::request("gnoblin", "portal.ui", 2, b"a prompt".to_vec()),
            Envelope::reply("gnoblin", "portal.ui", 2, Vec::new()),
        ]
    }

    /// Reads every frame of `stream`, going on after each timeout as a pumping role does.
    fn read_all(stream: &mut Stalling) -> (Vec<Envelope>, usize) {
        let mut reader = FrameAssembler::default();
        let mut read = Vec::new();
        let mut timeouts = 0;
        loop {
            match reader.read(stream) {
                Ok(Some(envelope)) => read.push(envelope),
                Ok(None) => return (read, timeouts),
                Err(CodecError::Io(error)) if error.kind() == io::ErrorKind::TimedOut => timeouts += 1,
                Err(error) => panic!("the stream lost its framing: {error}"),
            }
        }
    }

    #[test]
    fn a_timeout_at_any_byte_of_a_frame_loses_nothing() {
        // The failure this prevents: the portal pumped its connection with a 20 ms read timeout, a timeout fell
        // between a frame's prefix and its body, the prefix was dropped, and the body was read as the next length.
        let mut bytes = Vec::new();
        for frame in frames() {
            write_frame(&mut bytes, &frame).expect("a frame");
        }
        for burst in 1..=bytes.len().min(40) {
            let mut stream = Stalling {
                bytes: bytes.clone(),
                at: 0,
                burst,
                served_in_burst: 0,
            };
            let (read, timeouts) = read_all(&mut stream);
            assert_eq!(read, frames(), "a timeout every {burst} bytes changed the frames");
            assert!(timeouts > 0 || burst >= bytes.len(), "burst {burst} never timed out");
        }
    }

    #[test]
    fn a_clean_end_between_frames_is_none_and_one_inside_a_frame_is_truncated() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &frames()[0]).expect("a frame");
        let mut whole = Stalling {
            bytes: bytes.clone(),
            at: 0,
            burst: usize::MAX,
            served_in_burst: 0,
        };
        let mut reader = FrameAssembler::default();
        assert!(matches!(reader.read(&mut whole), Ok(Some(envelope)) if matches!(envelope.kind, Some(Kind::Event(_)))));
        assert!(matches!(reader.read(&mut whole), Ok(None)));

        bytes.truncate(bytes.len() - 1);
        let mut cut = Stalling {
            bytes,
            at: 0,
            burst: usize::MAX,
            served_in_burst: 0,
        };
        assert!(matches!(
            FrameAssembler::default().read(&mut cut),
            Err(CodecError::Truncated)
        ));
    }
}
