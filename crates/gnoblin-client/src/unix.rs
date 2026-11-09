//! The Unix socket under every client: where it is, and the transport over it.
//!
//! Design/rust-compositor.md, "One IPC": the socket is
//! `$XDG_RUNTIME_DIR/gnoblin/ipc.sock`, found through `GNOBLIN_IPC_SOCKET`. Both
//! the path and the stream belong here rather than in each client, because every
//! client of the IPC — `gnoblinctl`, the runtime, the services role, the portal,
//! the shells and the tests — has to find the same socket and speak the same
//! handshake on it. One copy is what keeps them from disagreeing.
//!
//! [`UnixTransport`] is deliberately descriptor-less: a frame and its
//! `SCM_RIGHTS` payload travel together only on a transport that can carry one,
//! and receiving ancillary data needs `recvmsg`, which the workspace confines to
//! a named FFI shim. A descriptor handed to this transport is refused instead of
//! silently dropped, so a caller whose frame needed one learns it here rather
//! than from a receiver that got a frame with no buffer.
//!
//! # Failure modes first
//!
//! 1. **A client looks in the wrong place.** The override wins over the runtime
//!    directory, an empty variable counts as unset, and an environment that names
//!    neither is an error naming both: `tests` in this module.
//! 2. **A descriptor is dropped on the floor.** `send` refuses a non-empty
//!    descriptor list with `Unsupported` rather than sending the frame without
//!    them: `a_descriptor_is_refused_rather_than_dropped`.
//! 3. **A peer closes between frames and the client treats it as an empty frame.**
//!    The envelope path reports `Ok(None)` (a clean end), and the handshake path
//!    reports an error, because a peer that closed before its handshake never
//!    started the protocol: `a_closed_socket_ends_the_frame_stream`.
//! 4. **A peer sends an envelope where a handshake belongs.** The handshake read
//!    fails on the magic rather than reading four bytes of length prefix as one:
//!    `an_envelope_is_not_a_handshake`.

use std::io::{self, BufReader, Read, Write};
use std::os::fd::{BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gnoblin_proto::{CodecError, Envelope, Grant, HANDSHAKE_LEN, Handshake, read_frame, read_grants, write_frame};

use crate::error::HandshakeError;
use crate::transport::Transport;

/// The variable that overrides the socket path.
pub const SOCKET_VARIABLE: &str = "GNOBLIN_IPC_SOCKET";

/// The directory variable the default path is built from.
pub const RUNTIME_VARIABLE: &str = "XDG_RUNTIME_DIR";

/// The socket path under `$XDG_RUNTIME_DIR`.
pub const DEFAULT_SOCKET: &str = "gnoblin/ipc.sock";

/// The socket path, from the two variables that decide it.
///
/// `env` is passed in so the precedence can be tested without a process-global
/// write, which is `unsafe` in this edition and would make the tests order
/// dependent.
///
/// # Errors
///
/// A message naming both variables when neither is set, or when the override is
/// empty and the runtime directory is unset.
pub fn socket_path(env: impl Fn(&str) -> Option<String>) -> Result<PathBuf, String> {
    if let Some(path) = env(SOCKET_VARIABLE).filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    match env(RUNTIME_VARIABLE).filter(|dir| !dir.is_empty()) {
        Some(dir) => Ok(Path::new(&dir).join(DEFAULT_SOCKET)),
        None => Err(format!(
            "no IPC socket: set {SOCKET_VARIABLE}, or {RUNTIME_VARIABLE} so that ${RUNTIME_VARIABLE}/{DEFAULT_SOCKET} can be found"
        )),
    }
}

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
    ///
    /// # Errors
    ///
    /// The connect failure as it is: a missing or refused socket, so the caller can
    /// name the path it tried.
    pub fn connect(path: &Path) -> io::Result<UnixTransport> {
        UnixTransport::from_stream(UnixStream::connect(path)?)
    }

    /// Set the deadline one read waits for, which is how a role pumps on a clock.
    ///
    /// A read that times out is [`crate::ClientError::Idle`]: nothing has arrived yet, and
    /// the caller wanted a chance to do something else.
    ///
    /// # Errors
    ///
    /// The socket's refusal, which on a Unix stream is only a closed connection.
    pub fn read_timeout(&self, timeout: Option<std::time::Duration>) -> io::Result<()> {
        self.reader.get_ref().set_read_timeout(timeout)
    }

    /// Take a stream that is already connected.
    ///
    /// A server accepts one of these from its listener; a test uses it for the
    /// compositor's end of the pair.
    ///
    /// # Errors
    ///
    /// The failure to duplicate the stream, which leaves the caller with the one it
    /// passed in.
    pub fn from_stream(stream: UnixStream) -> io::Result<UnixTransport> {
        let writer = stream.try_clone()?;
        Ok(UnixTransport {
            writer,
            reader: BufReader::new(stream),
        })
    }

    /// Give back the stream under the transport, for a caller that hands the socket to
    /// another reader once the handshake and grants are done.
    ///
    /// # Errors
    ///
    /// When the reader holds bytes past the grants, which the caller would lose. A
    /// session that has only handshaken never has any, because the compositor sends
    /// nothing until the client asks for something.
    pub fn into_stream(self) -> io::Result<UnixStream> {
        if !self.reader.buffer().is_empty() {
            return Err(io::Error::other("the session socket holds bytes past its grants"));
        }
        Ok(self.reader.into_inner())
    }
}

impl gnoblin_proto::Transport for UnixTransport {
    fn send(&mut self, envelope: &Envelope, fds: &[BorrowedFd<'_>]) -> Result<(), CodecError> {
        if !fds.is_empty() {
            return Err(CodecError::Io(io::Error::new(
                io::ErrorKind::Unsupported,
                "this transport does not pass file descriptors",
            )));
        }
        write_frame(&mut self.writer, envelope)
    }

    fn receive(&mut self, _fds: &mut Vec<OwnedFd>) -> Result<Option<Envelope>, CodecError> {
        read_frame(&mut self.reader)
    }
}

impl Transport for UnixTransport {
    fn send_handshake(&mut self, frame: &Handshake) -> Result<(), HandshakeError> {
        self.writer
            .write_all(&frame.encode())
            .map_err(|error| HandshakeError::Codec(CodecError::Io(error)))?;
        self.writer
            .flush()
            .map_err(|error| HandshakeError::Codec(CodecError::Io(error)))?;
        Ok(())
    }

    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> Result<(), crate::ClientError> {
        self.reader
            .get_ref()
            .set_read_timeout(timeout)
            .map_err(|error| crate::ClientError::Codec(CodecError::Io(error)))
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

    fn receive_grants(&mut self) -> Result<Vec<Grant>, HandshakeError> {
        read_grants(&mut self.reader).map_err(HandshakeError::Codec)
    }
}

#[cfg(test)]
mod tests {
    use std::os::fd::AsFd;

    use gnoblin_proto::{CodecError, Envelope, HANDSHAKE_LEN, Handshake, Kind, Transport as _, Version, write_frame};

    use super::{DEFAULT_SOCKET, RUNTIME_VARIABLE, SOCKET_VARIABLE, UnixTransport, socket_path};
    use crate::transport::Transport as _;

    fn environment(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(known, _)| known == name)
                .map(|(_, value)| value.clone())
        }
    }

    #[test]
    fn the_override_wins_over_the_runtime_directory() {
        let env = environment(&[(SOCKET_VARIABLE, "/tmp/ipc.sock"), (RUNTIME_VARIABLE, "/run/user/1000")]);
        assert_eq!(
            socket_path(env).expect("a path"),
            std::path::PathBuf::from("/tmp/ipc.sock")
        );
    }

    #[test]
    fn the_runtime_directory_is_the_default() {
        let env = environment(&[(RUNTIME_VARIABLE, "/run/user/1000")]);
        assert_eq!(
            socket_path(env).expect("a path"),
            std::path::Path::new("/run/user/1000").join(DEFAULT_SOCKET)
        );
    }

    #[test]
    fn an_empty_override_counts_as_unset() {
        let env = environment(&[(SOCKET_VARIABLE, ""), (RUNTIME_VARIABLE, "/run/user/1000")]);
        assert_eq!(
            socket_path(env).expect("a path"),
            std::path::Path::new("/run/user/1000").join(DEFAULT_SOCKET)
        );
    }

    #[test]
    fn an_environment_that_names_neither_says_both() {
        let message = socket_path(environment(&[])).expect_err("no socket");
        assert!(message.contains(SOCKET_VARIABLE), "{message}");
        assert!(message.contains(RUNTIME_VARIABLE), "{message}");
    }

    /// A connected pair: the far end a server would hold, and the transport.
    fn pair() -> (std::os::unix::net::UnixStream, UnixTransport) {
        let (near, far) = std::os::unix::net::UnixStream::pair().expect("a socket pair");
        (far, UnixTransport::from_stream(near).expect("a transport"))
    }

    #[test]
    fn a_version_travels_on_the_handshake() {
        let (mut far, mut client) = pair();
        client
            .send_handshake(&Handshake::hello(Version::new(0, 2, 0)))
            .expect("send");
        let mut frame = [0u8; HANDSHAKE_LEN];
        std::io::Read::read_exact(&mut far, &mut frame).expect("read");
        assert_eq!(
            Handshake::decode(&frame).expect("decode"),
            Handshake::hello(Version::new(0, 2, 0))
        );
    }

    #[test]
    fn a_descriptor_is_refused_rather_than_dropped() {
        let (_far, mut client) = pair();
        let file = std::fs::File::open("/dev/null").expect("open");
        let error = client
            .send(
                &Envelope::request("gnoblin", "object.subscribe", 1, Vec::new()),
                &[file.as_fd()],
            )
            .expect_err("refused");
        assert!(
            matches!(&error, CodecError::Io(io) if io.kind() == std::io::ErrorKind::Unsupported),
            "{error}"
        );
    }

    #[test]
    fn a_closed_socket_ends_the_frame_stream() {
        let (far, mut client) = pair();
        drop(far);
        assert_eq!(
            gnoblin_proto::Transport::receive(&mut client, &mut Vec::new()).expect("clean end"),
            None
        );
    }

    #[test]
    fn an_envelope_is_not_a_handshake() {
        let (mut far, mut client) = pair();
        let mut bytes = Vec::new();
        write_frame(
            &mut bytes,
            &Envelope {
                namespace: "gnoblin".to_owned(),
                member: "object.subscribe".to_owned(),
                request_id: 1,
                flags: 0,
                kind: Some(Kind::Request(gnoblin_proto::Request { payload: Vec::new() })),
            },
        )
        .expect("write");
        std::io::Write::write_all(&mut far, &bytes).expect("write");
        let error = client.receive_handshake().expect_err("not a handshake");
        assert!(matches!(error, crate::error::HandshakeError::Codec(_)), "{error}");
    }
}
