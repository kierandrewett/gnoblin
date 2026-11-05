//! The first frame on the socket: a fixed-layout handshake.
//!
//! A protobuf handshake would be the wrong shape for its job. If the two sides
//! disagree about the protocol, a message that has to decode before it can be
//! read might not decode at all, and the peer would learn nothing but "bad
//! message". A fixed layout is readable by any future version: magic, kind, then
//! three 32-bit version numbers (design/rust-compositor.md, "Versioning" and
//! D16).
//!
//! Both sides send their full version. The client sends [`HandshakeKind::Hello`]
//! and waits; the compositor answers [`HandshakeKind::Accept`] with its own
//! version, or [`HandshakeKind::Refuse`] with the version it does serve. In both
//! cases the client can build the error that names both versions.

use crate::error::CodecError;
use crate::version::{self, Mismatch};
use gnoblin_types::Version;

/// Magic that starts every handshake frame, so one can be told from an envelope.
pub const MAGIC: [u8; 4] = *b"GNBL";

/// Fixed handshake frame length: 4 magic, 1 kind, 3 reserved, 3 x 4 version.
pub const HANDSHAKE_LEN: usize = 20;

/// Which handshake frame this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandshakeKind {
    /// Client to compositor: "I speak this version."
    Hello = 1,
    /// Compositor to client: "I accept, and I speak this version."
    Accept = 2,
    /// Compositor to client: "I cannot serve your version; this is mine."
    Refuse = 3,
}

fn kind_from_byte(byte: u8) -> Option<HandshakeKind> {
    match byte {
        1 => Some(HandshakeKind::Hello),
        2 => Some(HandshakeKind::Accept),
        3 => Some(HandshakeKind::Refuse),
        _ => None,
    }
}

/// One fixed-layout handshake frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handshake {
    pub kind: HandshakeKind,
    /// The sender's full IPC version.
    pub version: Version,
}

impl Handshake {
    /// The client's opening frame.
    pub const fn hello(version: Version) -> Handshake {
        Handshake {
            kind: HandshakeKind::Hello,
            version,
        }
    }

    /// The compositor's agreement.
    pub const fn accept(version: Version) -> Handshake {
        Handshake {
            kind: HandshakeKind::Accept,
            version,
        }
    }

    /// The compositor's refusal. The version is the compositor's own, so the
    /// client can name both versions in the error it reports.
    pub const fn refuse(version: Version) -> Handshake {
        Handshake {
            kind: HandshakeKind::Refuse,
            version,
        }
    }

    /// The fixed 20-byte frame. Reserved bytes are written as zero and ignored on
    /// read, which leaves room for a future field without a length change.
    pub fn encode(&self) -> [u8; HANDSHAKE_LEN] {
        let mut frame = [0u8; HANDSHAKE_LEN];
        frame[0..4].copy_from_slice(&MAGIC);
        frame[4] = self.kind as u8;
        frame[8..12].copy_from_slice(&self.version.major.to_be_bytes());
        frame[12..16].copy_from_slice(&self.version.minor.to_be_bytes());
        frame[16..20].copy_from_slice(&self.version.patch.to_be_bytes());
        frame
    }

    /// Read a fixed handshake frame.
    pub fn decode(frame: &[u8]) -> Result<Handshake, CodecError> {
        if frame.len() != HANDSHAKE_LEN {
            return Err(CodecError::BadHandshakeLength { got: frame.len() });
        }
        let mut magic = [0u8; 4];
        magic.copy_from_slice(&frame[0..4]);
        if magic != MAGIC {
            return Err(CodecError::BadMagic { got: magic });
        }
        let kind = kind_from_byte(frame[4]).ok_or(CodecError::UnknownHandshakeKind { kind: frame[4] })?;
        Ok(Handshake {
            kind,
            version: Version::new(read_u32(frame, 8), read_u32(frame, 12), read_u32(frame, 16)),
        })
    }

    /// Answer a client hello.
    ///
    /// On a compatible pairing this returns the accept frame to send. On a
    /// refusal it returns the error that names both versions, and the caller
    /// sends [`Handshake::refuse`] with the compositor's own version, so the
    /// client can name both versions from the frame alone.
    pub fn respond(hello: &Handshake, server: Version) -> Result<Handshake, Mismatch> {
        version::check(server, hello.version)?;
        Ok(Handshake::accept(server))
    }
}

/// Read a big-endian 32-bit number at `at`, which callers only reach after the
/// length check.
fn read_u32(frame: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([frame[at], frame[at + 1], frame[at + 2], frame[at + 3]])
}

#[cfg(test)]
mod tests {
    use super::{HANDSHAKE_LEN, Handshake, HandshakeKind, MAGIC};
    use crate::CodecError;
    use gnoblin_types::Version;

    fn version(text: &str) -> Version {
        Version::parse(text).expect("version")
    }

    #[test]
    fn round_trips_every_kind() {
        for (kind, handshake) in [
            (HandshakeKind::Hello, Handshake::hello(version("0.2.0"))),
            (HandshakeKind::Accept, Handshake::accept(version("0.2.3"))),
            (HandshakeKind::Refuse, Handshake::refuse(version("1.0.0"))),
        ] {
            let frame = handshake.encode();
            assert_eq!(frame.len(), HANDSHAKE_LEN);
            assert_eq!(&frame[0..4], &MAGIC);
            assert_eq!(frame[4], kind as u8);
            assert_eq!(&frame[5..8], &[0, 0, 0], "reserved bytes stay zero");
            assert_eq!(Handshake::decode(&frame).expect("decode"), handshake);
        }
    }

    #[test]
    fn carries_large_version_parts() {
        let handshake = Handshake::accept(Version::new(u32::MAX, 0, 1));
        assert_eq!(Handshake::decode(&handshake.encode()).expect("decode"), handshake);
    }

    #[test]
    fn rejects_a_frame_of_the_wrong_length() {
        let error = Handshake::decode(&[0u8; HANDSHAKE_LEN - 1]).expect_err("short");
        assert!(
            matches!(error, CodecError::BadHandshakeLength { got } if got == HANDSHAKE_LEN - 1),
            "{error}"
        );
    }

    #[test]
    fn rejects_a_missing_magic() {
        let mut frame = Handshake::hello(version("0.2.0")).encode();
        frame[0] = b'X';
        let error = Handshake::decode(&frame).expect_err("magic");
        assert!(matches!(error, CodecError::BadMagic { .. }), "{error}");
        assert!(error.to_string().contains("expected"), "{error}");
    }

    #[test]
    fn rejects_an_unknown_kind() {
        let mut frame = Handshake::hello(version("0.2.0")).encode();
        frame[4] = 9;
        let error = Handshake::decode(&frame).expect_err("kind");
        assert!(
            matches!(error, CodecError::UnknownHandshakeKind { kind } if kind == 9),
            "{error}"
        );
    }

    #[test]
    fn accepts_a_compatible_client_and_names_both_versions_otherwise() {
        let hello = Handshake::hello(version("0.2.4"));
        let accepted = Handshake::respond(&hello, version("0.2.9")).expect("accepted");
        assert_eq!(accepted, Handshake::accept(version("0.2.9")));

        let mismatch = Handshake::respond(&hello, version("0.3.0")).expect_err("refused");
        let message = mismatch.to_string();
        assert!(message.contains("compositor IPC 0.3.0"), "{message}");
        assert!(message.contains("client IPC 0.2.4"), "{message}");
        // A refusal frame still carries the compositor's version, so a client can
        // explain the mismatch even when it only sees the frame.
        let refusal = Handshake::decode(&Handshake::refuse(mismatch.server).encode()).expect("decode");
        assert_eq!(refusal.kind, HandshakeKind::Refuse);
        assert_eq!(refusal.version, version("0.3.0"));
    }
}
