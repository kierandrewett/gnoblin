//! The hand-off from `pam_gnoblin_keyring` to `gnoblin-service-keyring`: the login password, or a
//! password change, sent over a unix socket. See `design/keyring.md` "Key source".
//!
//! A frame is a version byte, an operation byte, a big-endian `u32` payload length and the
//! payload. The reply is a version byte and a status byte. The payload is the password itself,
//! so the protocol never names it in a log line, and no request type derives `Debug`.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use zeroize::Zeroizing;

/// The only frame version this crate speaks. A peer on another version is refused, not guessed at.
pub const VERSION: u8 = 1;

/// The largest payload accepted. A password is far shorter; the cap stops a hostile peer from
/// making the service allocate on its say-so.
pub const MAX_PAYLOAD: usize = 4096;

/// How long either side waits for the other. The hand-off runs during login, so a hung peer
/// must not hang the login.
pub const TIMEOUT: Duration = Duration::from_secs(10);

const DIR_NAME: &str = "gnoblin";
const SOCKET_NAME: &str = "keyring-unlock.sock";

const OP_UNLOCK: u8 = 1;
const OP_CHANGE: u8 = 2;

const STATUS_OK: u8 = 0;
const STATUS_WRONG_KEY: u8 = 1;
const STATUS_REFUSED: u8 = 2;
const STATUS_FAILED: u8 = 3;

/// A request from the PAM module. Both passwords are wiped when the request is dropped.
pub enum UnlockRequest {
    /// Unlock the store with the login password.
    Unlock(Zeroizing<Vec<u8>>),
    /// Re-key the store from `old` to `new`, after a password change.
    Change {
        old: Zeroizing<Vec<u8>>,
        new: Zeroizing<Vec<u8>>,
    },
}

/// The service's answer to a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnlockReply {
    /// The store is unlocked, or re-keyed.
    Ok,
    /// The password does not open the store. Nothing was changed.
    WrongKey,
    /// The request was malformed, or the service does not accept it from this peer.
    Refused,
    /// The store could not be read or written.
    Failed,
}

/// Why a frame could not be read or written.
#[derive(Debug)]
pub enum FrameError {
    /// The socket failed or closed early.
    Io(io::Error),
    /// The peer speaks another frame version.
    Version(u8),
    /// The operation byte names no operation.
    Operation(u8),
    /// The payload is longer than `MAX_PAYLOAD`.
    TooLarge(usize),
    /// A change frame's old-password length does not fit its payload.
    Malformed,
    /// The reply status byte names no status.
    Status(u8),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "keyring hand-off socket failed: {error}"),
            Self::Version(version) => write!(f, "keyring hand-off version {version} is not {VERSION}"),
            Self::Operation(op) => write!(f, "keyring hand-off operation {op} is unknown"),
            Self::TooLarge(len) => write!(f, "keyring hand-off payload of {len} bytes is too large"),
            Self::Malformed => write!(f, "keyring hand-off change frame is malformed"),
            Self::Status(status) => write!(f, "keyring hand-off status {status} is unknown"),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<io::Error> for FrameError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// The socket the service listens on: `<runtime_dir>/gnoblin/keyring-unlock.sock`.
/// `XDG_RUNTIME_DIR` is per user and per login, so the path is too.
pub fn socket_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join(DIR_NAME).join(SOCKET_NAME)
}

/// Write a request as one frame.
pub fn write_request<W: Write>(writer: &mut W, request: &UnlockRequest) -> Result<(), FrameError> {
    let (op, payload) = match request {
        UnlockRequest::Unlock(password) => (OP_UNLOCK, Zeroizing::new(password.to_vec())),
        UnlockRequest::Change { old, new } => {
            let mut payload = Zeroizing::new(Vec::with_capacity(4 + old.len() + new.len()));
            payload.extend_from_slice(&length_prefix(old.len())?);
            payload.extend_from_slice(old);
            payload.extend_from_slice(new);
            (OP_CHANGE, payload)
        }
    };
    if payload.len() > MAX_PAYLOAD {
        return Err(FrameError::TooLarge(payload.len()));
    }
    let mut frame = Zeroizing::new(Vec::with_capacity(6 + payload.len()));
    frame.push(VERSION);
    frame.push(op);
    frame.extend_from_slice(&length_prefix(payload.len())?);
    frame.extend_from_slice(&payload);
    writer.write_all(&frame)?;
    Ok(())
}

/// Read one request frame. The length is checked before the payload is allocated.
pub fn read_request<R: Read>(reader: &mut R) -> Result<UnlockRequest, FrameError> {
    let mut header = [0_u8; 6];
    reader.read_exact(&mut header)?;
    let [version, op, l0, l1, l2, l3] = header;
    if version != VERSION {
        return Err(FrameError::Version(version));
    }
    let len = u32::from_be_bytes([l0, l1, l2, l3]) as usize;
    if len > MAX_PAYLOAD {
        return Err(FrameError::TooLarge(len));
    }
    let mut payload = Zeroizing::new(vec![0_u8; len]);
    reader.read_exact(&mut payload)?;
    match op {
        OP_UNLOCK => Ok(UnlockRequest::Unlock(payload)),
        OP_CHANGE => {
            let old_len = payload
                .get(..4)
                .map(|bytes| u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize)
                .ok_or(FrameError::Malformed)?;
            let rest = &payload[4..];
            if old_len > rest.len() {
                return Err(FrameError::Malformed);
            }
            let (old, new) = rest.split_at(old_len);
            Ok(UnlockRequest::Change {
                old: Zeroizing::new(old.to_vec()),
                new: Zeroizing::new(new.to_vec()),
            })
        }
        other => Err(FrameError::Operation(other)),
    }
}

/// Write a reply as one frame.
pub fn write_reply<W: Write>(writer: &mut W, reply: UnlockReply) -> Result<(), FrameError> {
    let status = match reply {
        UnlockReply::Ok => STATUS_OK,
        UnlockReply::WrongKey => STATUS_WRONG_KEY,
        UnlockReply::Refused => STATUS_REFUSED,
        UnlockReply::Failed => STATUS_FAILED,
    };
    writer.write_all(&[VERSION, status])?;
    Ok(())
}

/// Read one reply frame.
pub fn read_reply<R: Read>(reader: &mut R) -> Result<UnlockReply, FrameError> {
    let mut frame = [0_u8; 2];
    reader.read_exact(&mut frame)?;
    let [version, status] = frame;
    if version != VERSION {
        return Err(FrameError::Version(version));
    }
    match status {
        STATUS_OK => Ok(UnlockReply::Ok),
        STATUS_WRONG_KEY => Ok(UnlockReply::WrongKey),
        STATUS_REFUSED => Ok(UnlockReply::Refused),
        STATUS_FAILED => Ok(UnlockReply::Failed),
        other => Err(FrameError::Status(other)),
    }
}

/// Send one request to the service at `socket` and return its reply. Used by the PAM module.
/// A connect failure means the service is not up yet; the caller decides what that means.
pub fn send(socket: &Path, request: &UnlockRequest) -> Result<UnlockReply, FrameError> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    write_request(&mut stream, request)?;
    read_reply(&mut stream)
}

fn length_prefix(len: usize) -> Result<[u8; 4], FrameError> {
    let len = u32::try_from(len).map_err(|_| FrameError::TooLarge(len))?;
    Ok(len.to_be_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    fn bytes(text: &str) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(text.as_bytes().to_vec())
    }

    fn round_trip(request: &UnlockRequest) -> UnlockRequest {
        let mut wire = Vec::new();
        write_request(&mut wire, request).expect("write");
        read_request(&mut wire.as_slice()).expect("read")
    }

    #[test]
    fn unlock_round_trips() {
        let UnlockRequest::Unlock(password) = round_trip(&UnlockRequest::Unlock(bytes("hunter2"))) else {
            panic!("expected an unlock request");
        };
        assert_eq!(password.as_slice(), b"hunter2");
    }

    #[test]
    fn change_round_trips_both_passwords() {
        let request = UnlockRequest::Change {
            old: bytes("old-pass"),
            new: bytes("new-pass-longer"),
        };
        let UnlockRequest::Change { old, new } = round_trip(&request) else {
            panic!("expected a change request");
        };
        assert_eq!(old.as_slice(), b"old-pass");
        assert_eq!(new.as_slice(), b"new-pass-longer");
    }

    #[test]
    fn replies_round_trip() {
        for reply in [
            UnlockReply::Ok,
            UnlockReply::WrongKey,
            UnlockReply::Refused,
            UnlockReply::Failed,
        ] {
            let mut wire = Vec::new();
            write_reply(&mut wire, reply).expect("write");
            assert_eq!(read_reply(&mut wire.as_slice()).expect("read"), reply);
        }
    }

    #[test]
    fn other_version_is_refused() {
        let wire = [VERSION + 1, OP_UNLOCK, 0, 0, 0, 0];
        assert!(matches!(
            read_request(&mut wire.as_slice()),
            Err(FrameError::Version(_))
        ));
    }

    #[test]
    fn oversize_length_is_refused_before_reading_payload() {
        // The header claims more than the cap; no payload bytes follow, so reading one would fail
        // with an I/O error instead of `TooLarge` if the length were trusted first.
        let len = u32::try_from(MAX_PAYLOAD + 1).expect("fits");
        let mut wire = vec![VERSION, OP_UNLOCK];
        wire.extend_from_slice(&len.to_be_bytes());
        assert!(matches!(
            read_request(&mut wire.as_slice()),
            Err(FrameError::TooLarge(_))
        ));
    }

    #[test]
    fn unknown_operation_is_refused() {
        let wire = [VERSION, 9, 0, 0, 0, 0];
        assert!(matches!(
            read_request(&mut wire.as_slice()),
            Err(FrameError::Operation(9))
        ));
    }

    #[test]
    fn change_with_old_length_past_payload_is_malformed() {
        let mut wire = vec![VERSION, OP_CHANGE, 0, 0, 0, 4];
        wire.extend_from_slice(&[0, 0, 0, 99]);
        assert!(matches!(read_request(&mut wire.as_slice()), Err(FrameError::Malformed)));
    }

    #[test]
    fn socket_path_sits_under_the_runtime_dir() {
        assert_eq!(
            socket_path(Path::new("/run/user/1000")),
            PathBuf::from("/run/user/1000/gnoblin/keyring-unlock.sock")
        );
    }

    /// A fake service: accepts one connection, checks the request, sends `reply`.
    fn fake_service(socket: PathBuf, reply: UnlockReply) -> std::thread::JoinHandle<UnlockRequest> {
        let listener = UnixListener::bind(&socket).expect("bind");
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let request = read_request(&mut stream).expect("read request");
            write_reply(&mut stream, reply).expect("write reply");
            request
        })
    }

    fn scratch_socket(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gnoblin-handoff-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let socket = dir.join("s.sock");
        let _ = std::fs::remove_file(&socket);
        socket
    }

    #[test]
    fn send_delivers_the_password_and_returns_the_reply() {
        let socket = scratch_socket("unlock");
        let service = fake_service(socket.clone(), UnlockReply::Ok);
        let reply = send(&socket, &UnlockRequest::Unlock(bytes("login-pass"))).expect("send");
        assert_eq!(reply, UnlockReply::Ok);
        let UnlockRequest::Unlock(received) = service.join().expect("service") else {
            panic!("expected an unlock request");
        };
        assert_eq!(received.as_slice(), b"login-pass");
    }

    #[test]
    fn send_reports_a_missing_service_as_io() {
        let socket = scratch_socket("absent");
        assert!(matches!(
            send(&socket, &UnlockRequest::Unlock(bytes("x"))),
            Err(FrameError::Io(_))
        ));
    }
}
