//! The IPC socket and the object graph it serves.
//!
//! The plan puts the socket server in the compositor process ("Process model"),
//! and puts the object store there too ("One IPC": the compositor keeps the live
//! object graph). This module is therefore the whole server side of the IPC: where
//! the socket lives ([`path`]), what feeds the graph ([`session`]) and the
//! connections that read and answer it ([`server`]).
//!
//! Why here rather than in `gnoblin-model` or `gnoblin-proto`: the model is pure —
//! it does no I/O, spawns no thread and holds no socket — and the proto crate is
//! the codec. What is left is wiring, which is exactly what the composition root
//! is for. The alternative, a `gnoblin-ipc` crate, would be a crate that only this
//! process uses, so it would add a package to the graph without a boundary.
//!
//! The wire vocabulary is not re-implemented: the members, the snapshot and
//! observation payloads and the error codes are [`gnoblin_client::wire`]'s, which
//! is the same description a client sends and decodes. A server that spelled a
//! member differently from the client that reads it is the one bug this cannot
//! have.
//!
//! # Failure modes first
//!
//! The list is in the crate documentation; this module's own tests are beside the
//! code that keeps the promise, and `tests/ipc.rs` drives the same paths through a
//! real socket.

mod extension;
mod path;
mod portal;
mod privileged;
mod reload;
mod requests;
mod server;
mod session;
mod settings;
mod tree;
mod tree_host;

use std::path::PathBuf;

pub use extension::ExtensionRoute;
pub use path::SocketPath;
pub(crate) use path::replace_stale_socket;
pub use privileged::{Privileged, guardian_from_environment};
pub(crate) use server::IpcServer;
pub use session::SessionSync;

pub(crate) use server::{events, wake};

/// Why the IPC socket could not be prepared, bound or served.
#[derive(Debug)]
pub enum IpcError {
    /// The path names no directory, so there is nowhere to hold a socket.
    Path { path: PathBuf, reason: String },
    /// The directory around the socket is not one this process may trust: it is not
    /// a directory, not `0700`, or owned by another user.
    Directory { path: PathBuf, reason: String },
    /// Something that is not a socket is in the way. It is refused rather than
    /// removed: deleting a file this process did not create is not a start-up step.
    NotASocket { path: PathBuf },
    /// Another compositor is answering on the socket, so this one must not steal it.
    Running { path: PathBuf },
    /// The socket could not be bound or its permissions could not be set.
    Bind { path: PathBuf, source: std::io::Error },
    /// A temporary directory could not be created or a stale socket removed.
    Io { path: PathBuf, source: std::io::Error },
    /// The extension grants cannot be encoded as a grants frame, so nothing is bound.
    Grants(gnoblin_proto::GrantsError),
    /// The tree the shell serves could not be built, so nothing is bound.
    Tree(String),
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IpcError::Path { path, reason } => write!(formatter, "{}: {reason}", path.display()),
            IpcError::Directory { path, reason } => write!(
                formatter,
                "the IPC socket directory {} is not usable: {reason}",
                path.display()
            ),
            IpcError::NotASocket { path } => write!(
                formatter,
                "{} exists and is not a socket, so this compositor will not replace it",
                path.display()
            ),
            IpcError::Running { path } => write!(
                formatter,
                "another compositor is already serving {}, so this one cannot",
                path.display()
            ),
            IpcError::Bind { path, source } => write!(formatter, "cannot bind {}: {source}", path.display()),
            IpcError::Io { path, source } => write!(formatter, "{}: {source}", path.display()),
            IpcError::Grants(source) => write!(formatter, "the extension grants are not valid: {source}"),
            IpcError::Tree(reason) => write!(formatter, "the tree could not be built: {reason}"),
        }
    }
}

impl std::error::Error for IpcError {}
