//! The EIS endpoint a session serves beside its IPC and Wayland sockets.
//!
//! Default deny: every connection starts waiting for a consent decision, and nothing
//! it sends reaches the seat until [`EisGate::grant`] approves it. Nothing in this
//! crate grants yet: the portal consent path is a later slice, so the gate is the
//! hook it will call.

use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use gnoblin_input_eis::{Caps, Endpoint, Grant, Options, Quota};
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

/// A handle on a session's running EIS endpoint.
///
/// It is empty until [`Compositor::run`](crate::Compositor::run) binds the endpoint, and
/// empty again once the session stops. Clones share one endpoint.
#[derive(Clone, Default)]
pub struct EisGate {
    endpoint: Arc<Mutex<Option<Endpoint>>>,
}

impl std::fmt::Debug for EisGate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("EisGate").finish_non_exhaustive()
    }
}

impl EisGate {
    /// Approves `capabilities` for every connection waiting on a decision.
    ///
    /// Returns `false` when no endpoint is running, so nothing was granted.
    pub fn grant(&self, capabilities: Caps) -> bool {
        self.with(|endpoint| endpoint.grant(capabilities))
    }

    /// Refuses every connection waiting on a decision, and every later one.
    ///
    /// Returns `false` when no endpoint is running.
    pub fn refuse(&self) -> bool {
        self.with(Endpoint::refuse)
    }

    fn with(&self, action: impl FnOnce(&Endpoint)) -> bool {
        let guard = self.endpoint.lock().unwrap_or_else(PoisonError::into_inner);
        match guard.as_ref() {
            Some(endpoint) => {
                action(endpoint);
                true
            }
            None => false,
        }
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
            // No output layout yet, so absolute devices have no region to map into.
            // The layout arrives with the output model; until then nothing is granted.
            regions: Vec::new(),
            quota: Quota::default(),
        };
        let endpoint = Endpoint::bind(options, move |event| sink.inject(event))?;
        std::fs::set_permissions(endpoint.path(), std::fs::Permissions::from_mode(0o600))?;
        *self.endpoint.lock().unwrap_or_else(PoisonError::into_inner) = Some(endpoint);
        Ok(())
    }

    /// Stops the endpoint and removes its socket.
    pub(crate) fn stop(&self) {
        let endpoint = self.endpoint.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(endpoint) = endpoint {
            endpoint.stop();
        }
    }
}
