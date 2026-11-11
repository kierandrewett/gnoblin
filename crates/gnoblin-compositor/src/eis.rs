//! The EIS endpoint a session serves beside its IPC and Wayland sockets.
//!
//! Default deny: every connection starts waiting for a consent decision, and nothing
//! it sends reaches the seat until the user's approval reaches it. There are two doors,
//! and only one of them has a production caller.
//!
//! - **The endpoint's own socket** (`eis.sock`) has one decision for every client that
//!   connects to it, which [`EisGate::grant`] and [`EisGate::refuse`] make. Nothing in
//!   a running session calls them: the socket stays default deny, and the test harness
//!   is what grants it (`design/rust-compositor.md`, "one input injection path").
//! - **A remote desktop session** has a decision of its own. The portal asks the user,
//!   and the IPC members `remote.grant` and `remote.refuse` ([`crate::ipc`]) bring the
//!   answer here as [`EisGate::grant_session`] and [`EisGate::refuse_session`].
//!   [`EisGate::connect_session`] then opens a one-shot socket for that session and
//!   nobody else, [`EisGate::revoke_session`] ends it, and the answer for one session
//!   never reaches another session or a client of `eis.sock`. A session the user did not
//!   approve gets no socket at all.

use std::collections::BTreeMap;
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use gnoblin_input_eis::{Caps, Consent, Endpoint, Grant, Options, Quota, Region, Scope};
use gnoblin_wayland::InputSink;

/// The file name of the EIS socket, beside the IPC socket.
const SOCKET_NAME: &str = "eis.sock";

/// The socket an endpoint serves, next to the IPC socket it shares a directory with.
///
/// The IPC path is the one place a session's runtime directory is resolved, so this
/// derives from it rather than reading the environment a second time.
#[must_use]
pub fn socket_path(ipc: &Path) -> PathBuf {
    ipc.with_file_name(SOCKET_NAME)
}

/// Why the gate refused a remote desktop session's call.
#[derive(Debug)]
pub enum SessionError {
    /// No endpoint is running, so nothing can be decided or connected.
    NotRunning,
    /// The session was already decided, once: a decision is not taken back or replaced,
    /// and only ending the session removes it.
    Decided,
    /// No decision was recorded for the session, so there is no consent to connect under.
    NoConsent,
    /// The user refused the session, which stays refused until it ends.
    Refused,
    /// The session's socket could not be made.
    Socket(io::Error),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotRunning => formatter.write_str("no EIS endpoint is running"),
            Self::Decided => formatter.write_str("the session was already decided"),
            Self::NoConsent => formatter.write_str("no consent was recorded for the session"),
            Self::Refused => formatter.write_str("the user refused the session"),
            Self::Socket(error) => write!(formatter, "the session's socket could not be made: {error}"),
        }
    }
}

impl std::error::Error for SessionError {}

/// What the gate knows about one remote desktop session.
struct Session {
    /// The scope the endpoint decides it under.
    scope: Scope,
    /// What the user answered: only [`Consent::Granted`] and [`Consent::Refused`] are held.
    consent: Consent,
}

/// The running endpoint and the sessions decided on it.
#[derive(Default)]
struct Inner {
    endpoint: Option<Endpoint>,
    /// The directory the endpoint's socket is in, where a session's socket goes.
    directory: PathBuf,
    /// The decided sessions, by the name the portal gave them.
    sessions: BTreeMap<String, Session>,
    next_scope: u64,
    next_socket: u64,
}

/// A handle on a session's running EIS endpoint.
///
/// It is empty until [`Compositor::run`](crate::Compositor::run) binds the endpoint, and
/// empty again once the session stops. Clones share one endpoint.
#[derive(Clone, Default)]
pub struct EisGate {
    inner: Arc<Mutex<Inner>>,
}

impl std::fmt::Debug for EisGate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("EisGate").finish_non_exhaustive()
    }
}

impl EisGate {
    /// Approves `capabilities` for every connection of `eis.sock` waiting on a decision.
    ///
    /// A remote desktop session is not decided here: see [`EisGate::grant_session`].
    /// Returns `false` when no endpoint is running, so nothing was granted.
    pub fn grant(&self, capabilities: Caps) -> bool {
        self.with(|inner| {
            if let Some(endpoint) = &inner.endpoint {
                endpoint.grant(capabilities);
            }
        })
    }

    /// Refuses every connection of `eis.sock` waiting on a decision, and every later one.
    ///
    /// Returns `false` when no endpoint is running.
    pub fn refuse(&self) -> bool {
        self.with(|inner| {
            if let Some(endpoint) = &inner.endpoint {
                endpoint.refuse();
            }
        })
    }

    /// The user approved remote desktop session `session` for `capabilities`: the seat of
    /// its EIS connection advertises them, and its connection alone.
    ///
    /// # Errors
    ///
    /// [`SessionError::NotRunning`] without an endpoint, and [`SessionError::Decided`]
    /// when the session already has an answer.
    pub fn grant_session(&self, session: &str, capabilities: Caps) -> Result<(), SessionError> {
        self.decide(session, Consent::Granted, |endpoint, scope| {
            endpoint.grant_scope(scope, capabilities);
        })
    }

    /// The user refused remote desktop session `session`: its connection ends, and no
    /// socket is ever opened for it.
    ///
    /// # Errors
    ///
    /// As [`EisGate::grant_session`].
    pub fn refuse_session(&self, session: &str) -> Result<(), SessionError> {
        self.decide(session, Consent::Refused, |endpoint, scope| {
            endpoint.refuse_scope(scope)
        })
    }

    /// Opens the one-shot socket of an approved session and answers its path.
    ///
    /// The first client to connect to it is the session's EIS connection; the path is
    /// removed as soon as it has, or when the session ends. `regions` is the output
    /// layout the session's absolute devices are created with.
    ///
    /// # Errors
    ///
    /// [`SessionError::NoConsent`] for a session nobody answered for,
    /// [`SessionError::Refused`] for one the user refused, and
    /// [`SessionError::Socket`] when the socket cannot be made.
    pub fn connect_session(&self, session: &str, regions: Vec<Region>) -> Result<PathBuf, SessionError> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(held) = inner.sessions.get(session) else {
            return Err(SessionError::NoConsent);
        };
        if held.consent != Consent::Granted {
            return Err(SessionError::Refused);
        }
        let scope = held.scope;
        inner.next_socket += 1;
        let path = inner.directory.join(format!("eis-{}.sock", inner.next_socket));
        let Some(endpoint) = &inner.endpoint else {
            return Err(SessionError::NotRunning);
        };
        endpoint.set_regions(regions);
        endpoint.listen_scope(scope, &path).map_err(SessionError::Socket)?;
        Ok(path)
    }

    /// Ends remote desktop session `session`: its connection ends after releasing what it
    /// held, its sockets are removed, and nothing can connect under it again.
    ///
    /// Returns whether the gate held the session.
    pub fn revoke_session(&self, session: &str) -> bool {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(held) = inner.sessions.remove(session) else {
            return false;
        };
        if let Some(endpoint) = &inner.endpoint {
            endpoint.revoke_scope(held.scope);
        }
        true
    }

    /// Whether the gate holds a decision for `session`.
    #[must_use]
    pub fn has_session(&self, session: &str) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .sessions
            .contains_key(session)
    }

    /// Records the answer for `session` once and hands the endpoint its scope.
    fn decide(&self, session: &str, consent: Consent, tell: impl FnOnce(&Endpoint, Scope)) -> Result<(), SessionError> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if inner.endpoint.is_none() {
            return Err(SessionError::NotRunning);
        }
        if inner.sessions.contains_key(session) {
            return Err(SessionError::Decided);
        }
        inner.next_scope += 1;
        let scope = Scope::new(inner.next_scope);
        if let Some(endpoint) = &inner.endpoint {
            tell(endpoint, scope);
        }
        inner.sessions.insert(session.to_owned(), Session { scope, consent });
        Ok(())
    }

    fn with(&self, action: impl FnOnce(&Inner)) -> bool {
        let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if inner.endpoint.is_none() {
            return false;
        }
        action(&inner);
        true
    }

    /// Binds the endpoint at `path` and hands its input to `sink`.
    ///
    /// A socket file a crashed session left behind is replaced, as the IPC socket is; a live
    /// one, or a file that is not a socket, is refused. The socket is made `0600` straight
    /// after the bind. The directory holding it is already private, because the IPC socket
    /// path prepares it.
    pub(crate) fn serve(&self, path: &Path, sink: InputSink) -> io::Result<()> {
        crate::ipc::replace_stale_socket(path).map_err(|error| match error {
            crate::ipc::IpcError::Io { source, .. } => source,
            other => io::Error::new(io::ErrorKind::AddrInUse, other.to_string()),
        })?;
        let options = Options {
            path: path.to_path_buf(),
            grant: Grant::pending(),
            // No output layout yet: a session's layout arrives with its `connect_session`, and
            // `eis.sock`, which nothing grants in a running session, has no absolute device.
            regions: Vec::new(),
            quota: Quota::default(),
        };
        let endpoint = Endpoint::bind(options, move |event| sink.inject(event))?;
        std::fs::set_permissions(endpoint.path(), std::fs::Permissions::from_mode(0o600))?;
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.directory = path.parent().map(Path::to_path_buf).unwrap_or_default();
        inner.endpoint = Some(endpoint);
        Ok(())
    }

    /// Stops the endpoint and removes its socket, and every session's with it.
    pub(crate) fn stop(&self) {
        let endpoint = {
            let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
            inner.sessions.clear();
            inner.endpoint.take()
        };
        if let Some(endpoint) = endpoint {
            endpoint.stop();
        }
    }
}
