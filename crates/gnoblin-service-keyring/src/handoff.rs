//! The unix socket the PAM module hands the login password to. The frame format is in
//! `gnoblin-keyring-handoff`; this module decides who may connect and what each request does.
//!
//! The socket lives in the user's `XDG_RUNTIME_DIR`, which is `0700` and per login. The directory
//! and the socket are chmodded anyway, so a runtime dir made by something else is still private.
//! The peer must be root (the login process) or the user. Any other uid is refused before a byte
//! of the password is read.

use std::fs::{DirBuilder, Permissions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use gnoblin_keyring_handoff::{self as wire, TIMEOUT, UnlockReply, UnlockRequest};

use crate::log;
use crate::unlock::{UnlockError, UnlockKeySource};

/// The mode of the socket's directory and of the socket itself.
const DIR_MODE: u32 = 0o700;
const SOCKET_MODE: u32 = 0o600;

/// Bind the hand-off socket at `socket` and serve it on a thread that lives as long as the
/// process. A stale socket file from an earlier run is removed first.
pub fn listen(socket: &Path, source: Arc<Mutex<UnlockKeySource>>) -> io::Result<()> {
    let dir = socket
        .parent()
        .ok_or_else(|| io::Error::other("the hand-off socket has no directory"))?;
    DirBuilder::new().mode(DIR_MODE).recursive(true).create(dir)?;
    std::fs::set_permissions(dir, Permissions::from_mode(DIR_MODE))?;
    match std::fs::remove_file(socket) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, Permissions::from_mode(SOCKET_MODE))?;
    let own_uid = rustix::process::getuid().as_raw();
    log::info!("listening for the login password hand-off");
    thread::Builder::new()
        .name("keyring-unlock".to_owned())
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => serve_one(stream, own_uid, &source),
                    Err(error) => log::warn!("the hand-off socket failed to accept: {error}"),
                }
            }
        })?;
    Ok(())
}

/// Answer one connection. Hand-offs are serial: a login sends one request, and the store lock
/// is held for the length of the open anyway.
fn serve_one(mut stream: UnixStream, own_uid: u32, source: &Arc<Mutex<UnlockKeySource>>) {
    if stream.set_read_timeout(Some(TIMEOUT)).is_err() || stream.set_write_timeout(Some(TIMEOUT)).is_err() {
        return;
    }
    if !peer_allowed(&stream, own_uid) {
        log::warn!("refused a hand-off from a peer that is neither root nor the user");
        let _ = wire::write_reply(&mut stream, UnlockReply::Refused);
        return;
    }
    let request = match wire::read_request(&mut stream) {
        Ok(request) => request,
        Err(error) => {
            log::warn!("a hand-off request could not be read: {error}");
            let _ = wire::write_reply(&mut stream, UnlockReply::Refused);
            return;
        }
    };
    let reply = {
        let mut source = lock(source);
        match request {
            UnlockRequest::Unlock(password) => reply_for(source.unlock(&password)),
            UnlockRequest::Change { old, new } => reply_for(source.change_password(&old, &new)),
        }
    };
    if let Err(error) = wire::write_reply(&mut stream, reply) {
        log::warn!("the hand-off reply could not be written: {error}");
    }
}

/// Map the unlock outcome onto the wire reply. The reason text stays in the log line and
/// never reaches the peer, and never names the password.
fn reply_for(outcome: Result<(), UnlockError>) -> UnlockReply {
    match outcome {
        Ok(()) => UnlockReply::Ok,
        Err(UnlockError::WrongKey) => UnlockReply::WrongKey,
        Err(UnlockError::Refused(reason)) => {
            log::warn!("a hand-off was refused: {reason}");
            UnlockReply::Refused
        }
        Err(UnlockError::Failed(reason)) => {
            log::warn!("a hand-off failed: {reason}");
            UnlockReply::Failed
        }
    }
}

/// Whether the process on the other end runs as root or as the user who owns the store.
fn peer_allowed(stream: &UnixStream, own_uid: u32) -> bool {
    match rustix::net::sockopt::socket_peercred(stream) {
        Ok(cred) => {
            let uid = cred.uid.as_raw();
            uid == 0 || uid == own_uid
        }
        Err(error) => {
            log::warn!("the hand-off peer's credentials could not be read: {error}");
            false
        }
    }
}

fn lock(source: &Mutex<UnlockKeySource>) -> MutexGuard<'_, UnlockKeySource> {
    // A panic in an earlier hand-off leaves the key source usable: its state is the store handle,
    // which is transactional, so refusing every later login would be worse.
    source.lock().unwrap_or_else(PoisonError::into_inner)
}
