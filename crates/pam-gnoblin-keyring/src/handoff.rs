//! The hand-off with no PAM in it: which request to send, and where. The `ffi` module collects the
//! password from libpam and calls these functions, so the part that can be tested runs without a
//! PAM stack.

use std::path::Path;

use gnoblin_keyring_handoff::{self as wire, FrameError, UnlockReply, UnlockRequest};
use zeroize::Zeroizing;

/// Send the login password to the keyring service listening under `runtime_dir`.
///
/// Returns `Ok(None)` without touching the socket when there is no password, which is the
/// fingerprint and autologin case. The service then prompts once, as `design/keyring.md` says.
pub fn unlock(runtime_dir: &Path, password: &[u8]) -> Result<Option<UnlockReply>, FrameError> {
    if password.is_empty() {
        return Ok(None);
    }
    let request = UnlockRequest::Unlock(Zeroizing::new(password.to_vec()));
    wire::send(&wire::socket_path(runtime_dir), &request).map(Some)
}

/// Ask the service to re-key the store from `old` to `new`, after a password change.
///
/// Returns `Ok(None)` when either password is missing, because a root-run change with no old
/// password cannot prove the store's key, so nothing is sent.
pub fn change(runtime_dir: &Path, old: &[u8], new: &[u8]) -> Result<Option<UnlockReply>, FrameError> {
    if old.is_empty() || new.is_empty() {
        return Ok(None);
    }
    let request = UnlockRequest::Change {
        old: Zeroizing::new(old.to_vec()),
        new: Zeroizing::new(new.to_vec()),
    };
    wire::send(&wire::socket_path(runtime_dir), &request).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::thread;

    /// A fresh runtime dir with the `gnoblin/` subdirectory the service creates.
    fn runtime_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pam-gnoblin-keyring-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("gnoblin")).expect("create the runtime dir");
        dir
    }

    /// A fake service: accepts one connection, records the request and answers `reply`.
    fn endpoint(dir: &Path, reply: UnlockReply) -> thread::JoinHandle<UnlockRequest> {
        let listener = UnixListener::bind(wire::socket_path(dir)).expect("bind the fake socket");
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let request = wire::read_request(&mut stream).expect("read the request");
            wire::write_reply(&mut stream, reply).expect("write the reply");
            request
        })
    }

    fn unlock_password(request: UnlockRequest) -> Vec<u8> {
        match request {
            UnlockRequest::Unlock(password) => password.to_vec(),
            UnlockRequest::Change { .. } => panic!("expected an unlock request"),
        }
    }

    #[test]
    fn unlock_sends_the_password_and_returns_the_reply() {
        let dir = runtime_dir("unlock-ok");
        let service = endpoint(&dir, UnlockReply::Ok);
        let reply = unlock(&dir, b"hunter2").expect("hand-off");
        assert_eq!(reply, Some(UnlockReply::Ok));
        assert_eq!(unlock_password(service.join().expect("service")), b"hunter2");
    }

    #[test]
    fn a_wrong_key_reply_is_passed_through() {
        let dir = runtime_dir("unlock-wrong");
        let service = endpoint(&dir, UnlockReply::WrongKey);
        assert_eq!(unlock(&dir, b"nope").expect("hand-off"), Some(UnlockReply::WrongKey));
        service.join().expect("service");
    }

    #[test]
    fn an_empty_password_sends_nothing() {
        // No socket exists, so any attempt to connect would fail the test.
        let dir = runtime_dir("unlock-empty");
        assert_eq!(unlock(&dir, b"").expect("nothing to send"), None);
    }

    #[test]
    fn a_missing_service_is_an_error_for_the_caller_to_handle() {
        let dir = runtime_dir("unlock-down");
        assert!(unlock(&dir, b"hunter2").is_err());
    }

    #[test]
    fn change_sends_the_old_and_new_passwords() {
        let dir = runtime_dir("change-ok");
        let listener = UnixListener::bind(wire::socket_path(&dir)).expect("bind the fake socket");
        let service = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let request = wire::read_request(&mut stream).expect("read the request");
            wire::write_reply(&mut stream, UnlockReply::Ok).expect("write the reply");
            request
        });
        assert_eq!(
            change(&dir, b"old pw", b"new pw").expect("hand-off"),
            Some(UnlockReply::Ok)
        );
        let UnlockRequest::Change { old, new } = service.join().expect("service") else {
            panic!("expected a change request");
        };
        assert_eq!(old.as_slice(), b"old pw");
        assert_eq!(new.as_slice(), b"new pw");
    }

    #[test]
    fn a_change_without_the_old_password_sends_nothing() {
        let dir = runtime_dir("change-no-old");
        assert_eq!(change(&dir, b"", b"new pw").expect("nothing to send"), None);
    }
}
