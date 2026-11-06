//! The two real peers the integration tests need, and one running session.
//!
//! Nothing here is a mock. The session is [`Compositor::run`] on a headless backend
//! with both of its sockets in a temporary directory; the Wayland peer is a real
//! `wayland-client` connection that creates and maps an `xdg_toplevel`; the IPC peer
//! is the real `gnoblinctl` library entry point, called the way the binary calls it.
//! That is the arrangement the plan describes — one compositor process, one client
//! process, one short-lived CLI — with the two clients in threads instead of
//! processes because the tests assert on their in-process results.
//!
//! # Why the session runs in a thread
//!
//! A calloop loop and the backend it drives cannot move between threads, so
//! [`Compositor::run`] owns the thread that calls it. The tests are therefore the
//! supervisor: they start it in a thread with a [`StopHandle`] and stop it when the
//! body finishes.

#![allow(dead_code)]

use std::io::Write as _;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gnoblin_compositor::{Compositor, EisGate, SessionConfig, StopHandle};
use gnoblinctl::SocketConnection;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection as WaylandConnection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// How long a test waits for a state before failing.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// A temporary directory with the mode the IPC socket requires.
///
/// `tempfile` creates its directories `0755`, which is the mode the compositor
/// refuses on purpose, so a test that wants a session makes the directory private the
/// way `$XDG_RUNTIME_DIR` is.
pub fn private_directory() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("a temporary directory");
    std::fs::set_permissions(
        directory.path(),
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
    )
    .expect("the directory mode");
    directory
}

/// A compositor session on sockets of its own.
pub struct Session {
    /// Kept alive for the session's lifetime: dropping it would unlink both sockets.
    directory: tempfile::TempDir,
    ipc: PathBuf,
    wayland: PathBuf,
    stop: StopHandle,
    eis: EisGate,
    thread: Option<JoinHandle<Result<(), gnoblin_compositor::Error>>>,
}

impl Session {
    /// Starts a session whose IPC socket is `ipc` and whose Wayland socket is
    /// `wayland`, both inside a temporary directory this test owns.
    pub fn start() -> Session {
        Session::start_with(None)
    }

    /// Starts a session that was handed one privileged connection per role, and returns
    /// the ends the roles would hold. Each role's socket grants nothing.
    ///
    /// The pair is the guardian's, exactly as `design/rust-compositor.md`'s process model
    /// describes it: this test makes it because this test is the guardian.
    pub fn start_privileged(roles: &[&str]) -> (Session, Vec<(String, UnixStream)>) {
        let granted: Vec<(&str, Vec<gnoblin_proto::Grant>)> = roles.iter().map(|role| (*role, Vec::new())).collect();
        Session::start_privileged_granting(&granted)
    }

    /// Like [`Session::start_privileged`], but each role's socket advertises its own
    /// extension grants after every accept.
    pub fn start_privileged_granting(
        roles: &[(&str, Vec<gnoblin_proto::Grant>)],
    ) -> (Session, Vec<(String, UnixStream)>) {
        Session::start_privileged_extensions(roles, &[])
    }

    /// Like [`Session::start_privileged_granting`], and also serves `routes` as the
    /// session's loaded extensions, each provided by the role its route names.
    pub fn start_privileged_extensions(
        roles: &[(&str, Vec<gnoblin_proto::Grant>)],
        routes: &[gnoblin_compositor::ExtensionRoute],
    ) -> (Session, Vec<(String, UnixStream)>) {
        let routes = routes.to_vec();
        Session::start_privileged_configured(roles, move |config| config.extensions = routes)
    }

    /// Starts a session whose privileged connections are `roles`, and lets `configure` set
    /// the rest of its [`SessionConfig`] before the compositor runs. `configure` sees the
    /// privileged connections already in place, so it can load extensions against them.
    pub fn start_privileged_configured(
        roles: &[(&str, Vec<gnoblin_proto::Grant>)],
        configure: impl FnOnce(&mut SessionConfig),
    ) -> (Session, Vec<(String, UnixStream)>) {
        let directory = private_directory();
        let ipc = directory.path().join("ipc.sock");
        let wayland = directory.path().join("wayland.sock");
        let mut privileged = Vec::new();
        let mut ends = Vec::new();
        for (role, grants) in roles {
            let (compositor_end, role_end) = UnixStream::pair().expect("a socket pair");
            privileged.push(gnoblin_compositor::Privileged {
                role: (*role).to_owned(),
                stream: compositor_end,
                extension_grants: grants.clone(),
            });
            ends.push(((*role).to_owned(), role_end));
        }
        let stop = StopHandle::new();
        let eis = EisGate::default();
        let mut config = SessionConfig {
            ipc_socket: Some(ipc.clone()),
            stop: Some(stop.clone()),
            eis: eis.clone(),
            wayland: gnoblin_wayland::Config {
                socket_path: Some(wayland.clone()),
                ..gnoblin_wayland::Config::default()
            },
            privileged,
            ..SessionConfig::default()
        };
        configure(&mut config);
        let thread = std::thread::Builder::new()
            .name("compositor".to_owned())
            .spawn(move || Compositor::run(config))
            .expect("the session thread");
        let mut session = Session {
            directory,
            ipc,
            wayland,
            stop,
            eis,
            thread: Some(thread),
        };
        session.wait_until_listening();
        (session, ends)
    }

    /// Starts a session that serves `ipc` instead of the directory's default.
    pub fn start_with(ipc: Option<PathBuf>) -> Session {
        Session::start_with_window_process(ipc, None)
    }

    /// Like [`Session::start_with`], but every window reports `process` as its process
    /// instead of reading its client's connection. The in-process test client's pid is
    /// the test's own, which `window.kill` refuses, so a kill test names its victim here.
    /// Passing `Some(WindowProcess::Unknown)` reports the process as unknown.
    pub fn start_with_window_process(ipc: Option<PathBuf>, process: Option<gnoblin_wayland::WindowProcess>) -> Session {
        let directory = private_directory();
        let ipc = ipc.unwrap_or_else(|| directory.path().join("ipc.sock"));
        let wayland = directory.path().join("wayland-0.sock");
        let stop = StopHandle::new();
        let eis = EisGate::default();
        let config = SessionConfig {
            ipc_socket: Some(ipc.clone()),
            stop: Some(stop.clone()),
            eis: eis.clone(),
            wayland: gnoblin_wayland::Config {
                socket_path: Some(wayland.clone()),
                window_process_override: process,
                ..gnoblin_wayland::Config::default()
            },
            ..SessionConfig::default()
        };
        let thread = std::thread::Builder::new()
            .name("compositor".to_owned())
            .spawn(move || Compositor::run(config))
            .expect("the session thread");
        let mut session = Session {
            directory,
            ipc,
            wayland,
            stop,
            eis,
            thread: Some(thread),
        };
        session.wait_until_listening();
        session
    }

    /// The session's EIS gate, which grants or refuses input from EIS clients.
    pub fn eis(&self) -> &EisGate {
        &self.eis
    }

    /// The IPC socket `gnoblinctl` connects to.
    pub fn socket(&self) -> &Path {
        &self.ipc
    }

    /// The Wayland socket a client connects to.
    pub fn wayland(&self) -> &Path {
        &self.wayland
    }

    /// The directory both sockets live in.
    pub fn directory(&self) -> &Path {
        self.directory.path()
    }

    /// Waits until the IPC socket answers a connection.
    ///
    /// The socket is bound before the Wayland loop starts, so this is a wait for the
    /// server's thread to reach its bind, not for the loop.
    pub fn wait_until_listening(&mut self) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if UnixStream::connect(&self.ipc).is_ok() {
                return;
            }
            if self.thread.as_ref().is_some_and(std::thread::JoinHandle::is_finished) {
                let result = self.join();
                panic!("the compositor stopped before it listened: {result:?}");
            }
            assert!(
                Instant::now() < deadline,
                "the compositor never listened on {}",
                self.ipc.display()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Stops the session and reports how it ended.
    pub fn stop(mut self) -> Result<(), gnoblin_compositor::Error> {
        self.stop.request();
        self.join()
    }

    fn join(&mut self) -> Result<(), gnoblin_compositor::Error> {
        let Some(thread) = self.thread.take() else {
            return Ok(());
        };
        match thread.join() {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Runs one `gnoblinctl` command against the session's IPC socket.
    pub fn ctl(&self, arguments: &[&str]) -> Command {
        Command::run(&self.ipc, arguments)
    }

    /// Connects a real IPC client that speaks the wire protocol by hand.
    pub fn wire(&self) -> Wire {
        Wire::connect(&self.ipc)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.stop.request();
            let _ = self.join();
        }
    }
}

/// A configuration for a session rooted at `directory`, with both sockets named.
pub fn config(directory: &Path, ipc: PathBuf) -> SessionConfig {
    SessionConfig {
        ipc_socket: Some(ipc),
        wayland: gnoblin_wayland::Config {
            socket_path: Some(directory.join("wayland.sock")),
            ..gnoblin_wayland::Config::default()
        },
        ..SessionConfig::default()
    }
}

/// Runs a compositor that must be refused before it serves anything.
///
/// The refusal happens while the socket is prepared, which is before the Wayland
/// session exists, so this returns instead of needing a thread and a stop.
#[track_caller]
pub fn run_refused(config: SessionConfig) -> gnoblin_compositor::Error {
    match Compositor::run(config) {
        Ok(()) => panic!("the compositor should have been refused"),
        Err(error) => error,
    }
}

/// The result of one `gnoblinctl` run.
#[derive(Debug)]
pub struct Command {
    pub code: u8,
    pub out: String,
    pub err: String,
}

impl Command {
    /// Runs the real library entry point, exactly as `gnoblinctl`'s `main` does.
    pub fn run(socket: &Path, arguments: &[&str]) -> Command {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = gnoblinctl::run(
            arguments.iter().map(std::ffi::OsString::from),
            SocketConnection::at(socket),
            &mut out,
            &mut err,
        );
        Command {
            code,
            out: String::from_utf8_lossy(&out).into_owned(),
            err: String::from_utf8_lossy(&err).into_owned(),
        }
    }

    /// The run must have done what it was asked.
    #[track_caller]
    pub fn ok(self) -> Command {
        assert_eq!(self.code, 0, "gnoblinctl exited {}: {}", self.code, self.err);
        self
    }
}

/// How long [`Wire::receive`] waits for a frame the session has yet to write.
const FRAME_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// One raw IPC connection, for the frames a client library would not send.
pub struct Wire {
    stream: UnixStream,
    reader: std::io::BufReader<UnixStream>,
}

impl Wire {
    /// Connects and completes the handshake at `version`.
    ///
    /// Returns the socket whether the handshake was accepted or refused, so a test
    /// can read the refusal frame. A refusal is followed by a closed connection.
    pub fn connect_version(path: &Path, version: gnoblin_proto::Version) -> (Wire, gnoblin_proto::Handshake) {
        let stream = UnixStream::connect(path).expect("the session is listening");
        let reader = std::io::BufReader::new(stream.try_clone().expect("a duplicated socket"));
        let mut wire = Wire { stream, reader };
        let hello = gnoblin_proto::Handshake::hello(version);
        {
            let mut writer = &wire.stream;
            writer.write_all(&hello.encode()).expect("the hello");
            writer.flush().expect("the hello");
        }
        let answer = wire.read_handshake();
        if answer.kind == gnoblin_proto::HandshakeKind::Accept {
            // The grants frame follows every accept; a reader of envelopes reads past it.
            gnoblin_proto::read_grants(&mut wire.reader).expect("the grants frame after the accept");
        }
        (wire, answer)
    }

    /// Connects at the version this build serves.
    pub fn connect(path: &Path) -> Wire {
        let (wire, answer) = Wire::connect_version(path, gnoblin_compositor::version());
        assert_eq!(
            answer.kind,
            gnoblin_proto::HandshakeKind::Accept,
            "the session refused this build"
        );
        wire
    }

    /// Completes the handshake on a connection the test already holds, such as a
    /// privileged role's end, and returns the wire ready for envelopes.
    pub fn over(stream: UnixStream) -> Wire {
        let reader = std::io::BufReader::new(stream.try_clone().expect("a duplicated socket"));
        let mut wire = Wire { stream, reader };
        {
            let mut writer = &wire.stream;
            let hello = gnoblin_proto::Handshake::hello(gnoblin_compositor::version());
            writer.write_all(&hello.encode()).expect("the hello");
            writer.flush().expect("the hello");
        }
        let answer = wire.read_handshake();
        assert_eq!(
            answer.kind,
            gnoblin_proto::HandshakeKind::Accept,
            "the session refused this role"
        );
        gnoblin_proto::read_grants(&mut wire.reader).expect("the grants frame after the accept");
        wire
    }

    /// Makes [`Wire::receive_within`] wait at most `limit` for each frame.
    pub fn set_read_timeout(&self, limit: std::time::Duration) {
        self.stream.set_read_timeout(Some(limit)).expect("a read deadline");
    }

    /// Reads one frame, or `None` when none arrives before the deadline set by
    /// [`Wire::set_read_timeout`]. A closed connection or a bad frame still panics.
    pub fn receive_within(&mut self) -> Option<gnoblin_proto::Envelope> {
        match gnoblin_proto::read_frame(&mut self.reader) {
            Ok(envelope) => envelope,
            Err(gnoblin_proto::CodecError::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                None
            }
            Err(error) => panic!("a bad frame: {error:?}"),
        }
    }

    fn read_handshake(&mut self) -> gnoblin_proto::Handshake {
        let mut frame = [0u8; gnoblin_proto::HANDSHAKE_LEN];
        std::io::Read::read_exact(&mut self.reader, &mut frame).expect("a handshake");
        gnoblin_proto::Handshake::decode(&frame).expect("a valid handshake")
    }

    /// Sends one frame.
    pub fn send(&mut self, envelope: &gnoblin_proto::Envelope) {
        gnoblin_proto::write_frame(&mut self.stream, envelope).expect("a frame");
    }

    /// Sends raw bytes, as a client with a bug would.
    pub fn send_bytes(&mut self, bytes: &[u8]) {
        let mut writer = &self.stream;
        writer.write_all(bytes).expect("raw bytes");
        writer.flush().expect("raw bytes");
    }

    /// Reads one frame, or `None` when the session closed the connection.
    ///
    /// Waits up to [`FRAME_WAIT`] for the frame, so a reply the session has not
    /// written yet is waited for. Panics naming the missing frame if none arrives.
    pub fn receive(&mut self) -> Option<gnoblin_proto::Envelope> {
        self.await_frame(FRAME_WAIT);
        gnoblin_proto::read_frame(&mut self.reader).expect("a decodable frame")
    }

    /// Blocks until the reader holds bytes or the connection has closed, polling in
    /// short steps. `fill_buf` consumes nothing, so a timed-out poll never loses
    /// part of a frame the way a `read_exact` interrupted mid-frame would.
    fn await_frame(&mut self, limit: std::time::Duration) {
        const POLL: std::time::Duration = std::time::Duration::from_millis(20);
        let previous = self.stream.read_timeout().expect("the read deadline");
        self.stream.set_read_timeout(Some(POLL)).expect("a poll deadline");
        let deadline = std::time::Instant::now() + limit;
        loop {
            match std::io::BufRead::fill_buf(&mut self.reader) {
                Ok(_) => break,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    if std::time::Instant::now() >= deadline {
                        self.stream.set_read_timeout(previous).expect("the read deadline");
                        panic!("no frame arrived within {limit:?}: the session did not write the expected reply");
                    }
                }
                Err(error) => panic!("a frame: {error}"),
            }
        }
        self.stream.set_read_timeout(previous).expect("the read deadline");
    }

    /// Drops the connection the way a killed process does.
    pub fn kill(self) {
        drop(self);
    }
}

/// A real Wayland client for one connection.
pub struct Client {
    /// The client owns its connection, so dropping the client closes the socket — which
    /// is what a client that exits looks like to the compositor.
    _connection: WaylandConnection,
    queue: EventQueue<App>,
    app: App,
    qh: QueueHandle<App>,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    wm_base: xdg_wm_base::XdgWmBase,
    #[allow(dead_code)]
    seat: wl_seat::WlSeat,
}

/// One toplevel's protocol objects.
pub struct Window {
    pub surface: wl_surface::WlSurface,
    xdg_surface: xdg_surface::XdgSurface,
    toplevel: xdg_toplevel::XdgToplevel,
}

/// What the client records from the events it receives.
#[derive(Default)]
pub struct App {
    pub configures: u32,
    pub acks: u32,
    /// The size of the last `xdg_toplevel.configure` each toplevel received, keyed by its
    /// protocol id. A zero side means the client chooses that size.
    pub sizes: std::collections::HashMap<u32, (i32, i32)>,
    /// How many times the keyboard entered one of this client's surfaces.
    pub keyboard_enters: u32,
    /// Keys as `(code, state)`, the protocol's own numbers.
    pub keys: Vec<(u32, u32)>,
}

impl Client {
    /// Connects and binds the globals a toplevel needs.
    pub fn connect(socket: &Path) -> Client {
        let stream = UnixStream::connect(socket).expect("the session is listening");
        let connection = WaylandConnection::from_socket(stream).expect("a client connection");
        let (globals, queue) = registry_queue_init::<App>(&connection).expect("the global list");
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor = globals
            .bind(&qh, 1..=1, ())
            .expect("the session advertises wl_compositor");
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).expect("the session advertises wl_shm");
        let wm_base: xdg_wm_base::XdgWmBase = globals
            .bind(&qh, 1..=1, ())
            .expect("the session advertises xdg_wm_base");
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).expect("the session advertises wl_seat");
        Client {
            _connection: connection,
            queue,
            app: App::default(),
            qh,
            compositor,
            shm,
            wm_base,
            seat,
        }
    }

    /// Processes every event the session has sent.
    pub fn roundtrip(&mut self) {
        self.queue.roundtrip(&mut self.app).expect("a round trip");
    }

    /// The size of `window`'s last configure, or `None` when it has not been configured.
    /// The keys this client's keyboard received, as `(code, state)`.
    pub fn keys(&self) -> &[(u32, u32)] {
        &self.app.keys
    }

    pub fn configured_size(&self, window: &Window) -> Option<(i32, i32)> {
        self.app.sizes.get(&window.toplevel.id().protocol_id()).copied()
    }

    /// Creates a toplevel, asks for a configure and acknowledges it.
    pub fn create_window(&mut self, app_id: &str, title: &str) -> Window {
        let surface = self.compositor.create_surface(&self.qh, ());
        let xdg_surface = self.wm_base.get_xdg_surface(&surface, &self.qh, ());
        let toplevel = xdg_surface.get_toplevel(&self.qh, ());
        toplevel.set_app_id(app_id.to_owned());
        toplevel.set_title(title.to_owned());
        surface.commit();
        self.roundtrip();
        Window {
            surface,
            xdg_surface,
            toplevel,
        }
    }

    /// Attaches a shared-memory buffer and commits, which maps the window.
    pub fn attach_buffer(&mut self, window: &Window, width: i32, height: i32) {
        let stride = width * 4;
        let length = stride * height;
        let file = tempfile::tempfile().expect("a shared memory file");
        file.set_len(length as u64).expect("a buffer that big");
        let pool = self.shm.create_pool(file.as_fd(), length, &self.qh, ());
        let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, &self.qh, ());
        window.surface.attach(Some(&buffer), 0, 0);
        window.surface.damage(0, 0, width, height);
        window.surface.commit();
        self.roundtrip();
    }

    /// Creates a toplevel and maps it with a buffer.
    pub fn map_window(&mut self, app_id: &str, title: &str) -> Window {
        let window = self.create_window(app_id, title);
        self.attach_buffer(&window, 200, 150);
        window
    }

    /// Changes the title the way a client does once it has a document name.
    pub fn set_title(&mut self, window: &Window, title: &str) {
        window.toplevel.set_title(title.to_owned());
        // A title needs a commit to be applied.
        window.surface.commit();
        self.roundtrip();
    }

    /// Closes a window the way a well-behaved client does.
    pub fn destroy(&mut self, window: Window) {
        window.toplevel.destroy();
        window.xdg_surface.destroy();
        window.surface.destroy();
        self.roundtrip();
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_compositor::WlCompositor,
        _: <wl_compositor::WlCompositor as Proxy>::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_surface::WlSurface,
        _: <wl_surface::WlSurface as Proxy>::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm::WlShm, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_shm::WlShm,
        _: <wl_shm::WlShm as Proxy>::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_shm_pool::WlShmPool,
        _: <wl_shm_pool::WlShmPool as Proxy>::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_buffer::WlBuffer,
        _: <wl_buffer::WlBuffer as Proxy>::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for App {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: <wl_registry::WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for App {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: <xdg_wm_base::XdgWmBase as Proxy>::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for App {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: <xdg_surface::XdgSurface as Proxy>::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            state.configures += 1;
            xdg_surface.ack_configure(serial);
            state.acks += 1;
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for App {
    fn event(
        state: &mut Self,
        toplevel: &xdg_toplevel::XdgToplevel,
        event: <xdg_toplevel::XdgToplevel as Proxy>::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Configure { width, height, .. } = event {
            state.sizes.insert(toplevel.id().protocol_id(), (width, height));
        }
    }
}

/// Waits for `check` to hold, or fails with the last state it saw.
#[track_caller]
pub fn wait_for<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(Instant::now() < deadline, "the session never reached: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The object id `window list` prints for the window titled `title`, if it is there.
pub fn listed_id(session: &Session, title: &str) -> Option<u64> {
    let command = session.ctl(&["window", "list"]).ok();
    command
        .out
        .lines()
        .find(|line| line.contains(&format!("{title:?}")))
        .and_then(|line| line.split('\t').next())
        .and_then(|id| id.parse().ok())
}

/// The object id `window list` prints for the window titled `title`, once it is there.
#[track_caller]
pub fn wait_for_listed_id(session: &Session, title: &str) -> u64 {
    wait_for(&format!("a window titled {title:?} is listed"), || {
        listed_id(session, title)
    })
}

/// Every window title `window list` prints, in order.
pub fn listed_titles(session: &Session) -> Vec<String> {
    let command = session.ctl(&["window", "list"]).ok();
    command
        .out
        .lines()
        .filter_map(|line| {
            // The properties are printed as `name = value` pairs with text quoted, so a
            // title is the quoted value after `title = `.
            let at = line.find("title = ")? + "title = ".len();
            Some(
                line[at..]
                    .split(' ')
                    .next()
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_owned(),
            )
        })
        .collect()
}

impl Dispatch<wl_seat::WlSeat, ()> for App {
    fn event(
        _: &mut Self,
        seat: &wl_seat::WlSeat,
        event: <wl_seat::WlSeat as Proxy>::Event,
        _: &(),
        _: &WaylandConnection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
            && capabilities.contains(wl_seat::Capability::Keyboard)
        {
            seat.get_keyboard(qh, ());
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for App {
    fn event(
        app: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: <wl_keyboard::WlKeyboard as Proxy>::Event,
        _: &(),
        _: &WaylandConnection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { .. } => app.keyboard_enters += 1,
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(value),
                ..
            } => app.keys.push((key, value as u32)),
            _ => {}
        }
    }
}
