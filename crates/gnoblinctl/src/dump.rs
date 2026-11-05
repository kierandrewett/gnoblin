//! `ipc dump`: decode a recorded or piped frame stream into readable lines.
//!
//! Design/rust-compositor.md, "One IPC": "`gnoblinctl ipc dump` decodes live traffic
//! for debugging", and "IPC extensions": "Logs, the parity recorder and
//! `gnoblinctl ipc dump` drop the payload of every `sensitive` frame, so none of
//! them needs the extension schema to keep a secret out."
//!
//! So this module decodes the envelope and nothing else. It never looks inside a
//! payload, which is exactly why a sensitive frame cannot leak through it: the only
//! bytes it would have printed are dropped by [`Envelope::redact`] before any line
//! is built, and the frame is marked as redacted rather than shown short.
//!
//! The first frame of a connection is the fixed-layout handshake, so a recorded
//! stream that starts at connection time begins with it. A handshake is told from a
//! length prefix by its magic: as a big-endian length, `GNBL` is larger than the
//! frame limit, so no valid prefix can be mistaken for one.

use std::fs::File;
use std::io::{self, BufReader, Read, Write};

use gnoblin_proto::{
    Envelope, Grant, HANDSHAKE_LEN, Handshake, HandshakeKind, Kind, MAGIC, MAX_FRAME_LEN, read_grants,
};
use prost::Message as _;

use crate::cli::DumpSource;

/// Decode every frame in `source` onto `out`.
pub(crate) fn run(source: &DumpSource, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    match dump(source, out) {
        Ok(()) => 0,
        Err(error) => {
            let _ = writeln!(err, "gnoblinctl: ipc dump: {error}");
            1
        }
    }
}

/// The whole stream, one line per frame.
fn dump(source: &DumpSource, out: &mut dyn Write) -> Result<(), String> {
    let stream: Box<dyn Read> = match source {
        DumpSource::Stdin => Box::new(io::stdin()),
        DumpSource::File(path) => {
            Box::new(File::open(path).map_err(|error| format!("cannot open {}: {error}", path.display()))?)
        }
    };
    let mut stream = BufReader::new(stream);
    let mut frames = 0;
    while let Some(prefix) = read_prefix(&mut stream)? {
        frames += 1;
        if prefix == MAGIC {
            let handshake = read_handshake(&mut stream, prefix)?;
            writeln!(out, "handshake {} {}", kind_text(handshake.kind), handshake.version)
                .map_err(|error| format!("cannot write the dump: {error}"))?;
            if handshake.kind == HandshakeKind::Accept {
                let grants =
                    read_grants(&mut stream).map_err(|error| format!("the grants frame did not decode: {error}"))?;
                write_grants_dump(out, &grants)?;
            }
            continue;
        }
        let envelope = read_envelope(&mut stream, prefix, frames)?;
        writeln!(out, "{}", line(&envelope)).map_err(|error| format!("cannot write the dump: {error}"))?;
    }
    Ok(())
}

/// The four bytes that start the next frame, or `None` at a clean end of stream.
fn read_prefix(stream: &mut impl Read) -> Result<Option<[u8; 4]>, String> {
    let mut prefix = [0u8; 4];
    let mut filled = 0;
    while filled < prefix.len() {
        match stream.read(&mut prefix[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(format!("cannot read the frame stream: {error}")),
        }
    }
    match filled {
        0 => Ok(None),
        4 => Ok(Some(prefix)),
        _ => Err(format!("the stream ended inside a length prefix ({filled} of 4 bytes)")),
    }
}

/// Fill `buffer`, reporting a stream that ends inside a frame.
fn read_exactly(stream: &mut impl Read, buffer: &mut [u8]) -> Result<(), String> {
    let mut filled = 0;
    while filled < buffer.len() {
        match stream.read(&mut buffer[filled..]) {
            Ok(0) => {
                return Err(format!(
                    "the stream ended inside a frame ({filled} of {} bytes)",
                    buffer.len()
                ));
            }
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(format!("cannot read the frame stream: {error}")),
        }
    }
    Ok(())
}

/// The fixed-layout handshake that starts a connection.
fn read_handshake(stream: &mut impl Read, prefix: [u8; 4]) -> Result<Handshake, String> {
    let mut frame = [0u8; HANDSHAKE_LEN];
    frame[..4].copy_from_slice(&prefix);
    read_exactly(stream, &mut frame[4..])?;
    Handshake::decode(&frame).map_err(|error| format!("the handshake frame did not decode: {error}"))
}

/// The grants frame after an accept: one line per granted namespace, or `grants none`.
fn write_grants_dump(out: &mut dyn Write, grants: &[Grant]) -> Result<(), String> {
    if grants.is_empty() {
        return writeln!(out, "grants none").map_err(|error| format!("cannot write the dump: {error}"));
    }
    for grant in grants {
        writeln!(out, "grant {} {}", grant.namespace, grant.version)
            .map_err(|error| format!("cannot write the dump: {error}"))?;
    }
    Ok(())
}

/// One length-prefixed envelope.
fn read_envelope(stream: &mut impl Read, prefix: [u8; 4], index: u64) -> Result<Envelope, String> {
    let len = u32::from_be_bytes(prefix) as usize;
    if len > MAX_FRAME_LEN {
        return Err(format!(
            "frame {index} declares {len} bytes, more than the {MAX_FRAME_LEN} byte limit"
        ));
    }
    let mut body = vec![0u8; len];
    read_exactly(stream, &mut body)?;
    Envelope::decode(body.as_slice()).map_err(|error| format!("frame {index} did not decode: {error}"))
}

/// One readable line for one frame, built only from the redacted envelope.
fn line(envelope: &Envelope) -> String {
    let redacted = envelope.redact();
    let kind = match &redacted.kind {
        Some(Kind::Request(_)) => "request",
        Some(Kind::Reply(_)) => "reply",
        Some(Kind::Event(_)) => "event",
        Some(Kind::Error(_)) => "error",
        None => "empty",
    };
    let detail = if envelope.is_sensitive() {
        // The schema flagged this member secret, so not even the error text or the
        // payload length is shown: a length is a fact about a secret.
        "[redacted]".to_owned()
    } else {
        match &redacted.kind {
            Some(Kind::Error(error)) => format!("{} {:?}", error.code, error.message),
            Some(_) => format!("{} bytes", redacted.payload().map_or(0, |payload| payload.len())),
            None => "no body".to_owned(),
        }
    };
    format!(
        "{kind:<8} {}.{} #{} {detail}",
        redacted.namespace, redacted.member, redacted.request_id
    )
}

/// The handshake kind as a word.
fn kind_text(kind: HandshakeKind) -> &'static str {
    match kind {
        HandshakeKind::Hello => "hello",
        HandshakeKind::Accept => "accept",
        HandshakeKind::Refuse => "refuse",
    }
}
