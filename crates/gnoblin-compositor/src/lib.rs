//! Composition root: one runnable compositor process, and the only crate that
//! knows every subsystem.
//!
//! This is the integration lane of `design/rust-compositor.md` ("Process model",
//! "One IPC", "Main data paths" and the `gnoblin-compositor` card): a process that
//! starts a Wayland session on a backend, keeps the live object graph the IPC
//! serves, and answers `gnoblinctl` and every other IPC client on
//! `$XDG_RUNTIME_DIR/gnoblin/ipc.sock`. Nothing here decides policy, renders or
//! speaks a protocol by hand: it wires [`gnoblin_wayland`], [`gnoblin_backend`],
//! [`gnoblin_model`] and the wire vocabulary together and gets out of the way.
//!
//! # What a run does
//!
//! 1. The schema is the graph's vocabulary, embedded at build time, and the
//!    version its `gnoblin` namespace declares is the IPC version this process
//!    serves — the plan makes that exactly the Gnoblin version.
//! 2. The IPC socket is prepared and bound before anything else, on a directory
//!    only this user can reach. Binding first is what lets a shell connect the
//!    moment it sees the file, and it is where a second compositor is refused
//!    rather than allowed to overwrite a live socket.
//! 3. The Wayland session starts on the backend [`SessionConfig::backend`] names, with
//!    an [`gnoblin_wayland::Observer`] that forwards every published snapshot.
//! 4. The loop runs on this thread. The IPC server owns the graph on another one,
//!    because a calloop loop and the backend it drives are not `Send`; the two
//!    sides meet at the snapshot, which is the session's own description of its
//!    windows and lock state.
//! 5. When the loop stops, the IPC server is told to stop, both threads are
//!    joined and the socket is removed.
//!
//! # The object graph
//!
//! [`SessionSync`] owns it: one `session` object carrying the version, and one
//! `window` object per managed window carrying the title, the app id, focus and the
//! frame rectangle. Every change is a property diff with its revision, published
//! from the same [`SessionSnapshot`] every other reader sees, so the graph cannot
//! describe a window the session does not have. `window.focus` is the one method
//! this slice serves, and it goes through [`gnoblin_wayland::ControlHandle::focus`]
//! into the window manager, so focus policy still decides.
//!
//! # Failure modes first
//!
//! Each entry names the test that covers it, in `tests/ipc.rs` unless it says
//! otherwise.
//!
//! 1. **A stale socket blocks the next session.** A socket file a crashed session
//!    left behind is replaced, because nothing is listening on it:
//!    `a_stale_socket_is_replaced`.
//! 2. **Two compositors bind one socket.** The second start is refused with the
//!    path named and the first session keeps serving its clients:
//!    `two_compositors_on_one_socket_are_refused`.
//! 3. **The socket directory is reachable by other users.** A directory that is not
//!    `0700` and owned by this user is refused before anything binds, so no other
//!    account can drive the compositor: `a_reachable_socket_directory_is_refused`.
//! 4. **An IPC peer's crash stops the compositor.** A connection that dies
//!    mid-subscription ends that connection, its subscriptions and its pending
//!    operations, and the session keeps serving everyone else:
//!    `an_ipc_peer_that_dies_does_not_stop_the_compositor`.
//! 5. **A window object outlives its surface.** A client that disconnects leaves no
//!    `window` object behind, and the object it had is closed once:
//!    `a_window_object_outlives_its_surface_only_until_the_surface_ends`.
//! 6. **Focus is called on a window that is gone.** The call is refused with
//!    `object-unknown` instead of focusing an identity nobody holds, and the session
//!    keeps running: `focus_on_a_closed_window_is_refused`.
//! 7. **A client speaks a version this compositor cannot serve.** The handshake
//!    refuses before any envelope is decoded and names both versions:
//!    `a_wrong_version_client_is_refused`.
//! 8. **The graph publishes a window the session does not have, or keeps one it
//!    lost.** The diffs are computed from the snapshot, and the pure mapping is
//!    tested on its own in `ipc::session`'s tests.
//!
//! # Where the next lane plugs in
//!
//! Every seam below is a trait, a channel or a match arm, so a subsystem lands
//! without editing an existing module:
//!
//! - a backend is a variant of [`Backend`] and one arm of its `build`;
//! - a new IPC member is one arm of the IPC server's dispatch (a private module:
//!   `src/ipc/server.rs`), or a schema member plus a generated client;
//! - a snapshot consumer is a second [`gnoblin_wayland::Observer`];
//! - requests (prompts, polkit, GeoClue) are the [`gnoblin_model::RequestBroker`] the
//!   dispatcher owns: a privileged connection offers one, the slot registry routes it to
//!   the connection that provides the slot, and both sides learn its one completion.

mod backend;
mod bell;
mod compat;
mod cursor;
mod eis;
mod extensions;
mod ipc;
mod layout;
mod monitors;
/// Portal negotiation, the pure decision layer behind the `org.freedesktop.impl.portal` services.
pub mod portal;
mod renderer;
mod schema;
mod session_tables;
mod window_management;
mod xwayland;

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;

use gnoblin_wayland::{ControlHandle, LoopError, Server, StartupError};

pub use backend::Backend;
pub use bell::{
    configure_from_environment as configure_bell_from_environment, configure_from_store as configure_bell_from_store,
};
pub use compat::configure_compat_extensions;
pub use cursor::{
    configure_from_environment as configure_cursor_from_environment,
    configure_from_store as configure_cursor_from_store,
};
pub use eis::EisGate;
pub use extensions::{configure_from_environment, configure_from_store};
pub use gnoblin_config::{LogConsole, Logging, logging_from_environment};
pub use gnoblin_types::Version;
pub use gnoblin_wayland::{Extension, Observer, SessionSnapshot};
pub use gnoblin_xwayland::xsettings::XSettings;
pub use ipc::{ExtensionRoute, IpcError, Privileged, SessionSync, guardian_from_environment};
pub use layout::{
    LayoutSettings, configure_from_environment as configure_layouts_from_environment,
    configure_from_store as configure_layouts_from_store,
};
pub use monitors::{
    configure_from_environment as configure_monitors_from_environment,
    configure_from_store as configure_monitors_from_store,
};
pub use renderer::{
    configure_from_environment as configure_renderer_from_environment,
    configure_from_store as configure_renderer_from_store,
};
pub use schema::catalog;
pub use session_tables::{
    configure_from_environment as configure_session_tables_from_environment,
    configure_from_store as configure_session_tables_from_store,
};
pub use window_management::{
    configure_from_environment as configure_window_management_from_environment,
    configure_from_store as configure_window_management_from_store,
};
pub use xwayland::{
    configure_from_environment as configure_xwayland_from_environment,
    configure_from_store as configure_xwayland_from_store,
};

use gnoblin_backend::nested::NestedError;
use gnoblin_model::ModelError;
use gnoblin_schema::SchemaError;

gnoblin_log::logger!(log = "compositor");

/// How a compositor session is configured.
///
/// Not `Clone`: a session's configuration owns the privileged connections it was handed,
/// and duplicating a descriptor would give two owners of one socket.
#[derive(Debug, Default)]
pub struct SessionConfig {
    /// The backend the session drives.
    pub backend: Backend,
    /// The renderer the `compositor.renderer` setting names, or [`None`] for `auto`, which is the
    /// renderer the backend draws with by default.
    ///
    /// [`configure_renderer_from_store`] fills it from the accepted document. A renderer the
    /// backend cannot use is refused at startup with [`Error::RendererRefused`].
    pub renderer: Option<gnoblin_config::RendererChoice>,
    /// The X11 session, or [`None`] for a session that starts no X server.
    ///
    /// [`None`] is the default. [`Some`] starts Xwayland under `gnoblin-xwayland`'s
    /// supervisor with these settings, and the compositor ticks it and owns its window policy.
    pub xwayland: Option<gnoblin_xwayland::Settings>,
    /// The XSETTINGS owner the X11 session publishes to X11 clients, or [`None`] for none.
    ///
    /// Only read when [`Self::xwayland`] is [`Some`]: without an X server there is no root window to
    /// publish on. The owner is asked for its values once the X11 window manager is up.
    pub xsettings: Option<Box<dyn gnoblin_xwayland::xsettings::XSettings>>,
    /// How the Wayland session itself is configured: its socket, seat, workspaces
    /// and window management options.
    pub wayland: gnoblin_wayland::Config,
    /// The IPC socket, or [`None`] for `$XDG_RUNTIME_DIR/gnoblin/ipc.sock`.
    ///
    /// A test passes its own path so the run touches no shared directory.
    pub ipc_socket: Option<PathBuf>,
    /// The EIS endpoint's consent gate. The endpoint is served at `eis.sock` beside the IPC
    /// socket, and every connection on it waits until the gate grants it input.
    pub eis: EisGate,
    /// A handle a supervisor or a test stops the session with.
    pub stop: Option<StopHandle>,
    /// The privileged roles this process serves: which roles exist and what each role's
    /// socket advertises. [`ipc::Privileged::declared`] builds it. The connections arrive
    /// over [`SessionConfig::guardian`], one handover per privileged child.
    pub privileged: Vec<ipc::Privileged>,
    /// The guardian's control socket, from which privileged connections arrive as handovers.
    ///
    /// The guardian keeps this open for the whole session and sends one handover for each
    /// privileged child it starts, naming the role the child was made for. The binary role
    /// reads it with [`guardian_from_environment`]. `None` means there is no guardian, so no
    /// privileged connection is ever accepted; a test passes one end of a socket pair it made.
    pub guardian: Option<UnixStream>,
    /// The extensions this session serves, each with the role that provides it.
    ///
    /// A namespace is routed to its provider connection and granted to the privileged
    /// connections that list it. An extension that fails to load is absent from this list.
    pub extensions: Vec<ipc::ExtensionRoute>,
    /// The `layout` section: which layout each workspace runs. The default floats every
    /// workspace; [`configure_layouts_from_store`] fills it from the accepted document.
    pub layouts: LayoutSettings,
}

/// A handle that stops a running session from another thread.
///
/// A [`gnoblin_wayland::Server`] is created and run on one thread, so a supervisor
/// that started the process in another thread cannot hold its
/// [`ControlHandle`]. This is the seam: the handle is a request, and the run's own
/// watcher resolves it into `ControlHandle::stop`, which is the only way the loop
/// is ever stopped.
#[derive(Clone, Debug)]
pub struct StopHandle {
    requested: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
}

impl Default for StopHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl StopHandle {
    /// A handle that has not been asked to stop.
    #[must_use]
    pub fn new() -> StopHandle {
        StopHandle {
            requested: Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new())),
        }
    }

    /// Asks the session to stop after its current turn. It is safe to call more than
    /// once, and from any thread.
    pub fn request(&self) {
        let (requested, wake) = &*self.requested;
        let mut guard = requested.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // The flag is set under the lock and the waiter is woken after it, so a waiter that
        // checks the flag before this call cannot miss the wake-up.
        *guard = true;
        wake.notify_all();
    }

    /// Whether a stop has been requested.
    #[must_use]
    pub fn requested(&self) -> bool {
        let (requested, _) = &*self.requested;
        *requested.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Blocks until a stop is requested.
    fn wait(&self) {
        let (requested, wake) = &*self.requested;
        let mut guard = requested.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while !*guard {
            guard = wake.wait(guard).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
}

/// The Gnoblin version this build serves, which is its IPC version.
///
/// It is the schema's, not the package's: the plan makes the IPC version exactly the
/// Gnoblin version, and a candidate build (`0.2.0-alpha.N`) advertises the triple of
/// the release it becomes.
#[must_use]
pub fn version() -> Version {
    let (major, minor, patch) = gnoblin_proto::IPC_VERSION;
    Version::new(major, minor, patch)
}

/// The compositor process.
pub struct Compositor;

impl Compositor {
    /// Runs a session until it stops.
    ///
    /// The Wayland loop runs on the calling thread — a calloop loop and the backend
    /// it drives cannot move between threads — so this returns only when the session
    /// does. A supervisor that needs the handle back passes a [`StopHandle`] in
    /// [`SessionConfig::stop`].
    ///
    /// # Errors
    ///
    /// [`Error::Schema`] when the embedded schema cannot be loaded,
    /// [`Error::Ipc`] when the socket cannot be prepared or bound,
    /// [`Error::BackendRefused`] names a backend this build does not carry, and
    /// [`Error::Nested`] a parent compositor that cannot host the session. [`Error::Wayland`]
    /// when the Wayland session cannot start, and [`Error::Loop`] when the running loop fails.
    pub fn run(config: SessionConfig) -> Result<(), Error> {
        // The schema is embedded at build time (see `schema.rs`), so a failure here is a
        // problem with this build rather than with the host configuration.
        let catalog = schema::catalog().map_err(Error::Schema)?;
        let version = version();
        let mut store = SessionSync::new(catalog, version).map_err(Error::Model)?;

        let path = match &config.ipc_socket {
            Some(path) => path.clone(),
            None => gnoblin_client::unix::socket_path(|name| std::env::var(name).ok()).map_err(Error::IpcPath)?,
        };

        // Everything the socket has to be — its directory, its mode, no live
        // compositor — is settled before a Wayland session exists to tear down, and
        // the socket is bound before the loop starts so a shell that sees the file can
        // connect.
        let prepared = ipc::SocketPath::prepare(&path, version).map_err(Error::Ipc)?;
        let (events, received) = ipc::events();
        let (observer, pending) = ipc::wake(events.clone());

        // The display name a client puts in `WAYLAND_DISPLAY` is the Wayland socket's file
        // name, and it has to be read before the configuration is moved into the server.
        let display_name = config
            .wayland
            .socket_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map_or_else(
                || gnoblin_wayland::DEFAULT_SOCKET_NAME.to_owned(),
                |name| name.to_string_lossy().into_owned(),
            );
        let backend_name = config.backend.name();
        let backend = config
            .backend
            .build(config.renderer.unwrap_or(gnoblin_config::RendererChoice::Auto))?;
        let wayland = gnoblin_wayland::Config {
            observer: Some(observer),
            ..config.wayland
        };
        let mut server = match config.xwayland {
            // No X11: the server has no X server, no grab manager and no process.
            None => Server::new(backend, wayland),
            Some(settings) => {
                let supervisor =
                    gnoblin_xwayland::Xwayland::with_settings(settings, std::env::var_os("PATH"), config.xsettings);
                let (handle, windows) = supervisor.parts();
                let session = gnoblin_wayland::XwaylandSession::new(Box::new(handle), Box::new(windows));
                Server::with_xwayland(backend, wayland, session)
            }
        }
        .map_err(Error::Wayland)?;
        let layouts = std::rc::Rc::new(layout::WorkspaceLayouts::new(config.layouts));
        server.install_layouts(layouts.clone());
        let control = server.control();
        let observation = server.observation();

        // The EIS endpoint is bound first: the IPC socket is what a client connects to, and the
        // session's ready signal goes out with it, so a client that sees either looks for
        // `eis.sock` straight away and has to find it. Every connection on it waits for a grant.
        let eis_gate = config.eis.clone();
        eis_gate
            .serve(&eis::socket_path(&path), control.input_sink())
            .map_err(Error::Eis)?;
        store.ready();

        let ipc = ipc::IpcServer::start(
            prepared,
            store,
            events,
            received,
            pending,
            observation,
            control.clone(),
            config.privileged,
            config.guardian,
            &config.extensions,
        )
        .map_err(Error::Ipc)?;

        // The guardian waits for this report before it starts anything else: the session's
        // later children need the display, and the plan ends the session when a required
        // variable is missing.
        let mut reporter = gnoblin_client::Reporter::new(|name| std::env::var(name).ok());
        reporter.ready(&[
            ("WAYLAND_DISPLAY".to_owned(), display_name.clone()),
            ("GNOBLIN_IPC_SOCKET".to_owned(), path.display().to_string()),
        ]);
        log::info!(socket = %path.display(),
            display = %display_name,
            version = %version,
            backend = backend_name,
            "serving"
        );
        // The watcher is joined below, and it only returns once a stop is requested. The stop
        // is therefore requested after the loop ends, even when the loop stopped on its own.
        let stop = config.stop.clone();
        let watcher = spawn_watcher(stop.clone(), control);

        // The loop's result is kept and returned after cleanup, so a loop error still stops the
        // EIS gate and shuts the IPC server down first.
        let loop_result = server.run();
        eis_gate.stop();
        ipc.shutdown();
        if let Some(watcher) = watcher {
            // The watcher waits for exactly this: a stop it can then resolve.
            if let Some(stop) = &stop {
                stop.request();
            }
            let _ = watcher.join();
        }
        // The socket file is removed after both threads have stopped. A removal error is
        // ignored because the file may already be gone.
        let _ = std::fs::remove_file(&path);
        loop_result.map_err(Error::Loop)
    }
}

/// Why a session could not start or did not stop cleanly.
#[derive(Debug)]
pub enum Error {
    /// The embedded schema could not be loaded or is not valid.
    Schema(SchemaError),
    /// The EIS socket could not be bound.
    Eis(std::io::Error),
    /// No IPC socket path could be resolved from the environment.
    IpcPath(String),
    /// The IPC socket could not be prepared or bound.
    Ipc(IpcError),
    /// The Wayland session could not start.
    Wayland(StartupError),
    /// The running Wayland loop failed.
    Loop(LoopError),
    /// The object graph refused the session object, which means the schema and this
    /// crate disagree about it.
    Model(ModelError),
    /// The nested backend could not connect to its parent compositor.
    Nested(NestedError),
    /// The selected backend is not carried by this build. It is named so the refusal says
    /// which backend it refused, rather than starting a different one in its place.
    BackendRefused {
        /// The backend's name, as [`Backend::name`] reports it.
        backend: &'static str,
        /// Why this build cannot start it.
        reason: &'static str,
    },
    /// The renderer the `compositor.renderer` setting names cannot be used by the selected
    /// backend. The setting is named, and the session does not start with another renderer in
    /// its place.
    RendererRefused {
        /// The setting, as the configuration spells it.
        leaf: &'static str,
        /// The renderer the setting names.
        renderer: &'static str,
        /// The backend's name, as [`Backend::name`] reports it.
        backend: &'static str,
        /// Why the backend cannot draw with it.
        reason: &'static str,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Schema(error) => write!(formatter, "the object schema cannot be loaded: {error}"),
            Error::IpcPath(reason) => write!(formatter, "{reason}"),
            Error::Ipc(error) => write!(formatter, "{error}"),
            Error::Eis(error) => write!(formatter, "the EIS socket cannot be served: {error}"),
            Error::Wayland(error) => write!(formatter, "the Wayland session cannot start: {error}"),
            Error::Loop(error) => write!(formatter, "the Wayland session stopped: {error}"),
            Error::Model(error) => write!(formatter, "the object graph refused the session: {error}"),
            Error::Nested(error) => write!(formatter, "the nested session cannot start: {error}"),
            Error::BackendRefused { backend, reason } => {
                write!(formatter, "the {backend} backend cannot start: {reason}")
            }
            Error::RendererRefused {
                leaf,
                renderer,
                backend,
                reason,
            } => write!(
                formatter,
                "{leaf} = {renderer}: the {backend} backend cannot draw with it: {reason}"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// A watcher that turns a [`StopHandle`] into the loop's own stop.
///
/// It exists because the server is created and run on this thread: the handle is
/// the only part of the session another thread may hold, so it is resolved here into
/// the [`ControlHandle`] the loop answers.
fn spawn_watcher(stop: Option<StopHandle>, control: ControlHandle) -> Option<JoinHandle<()>> {
    let stop = stop?;
    Some(std::thread::spawn({
        let stop = stop.clone();
        move || {
            stop.wait();
            control.stop();
        }
    }))
}
