//! The socket under the client: a [`gnoblin_client::Transport`] over a Unix stream.
//!
//! `gnoblin-client` deliberately holds no socket: it says a connection arrives as a
//! [`gnoblin_client::Transport`] and leaves the socket to whoever owns one. This is
//! that socket for `gnoblinctl`, and the same type serves the tests' server thread,
//! because a handshake and a frame stream are the same in both directions.
//!
//! Descriptors are refused rather than dropped. `SCM_RIGHTS` needs `recvmsg` with a
//! control buffer, which is `unsafe` and which the workspace forbids outside a
//! crate that declares its own FFI shim; nothing `gnoblinctl` sends or receives
//! carries one, so refusing is honest instead of silently losing a descriptor the
//! peer thought it had passed.

use std::io::{self, BufReader, Read, Write};
use std::os::fd::{BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::Path;

use gnoblin_client::HandshakeError;
use gnoblin_proto::{CodecError, HANDSHAKE_LEN, Handshake, read_frame, write_frame};

/// One Unix stream that speaks the IPC.
#[derive(Debug)]
pub struct UnixTransport {
    /// The writing half. Cloned from the stream so the reader can buffer without
    /// swallowing bytes the writer owns.
    writer: UnixStream,
    /// The reading half, buffered so a length prefix and its body are one call.
    reader: BufReader<UnixStream>,
}

impl UnixTransport {
    /// Connect to the socket at `path`.
    pub fn connect(path: &Path) -> io::Result<UnixTransport> {
        UnixTransport::from_stream(UnixStream::connect(path)?)
    }

    /// Take a stream that is already connected.
    ///
    /// A server accepts one of these from its listener; a test uses it for the
    /// compositor's end of the pair.
    pub fn from_stream(stream: UnixStream) -> io::Result<UnixTransport> {
        let writer = stream.try_clone()?;
        Ok(UnixTransport {
            writer,
            reader: BufReader::new(stream),
        })
    }
}

impl gnoblin_proto::Transport for UnixTransport {
    fn send(&mut self, envelope: &gnoblin_proto::Envelope, fds: &[BorrowedFd<'_>]) -> Result<(), CodecError> {
        if !fds.is_empty() {
            return Err(CodecError::Io(io::Error::new(
                io::ErrorKind::Unsupported,
                "gnoblinctl does not pass file descriptors",
            )));
        }
        write_frame(&mut self.writer, envelope)
    }

    fn receive(&mut self, _fds: &mut Vec<OwnedFd>) -> Result<Option<gnoblin_proto::Envelope>, CodecError> {
        read_frame(&mut self.reader)
    }
}

impl gnoblin_client::Transport for UnixTransport {
    fn send_handshake(&mut self, frame: &Handshake) -> Result<(), HandshakeError> {
        self.writer
            .write_all(&frame.encode())
            .map_err(|error| HandshakeError::Codec(CodecError::Io(error)))?;
        self.writer
            .flush()
            .map_err(|error| HandshakeError::Codec(CodecError::Io(error)))?;
        Ok(())
    }

    fn receive_handshake(&mut self) -> Result<Handshake, HandshakeError> {
        let mut frame = [0u8; HANDSHAKE_LEN];
        // A peer that sent an envelope instead starts with a length prefix, and one
        // that closed says so; both are the handshake's own error rather than a
        // frame decode failure.
        let mut filled = 0;
        while filled < HANDSHAKE_LEN {
            match self.reader.read(&mut frame[filled..]) {
                Ok(0) => {
                    return Err(HandshakeError::Codec(CodecError::Io(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "the peer closed the connection before its handshake",
                    ))));
                }
                Ok(read) => filled += read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(HandshakeError::Codec(CodecError::Io(error))),
            }
        }
        Handshake::decode(&frame).map_err(HandshakeError::Codec)
    }
}
