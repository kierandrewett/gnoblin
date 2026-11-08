//! Where the IPC socket lives, and what has to be true before it is bound.
//!
//! Design/rust-compositor.md, "One IPC": the socket is
//! `$XDG_RUNTIME_DIR/gnoblin/ipc.sock`, found through `GNOBLIN_IPC_SOCKET`. The
//! path resolution is [`gnoblin_client::unix::socket_path`]'s — one implementation
//! for every side — and what this module adds is the server's own obligation: the
//! directory is only this user's, and a file left by a session that died is
//! replaced while a live one is not.
//!
//! # Failure modes first
//!
//! 1. **A stale socket blocks the next session.** The file is probed by connecting:
//!    a refusal means nothing is listening and the file is removed, so a crashed
//!    session never keeps the user out of their own desktop —
//!    `a_stale_socket_is_replaced`.
//! 2. **Two compositors bind one socket.** A successful connect means a compositor
//!    is answering, so the second start is refused with the path named, and the
//!    first keeps serving: `two_compositors_on_one_socket_are_refused`.
//! 3. **The directory is reachable by another account.** A directory that is not
//!    `0700` or is owned by another user is refused before anything is bound, and so
//!    is a regular file in the socket's place: the first would let another user
//!    connect to the compositor, the second would be deleted by a compositor that
//!    has no business deleting it —
//!    `a_reachable_socket_directory_is_refused` and `a_regular_file_is_not_replaced`.
//! 4. **The socket outlives the session.** The file is created `0600` and removed by
//!    [`super::IpcServer::shutdown`], so a second session finds it stale at worst.
//!    A session that is killed leaves a stale file, which is mode 1's case.

use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use gnoblin_types::Version;

use super::IpcError;

/// The mode the socket's directory must have: only its owner may enter it.
const DIRECTORY_MODE: u32 = 0o700;

/// The mode the socket file gets: only its owner may connect to it.
const SOCKET_MODE: u32 = 0o600;

/// A socket path that is ready to be bound.
#[derive(Clone, Debug)]
pub struct SocketPath {
    path: PathBuf,
    version: Version,
}

impl SocketPath {
    /// Checks everything that must be true before binding, and clears a stale socket.
    ///
    /// Nothing is bound here: the caller binds when it starts serving, so the
    /// checks and the refusal happen before a Wayland session exists to tear down.
    ///
    /// # Errors
    ///
    /// [`IpcError::Path`] when the path has no directory,
    /// [`IpcError::Directory`] when that directory is not `0700` and owned by this
    /// user, [`IpcError::NotASocket`] when a file that is not a socket is in the
    /// way, and [`IpcError::Running`] when another compositor answers on it.
    pub fn prepare(path: &Path, version: Version) -> Result<SocketPath, IpcError> {
        let directory = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| IpcError::Path {
                path: path.to_path_buf(),
                reason: "the socket path has no directory, so there is nowhere to create it".to_owned(),
            })?;
        prepare_directory(directory)?;
        replace_stale_socket(path)?;
        Ok(SocketPath {
            path: path.to_path_buf(),
            version,
        })
    }

    /// The socket's path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The IPC version this process serves, which the handshake answers with.
    #[must_use]
    pub fn version(&self) -> Version {
        self.version
    }

    /// Tightens the freshly bound socket to [`SOCKET_MODE`].
    pub(crate) fn restrict(&self) -> Result<(), IpcError> {
        std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(SOCKET_MODE)).map_err(|source| {
            IpcError::Bind {
                path: self.path.clone(),
                source,
            }
        })
    }
}

/// Creates the directory `0700` if it is missing, and refuses one that is not ours.
///
/// The order matters: a directory this process creates is ours by construction, and
/// an existing one is only trusted when it is `0700` and owned by the effective
/// user. Checking after creating means a missing `$XDG_RUNTIME_DIR/gnoblin` costs
/// nothing and the check has one form.
fn prepare_directory(directory: &Path) -> Result<(), IpcError> {
    match std::fs::DirBuilder::new()
        .recursive(true)
        .mode(DIRECTORY_MODE)
        .create(directory)
    {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(source) => {
            return Err(IpcError::Io {
                path: directory.to_path_buf(),
                source,
            });
        }
    }
    let metadata = std::fs::metadata(directory).map_err(|source| IpcError::Io {
        path: directory.to_path_buf(),
        source,
    })?;
    if !metadata.is_dir() {
        return Err(IpcError::Directory {
            path: directory.to_path_buf(),
            reason: "it is not a directory".to_owned(),
        });
    }
    let mode = metadata.permissions().mode() & 0o777;
    if mode != DIRECTORY_MODE {
        return Err(IpcError::Directory {
            path: directory.to_path_buf(),
            reason: format!("it has mode {mode:04o}, and the socket's directory must be 0700"),
        });
    }
    // The socket is only as private as the directory, and a directory another user
    // owns is a directory another user controls.
    let effective = rustix::process::geteuid().as_raw();
    if metadata.uid() != effective {
        return Err(IpcError::Directory {
            path: directory.to_path_buf(),
            reason: format!("it is owned by user {}, not by user {effective}", metadata.uid()),
        });
    }
    Ok(())
}

/// Removes a socket file nothing is listening on, and refuses a live one.
pub(crate) fn replace_stale_socket(path: &Path) -> Result<(), IpcError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(IpcError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if !metadata.file_type().is_socket() {
        return Err(IpcError::NotASocket {
            path: path.to_path_buf(),
        });
    }
    // A connect is the only honest test of "is a compositor there": a socket file
    // with a reader is refused by the kernel when it is stale, and a live one
    // completes the connection.
    match UnixStream::connect(path) {
        Ok(_) => Err(IpcError::Running {
            path: path.to_path_buf(),
        }),
        Err(_) => std::fs::remove_file(path).map_err(|source| IpcError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;
    use std::os::unix::net::UnixListener;

    use super::{SocketPath, prepare_directory};
    use crate::ipc::IpcError;
    use gnoblin_types::Version;

    /// A directory of this test's own, with the mode the checks require.
    fn private_directory() -> tempfile::TempDir {
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(super::DIRECTORY_MODE))
            .expect("the directory mode");
        directory
    }

    #[test]
    fn a_stale_socket_is_replaced() {
        let directory = private_directory();
        let path = directory.path().join("ipc.sock");
        // A socket nobody listens on is exactly what a session that was killed leaves.
        let listener = UnixListener::bind(&path).expect("bind");
        drop(listener);
        let prepared = SocketPath::prepare(&path, Version::new(0, 2, 0)).expect("the stale socket is replaced");
        assert_eq!(prepared.path(), path);
        assert!(!path.exists(), "the stale file is gone until the server binds");
    }

    #[test]
    fn two_compositors_on_one_socket_are_refused() {
        let directory = private_directory();
        let path = directory.path().join("ipc.sock");
        let _listener = UnixListener::bind(&path).expect("bind");
        match SocketPath::prepare(&path, Version::new(0, 2, 0)) {
            Err(IpcError::Running { path: reported }) => assert_eq!(reported, path),
            other => panic!("a live socket must be refused, got {other:?}"),
        }
        assert!(path.exists(), "the live socket is left alone");
    }

    #[test]
    fn a_reachable_socket_directory_is_refused() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o755)).expect("the mode");
        let path = directory.path().join("ipc.sock");
        match SocketPath::prepare(&path, Version::new(0, 2, 0)) {
            Err(IpcError::Directory { reason, .. }) => assert!(reason.contains("0700"), "{reason}"),
            other => panic!("a directory another user can enter must be refused, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_directory_is_created_private() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).expect("the mode");
        let nested = directory.path().join("gnoblin");
        SocketPath::prepare(&nested.join("ipc.sock"), Version::new(0, 2, 0)).expect("the directory is created");
        let metadata = std::fs::metadata(&nested).expect("the directory exists");
        assert_eq!(metadata.permissions().mode() & 0o777, super::DIRECTORY_MODE);
    }

    #[test]
    fn a_regular_file_is_not_replaced() {
        let directory = private_directory();
        let path = directory.path().join("ipc.sock");
        std::fs::write(&path, b"not a socket").expect("write");
        match SocketPath::prepare(&path, Version::new(0, 2, 0)) {
            Err(IpcError::NotASocket { .. }) => {}
            other => panic!("a regular file must not be deleted, got {other:?}"),
        }
        assert!(path.exists(), "the file is still there");
    }

    #[test]
    fn the_directory_check_accepts_what_it_created() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let nested = directory.path().join("gnoblin");
        prepare_directory(&nested).expect("created private");
        prepare_directory(&nested).expect("and accepted on the second call");
    }
}
