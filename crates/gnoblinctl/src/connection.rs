//! How `gnoblinctl` reaches the compositor, and the exit codes it reports.
//!
//! Design/rust-compositor.md, "One IPC": the socket is
//! `$XDG_RUNTIME_DIR/gnoblin/ipc.sock`, found through `GNOBLIN_IPC_SOCKET`. The
//! resolution is a function of the environment rather than of the process, so the
//! precedence is testable without changing a process-global variable.
//!
//! The exit codes are the whole contract with a script:
//!
//! | code | meaning                                                              |
//! | ---- | -------------------------------------------------------------------- |
//! | 0    | the command did what it was asked                                    |
//! | 1    | the compositor answered with an error, or the connection ended early  |
//! | 2    | the command line itself was wrong                                    |
//! | 3    | the compositor could not be reached                                  |
//! | 4    | the compositor speaks a version this build cannot use                 |
//!
//! 3 and 4 are separate because they need different reactions: 3 is "start the
//! compositor", 4 is "upgrade one side", and the mismatch message names both
//! versions so neither guess is necessary.

use std::fmt;
use std::path::{Path, PathBuf};

use gnoblin_client::{Client, ConnectError, Transport};
use gnoblin_proto::Mismatch;

use crate::socket::UnixTransport;

/// The variable that overrides the socket path.
pub const SOCKET_VARIABLE: &str = "GNOBLIN_IPC_SOCKET";

/// The directory variable the default path is built from.
pub const RUNTIME_VARIABLE: &str = "XDG_RUNTIME_DIR";

/// The socket path under `$XDG_RUNTIME_DIR`.
pub const DEFAULT_SOCKET: &str = "gnoblin/ipc.sock";

/// A way to open one connection.
///
/// A command that needs the compositor calls [`Connection::connect`] once. Keeping
/// the trait separate from the socket is what lets the tests drive the real command
/// logic over the in-process loopback and over a real socket with the same code.
pub trait Connection {
    /// The transport this connection produces.
    type Transport: Transport;

    /// Open the connection, or say why it could not be opened.
    fn connect(self) -> Result<Client<Self::Transport>, ConnectFailure>;
}

/// Why no usable connection was opened.
#[derive(Debug)]
pub enum ConnectFailure {
    /// No path is configured, or nothing is listening at it.
    Socket { path: Option<PathBuf>, reason: String },
    /// The socket answered, but not with a handshake this build understands.
    Handshake { reason: String },
    /// The compositor refused this build's IPC version.
    Version(Mismatch),
}

impl ConnectFailure {
    /// The exit code this failure reports.
    pub fn exit_code(&self) -> u8 {
        match self {
            ConnectFailure::Version(_) => 4,
            ConnectFailure::Socket { .. } | ConnectFailure::Handshake { .. } => 3,
        }
    }

    /// Map a client connect error, keeping the version mismatch separate from the
    /// protocol failures because only one of them is about versions.
    pub fn from_connect(error: ConnectError) -> ConnectFailure {
        match error {
            ConnectError::Refused(mismatch) => ConnectFailure::Version(mismatch),
            ConnectError::Handshake(error) => ConnectFailure::Handshake {
                reason: error.to_string(),
            },
        }
    }

    /// The error code printed in `--json` mode, so a script reads a code rather
    /// than a sentence.
    pub fn code(&self) -> &'static str {
        match self {
            ConnectFailure::Socket { .. } => "cannot-connect",
            ConnectFailure::Handshake { .. } => "handshake-failed",
            ConnectFailure::Version(_) => "version-mismatch",
        }
    }
}

impl fmt::Display for ConnectFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectFailure::Socket {
                path: Some(path),
                reason,
            } => write!(formatter, "cannot connect to {}: {reason}", path.display()),
            ConnectFailure::Socket { path: None, reason } => formatter.write_str(reason),
            ConnectFailure::Handshake { reason } => {
                write!(formatter, "the socket did not answer the handshake: {reason}")
            }
            ConnectFailure::Version(mismatch) => write!(formatter, "{mismatch}"),
        }
    }
}

impl std::error::Error for ConnectFailure {}

/// The socket path, from the two variables that decide it.
///
/// `env` is passed in so the precedence can be tested without a process-global
/// write, which is `unsafe` in this edition and would make the tests order
/// dependent.
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

/// The compositor's socket.
#[derive(Debug)]
pub struct SocketConnection {
    socket: Result<PathBuf, String>,
}

impl SocketConnection {
    /// The socket the environment names.
    pub fn from_environment() -> SocketConnection {
        SocketConnection {
            socket: socket_path(|name| std::env::var(name).ok()),
        }
    }

    /// The socket at `path`, whatever the environment says.
    pub fn at(path: impl Into<PathBuf>) -> SocketConnection {
        SocketConnection {
            socket: Ok(path.into()),
        }
    }
}

impl Connection for SocketConnection {
    type Transport = UnixTransport;

    fn connect(self) -> Result<Client<UnixTransport>, ConnectFailure> {
        let path = self
            .socket
            .map_err(|reason| ConnectFailure::Socket { path: None, reason })?;
        let transport = UnixTransport::connect(&path).map_err(|error| ConnectFailure::Socket {
            path: Some(path.clone()),
            reason: error.to_string(),
        })?;
        Client::connect(transport, crate::tree::ipc_version()).map_err(ConnectFailure::from_connect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn the_socket_variable_wins_over_the_runtime_directory() {
        let path = socket_path(environment(&[
            (SOCKET_VARIABLE, "/run/user/1000/other.sock"),
            (RUNTIME_VARIABLE, "/run/user/1000"),
        ]))
        .expect("both variables are set");
        assert_eq!(path, PathBuf::from("/run/user/1000/other.sock"));
    }

    #[test]
    fn the_default_is_built_from_the_runtime_directory() {
        let path = socket_path(environment(&[(RUNTIME_VARIABLE, "/run/user/1000")])).expect("the directory is set");
        assert_eq!(path, PathBuf::from("/run/user/1000/gnoblin/ipc.sock"));
    }

    #[test]
    fn neither_variable_is_a_cannot_connect_error() {
        let error = socket_path(environment(&[])).expect_err("nothing names a socket");
        assert!(
            error.contains(SOCKET_VARIABLE) && error.contains(RUNTIME_VARIABLE),
            "{error}"
        );
    }

    #[test]
    fn an_empty_variable_is_not_a_path() {
        let path = socket_path(environment(&[
            (SOCKET_VARIABLE, ""),
            (RUNTIME_VARIABLE, "/run/user/1000"),
        ]))
        .expect("the directory still names one");
        assert_eq!(path, PathBuf::from("/run/user/1000/gnoblin/ipc.sock"));
    }
}
