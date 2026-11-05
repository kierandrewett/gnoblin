//! Starting a session: the event loop, the socket and the run loop.
//!
//! The server owns three things that must stay separate: the calloop event loop, the
//! compositor state and the backend. They are separate so a backend can be replaced
//! without rebuilding the state, and so the state can be handed to the dispatcher
//! without handing it the loop.
//!
//! ## Why the loop is built this way
//!
//! `Display::dispatch_clients` needs `&mut Display` and `&mut State` at the same
//! time, and `Display` is generic over the state. Smithay's examples solve that with
//! an `unsafe` `get_mut` on a `calloop::generic::Generic<Display<State>>`. This crate
//! keeps the workspace's `unsafe_code = "forbid"` instead:
//!
//! - the display lives in [`State`] and is taken out for the duration of a dispatch
//!   ([`State::dispatch`](crate::State)), which is safe because nothing else can
//!   reach it then;
//! - the loop watches a duplicate of the display's poll file descriptor, which
//!   shares readiness with the original, so the source needs no access to the
//!   display at all.
//!
//! ## The socket
//!
//! The listening socket is a plain `UnixListener` at an explicit path, not
//! `ListeningSocketSource`, so a test can put it in its own temporary directory and
//! no environment variable has to be read or written. A session that wants the usual
//! location passes it in [`Config::socket_path`]; with no path the socket is
//! `gnoblin-wayland.sock` inside `$XDG_RUNTIME_DIR`, which is where clients look.

use std::collections::VecDeque;
use std::io;
use std::os::fd::AsFd;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::ping::{Ping, make_ping};
use smithay::reexports::calloop::{Error as CalloopError, EventLoop, Interest, LoopSignal, Mode, PostAction};
use smithay::reexports::wayland_server::Display;
use smithay::reexports::wayland_server::backend::InitError;
use tracing::{debug, info, warn};

use gnoblin_backend::render::Image;
use gnoblin_backend::{Backend, BackendEvent, Host, InputEvent};
use gnoblin_config::Protocols;
use gnoblin_output::LayoutError;
use gnoblin_types::{OutputId, Rect, WindowId};
use gnoblin_wm::WmOptions;

use crate::access::Role;
use crate::frame::{FrameStats, Screenshots};
use crate::observation::{Observer, SessionSnapshot};
use crate::protocols::compositor::ClientState;
use crate::state::State;

/// How long [`ControlHandle::focus`] waits for the loop's answer.
///
/// A focus request is applied in the loop's next turn, so the wait is a scheduling
/// delay and not work; the limit exists so a stopped loop answers "no" instead of
/// hanging the caller.
pub const FOCUS_TIMEOUT: Duration = Duration::from_secs(2);

/// How a session is configured.
#[derive(Clone, Debug)]
pub struct Config {
    /// Where to listen for clients.
    ///
    /// [`None`] means `gnoblin-wayland.sock` inside `$XDG_RUNTIME_DIR`, which is
    /// where a client looks when it has no explicit socket. A test passes its own
    /// path so the run touches no shared directory.
    pub socket_path: Option<PathBuf>,
    /// Where the session's own processes connect.
    ///
    /// A connection accepted here is recorded as [`crate::access::Role::Privileged`]
    /// and may bind the privileged globals; a connection on the ordinary socket may
    /// not. [`None`] means a session with no privileged path, which is the safe
    /// default: nothing is privileged by accident.
    pub privileged_socket_path: Option<PathBuf>,
    /// The seat name clients see in `wl_seat.name`.
    pub seat_name: String,
    /// How many workspaces to declare. The first one is active.
    pub workspaces: u32,
    /// The window management options from the configuration document.
    pub wm: WmOptions,
    /// A reader outside the loop that is told when the observable state changed.
    ///
    /// It is how the composition root feeds its object graph from the same
    /// snapshot every other reader sees, without a second description of the
    /// window state that could drift from this one.
    pub observer: Option<Arc<dyn Observer>>,
    /// Which globals the session advertises (`gnoblin.configure.protocols`).
    pub protocols: Protocols,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            socket_path: None,
            privileged_socket_path: None,
            seat_name: "seat0".to_owned(),
            workspaces: 1,
            wm: WmOptions::default(),
            observer: None,
            protocols: Protocols::default(),
        }
    }
}

/// A session that is ready to run.
pub struct Server {
    event_loop: EventLoop<'static, State>,
    state: State,
    backend: Box<dyn Backend<State>>,
    socket_path: PathBuf,
    stop: LoopSignal,
    control: Arc<Control>,
    frames: Arc<Mutex<FrameStats>>,
}

impl Server {
    /// Starts a session: globals, outputs, socket and event-loop sources.
    ///
    /// Nothing runs until [`Server::run`], so a caller can read the socket path and
    /// the observation first, which is exactly what a test does.
    ///
    /// # Errors
    ///
    /// [`StartupError::EventLoop`] when the loop or its sources cannot be created,
    /// [`StartupError::Display`] when the Wayland display cannot be created,
    /// [`StartupError::Socket`] when the path cannot be bound,
    /// [`StartupError::Keyboard`] when the seat has no keymap, and
    /// [`StartupError::Layout`] when the backend's outputs cannot be laid out.
    pub fn new(backend: Box<dyn Backend<State>>, config: Config) -> Result<Self, StartupError> {
        Self::start(backend, config, None)
    }

    /// Starts a session that also runs Xwayland.
    ///
    /// The session is a separate argument rather than a [`Config`] field because it is
    /// not a setting: it owns the process, the X11 connection and the window table, and it
    /// is the only thing in the server that cannot be compared or cloned. `None` in
    /// [`Server::new`] means no X11 support at all — no `xwayland-shell-v1`, no keyboard
    /// grab manager, no data device and no process — which is a different statement from
    /// "the server is not running yet", and that is what the session's status reports.
    ///
    /// # Errors
    ///
    /// The same as [`Server::new`].
    pub fn with_xwayland(
        backend: Box<dyn Backend<State>>,
        config: Config,
        xwayland: crate::xwayland::XwaylandSession,
    ) -> Result<Self, StartupError> {
        Self::start(backend, config, Some(xwayland))
    }

    fn start(
        mut backend: Box<dyn Backend<State>>,
        config: Config,
        xwayland: Option<crate::xwayland::XwaylandSession>,
    ) -> Result<Self, StartupError> {
        let event_loop: EventLoop<'static, State> = EventLoop::try_new().map_err(StartupError::EventLoop)?;
        let display: Display<State> = Display::new().map_err(StartupError::Display)?;

        // A duplicate of the poll file descriptor, so the loop source can be
        // `'static` and the display can stay in the state.
        let poll_fd = display.as_fd().try_clone_to_owned().map_err(StartupError::Io)?;

        let state = State::new(
            display,
            backend.outputs(),
            &config.seat_name,
            config.wm,
            config.workspaces,
            xwayland,
            config.observer,
            &config.protocols,
        )?;

        backend.register(&event_loop.handle()).map_err(StartupError::Io)?;

        let socket_path = config.socket_path.unwrap_or_else(default_socket_path);
        let listener = bind(&socket_path)?;
        info!(socket = %socket_path.display(), "[wayland] listening");

        event_loop
            .handle()
            .insert_source(
                Generic::new(listener, Interest::READ, Mode::Level),
                |_, listener, state: &mut State| {
                    accept_clients(listener, state, Role::Unprivileged);
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|error| StartupError::EventLoop(error.into()))?;

        if let Some(path) = &config.privileged_socket_path {
            let privileged = bind(path)?;
            info!(socket = %path.display(), "[wayland] listening for privileged clients");
            event_loop
                .handle()
                .insert_source(
                    Generic::new(privileged, Interest::READ, Mode::Level),
                    |_, listener, state: &mut State| {
                        accept_clients(listener, state, Role::Privileged);
                        Ok(PostAction::Continue)
                    },
                )
                .map_err(|error| StartupError::EventLoop(error.into()))?;
        }

        event_loop
            .handle()
            .insert_source(
                Generic::new(poll_fd, Interest::READ, Mode::Level),
                |_, _, state: &mut State| state.dispatch().map(|_| PostAction::Continue),
            )
            .map_err(|error| StartupError::EventLoop(error.into()))?;

        let (ping, source) = make_ping().map_err(StartupError::Io)?;
        let control = Arc::new(Control {
            queue: Mutex::new(VecDeque::new()),
            ping,
            screenshots: Screenshots::default(),
        });
        event_loop
            .handle()
            .insert_source(source, {
                let control = Arc::clone(&control);
                move |(), _, state: &mut State| {
                    for command in control.drain() {
                        match command {
                            Command::Input(event) => state.input(event),
                            Command::MoveWindow { window, geometry } => state.move_window(window, geometry),
                            Command::SetOpacity { window, opacity } => state.set_window_opacity(window, opacity),
                            Command::Focus { window, reply } => {
                                // The answer is sent after the publish, so a caller that
                                // reads the observation on the strength of it sees what
                                // the focus request did.
                                let _ = reply.send(state.focus_window(window));
                            }
                        }
                    }
                }
            })
            .map_err(|error| StartupError::EventLoop(error.into()))?;

        let stop = event_loop.get_signal();
        Ok(Self {
            event_loop,
            state,
            backend,
            socket_path,
            stop,
            control,
            frames: Arc::new(Mutex::new(FrameStats::default())),
        })
    }

    /// The socket clients connect to.
    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// The running window state, as tests and a diagnostics reader see it.
    #[must_use]
    pub fn observation(&self) -> Arc<Mutex<SessionSnapshot>> {
        self.state.observation()
    }

    /// The compositor state, for a caller that wants to read it before running.
    #[must_use]
    pub fn state(&self) -> &State {
        &self.state
    }

    /// What the loop's frame hook has done, for a test or a diagnostics reader.
    #[must_use]
    pub fn frames(&self) -> Arc<Mutex<FrameStats>> {
        Arc::clone(&self.frames)
    }

    /// A handle that stops the loop, injects input and takes screenshots.
    #[must_use]
    pub fn control(&self) -> ControlHandle {
        ControlHandle {
            stop: self.stop.clone(),
            control: Arc::clone(&self.control),
        }
    }

    /// Runs until the loop is stopped.
    ///
    /// Each iteration flushes the clients, takes the backend's input and state
    /// changes, and runs the frame hook: every output asks the scene for its
    /// damage and draws only when something changed, so an idle iteration costs no
    /// rendering at all.
    ///
    /// # Errors
    ///
    /// Any error the event loop or the Wayland backend reports. A client's protocol
    /// error is not one of them.
    pub fn run(self) -> Result<(), LoopError> {
        let socket_path = self.socket_path.clone();
        let result = {
            let Self {
                mut event_loop,
                mut state,
                mut backend,
                control,
                frames,
                ..
            } = self;
            // The loop's own handle, so a sandbox listener created during a dispatch
            // can be watched from inside the turn that created it, and so the Xwayland
            // session can be ticked from the loop's own timeout.
            let handle = event_loop.handle();
            event_loop.run(Duration::from_millis(100), &mut state, move |state| {
                state.flush();
                // A locker that dies sends no request, so the lock is told to look for
                // a connection that is gone, once per turn.
                state.poll_lock();
                // The Xwayland session is ticked here: the loop's own timeout is the only
                // clock its process needs, and a dead or hung instance is noticed within
                // one turn.
                state.poll_xwayland(&handle, Instant::now());
                state.sync_authority_lock();
                for source in state.take_pending_contexts() {
                    let inserted = handle.insert_source(source, |stream, (), state: &mut State| {
                        if let Err(error) = state.accept_sandbox(stream) {
                            warn!(%error, "[access] a sandboxed client could not be inserted");
                        }
                    });
                    if let Err(error) = inserted {
                        warn!(?error, "[access] a sandbox listener could not be watched");
                    }
                }
                let events = backend.dispatch(state);
                if events.iter().any(is_output_change) {
                    // A hotplug, a removal or a mode change: the whole layout is
                    // recomputed from the connected set, once, rather than patched
                    // per event.
                    state.sync_outputs(backend.outputs());
                }
                crate::frame::render(backend.as_mut(), state, &frames, &control.screenshots);
                // The frame just drawn produced buffer releases and frame callbacks;
                // flushing them now rather than on the next wake is the difference
                // between a client drawing at once and drawing up to a timeout later.
                state.flush();
            })
        };
        let _ = std::fs::remove_file(&socket_path);
        result.map_err(LoopError::Stopped)
    }
}

/// What can stop a running session.
#[derive(Clone)]
pub struct ControlHandle {
    stop: LoopSignal,
    control: Arc<Control>,
}

impl ControlHandle {
    /// Stops the loop after the current iteration.
    pub fn stop(&self) {
        self.stop.stop();
    }

    /// Queues an input event as if the backend had delivered it.
    ///
    /// This is how a test clicks a window and how a parity scenario drives input
    /// without a device: the event takes the same path through `Host::input` that a
    /// libinput event will.
    pub fn inject_input(&self, event: InputEvent) {
        self.control.push(Command::Input(event));
    }

    /// Moves a window through the window manager, as a drag would.
    ///
    /// The event takes the policy's own path, so the work-area constraint, the
    /// configure to the client and the scene's damage are the ones a real drag
    /// produces. It is the seam a test and the parity harness drive movement with
    /// until the input layer computes drag rectangles of its own.
    pub fn move_window(&self, window: WindowId, geometry: Rect) {
        self.control.push(Command::MoveWindow { window, geometry });
    }

    /// Sets a window's scene opacity, as an animation would.
    pub fn set_window_opacity(&self, window: WindowId, opacity: f32) {
        self.control.push(Command::SetOpacity { window, opacity });
    }

    /// Asks the window manager to focus a window, as the IPC's `window.focus` does.
    ///
    /// The request is applied in the loop's next turn through the same event a click
    /// produces, so focus policy still decides: `Some(true)` means the window holds
    /// focus now, `Some(false)` means the policy declined or the window has closed,
    /// and [`None`] means the loop did not answer within [`FOCUS_TIMEOUT`] because it
    /// is stopping.
    #[must_use]
    pub fn focus(&self, window: WindowId) -> Option<bool> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::Focus { window, reply });
        answer.recv_timeout(FOCUS_TIMEOUT).ok()
    }

    /// Takes a picture of what `output` currently shows.
    ///
    /// The request is answered by the frame hook at the end of the next iteration, so
    /// it is the last frame *drawn*, not a fresh one: a screenshot of an idle session
    /// is the screen that is on screen. [`None`] means the loop did not answer within
    /// [`crate::frame::SCREENSHOT_TIMEOUT`], which is what a stopped session does.
    #[must_use]
    pub fn screenshot(&self, output: OutputId) -> Option<Image> {
        self.control.screenshot(output)
    }
}

impl std::fmt::Debug for ControlHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ControlHandle").finish_non_exhaustive()
    }
}

/// One command a control handle queued for the loop.
///
/// They share a queue because they share a wake-up: a test that moves a window and
/// takes a screenshot must see both in the order it asked for them.
#[derive(Clone)]
enum Command {
    /// An input event, as the backend would have delivered it.
    Input(InputEvent),
    /// A window-manager move, as a drag would produce.
    MoveWindow { window: WindowId, geometry: Rect },
    /// A scene opacity, as an animation would produce.
    SetOpacity { window: WindowId, opacity: f32 },
    /// A focus request, and the channel its answer goes back on.
    Focus {
        window: WindowId,
        reply: std::sync::mpsc::SyncSender<bool>,
    },
}

/// The commands a control handle queued, and the ping that wakes the loop for them.
struct Control {
    queue: Mutex<VecDeque<Command>>,
    ping: Ping,
    screenshots: Screenshots,
}

impl Control {
    fn push(&self, command: Command) {
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(command);
        self.ping.ping();
    }

    fn drain(&self) -> Vec<Command> {
        let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        queue.drain(..).collect()
    }

    fn screenshot(&self, output: OutputId) -> Option<Image> {
        self.screenshots.request(output, &self.ping)
    }
}

/// Binds the listening socket.
fn bind(path: &Path) -> Result<UnixListener, StartupError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(StartupError::Socket)?;
    }
    // A socket file left by a session that crashed makes bind fail with
    // `AddrInUse`; removing it first is what every compositor does.
    match std::fs::remove_file(path) {
        Ok(()) => debug!(socket = %path.display(), "[wayland] removed a stale socket"),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(StartupError::Socket(error)),
    }
    let listener = UnixListener::bind(path).map_err(StartupError::Socket)?;
    listener.set_nonblocking(true).map_err(StartupError::Socket)?;
    Ok(listener)
}

/// Whether a backend event changes the connected set or an output's geometry.
fn is_output_change(event: &BackendEvent) -> bool {
    matches!(
        event,
        BackendEvent::OutputConnected(_)
            | BackendEvent::OutputDisconnected(_)
            | BackendEvent::OutputResized { .. }
            // A backend that lost the thing driving it has no outputs left, so the
            // layout is recomputed from an empty connected set.
            | BackendEvent::Disconnected
    )
}

/// Accepts every client that connected since the last wake.
///
/// The role is decided by which socket the connection arrived on and is recorded
/// before the client's first request, so its registry answer already knows.
fn accept_clients(listener: &UnixListener, state: &mut State, role: Role) {
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Err(error) = state.insert_client(stream, Arc::new(ClientState::new(&state.access())), role) {
                    warn!(%error, "[wayland] a client could not be inserted");
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                warn!(%error, "[wayland] accept failed");
                return;
            }
        }
    }
}

/// The socket a session listens on when the caller names none.
fn default_socket_path() -> PathBuf {
    let directory = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    directory.join("gnoblin-wayland.sock")
}

/// Why a session could not start.
#[derive(Debug)]
pub enum StartupError {
    /// The event loop or one of its sources could not be created.
    EventLoop(CalloopError),
    /// A socket or receive buffer could not be created or duplicated.
    Io(io::Error),
    /// The Wayland display could not be created.
    Display(InitError),
    /// The listening socket could not be bound or cleaned up.
    Socket(io::Error),
    /// The keyboard keymap could not be built, so the seat cannot have a keyboard.
    Keyboard(smithay::input::keyboard::Error),
    /// The outputs could not be laid out.
    Layout(LayoutError),
}

impl std::fmt::Display for StartupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EventLoop(error) => write!(formatter, "the event loop could not be created: {error}"),
            Self::Io(error) => write!(formatter, "a socket or buffer could not be created: {error}"),
            Self::Display(error) => write!(formatter, "the Wayland display could not be created: {error:?}"),
            Self::Socket(error) => write!(formatter, "the listening socket could not be bound: {error}"),
            Self::Keyboard(error) => write!(formatter, "the keyboard keymap could not be built: {error}"),
            Self::Layout(error) => write!(formatter, "the output layout could not be computed: {error:?}"),
        }
    }
}

impl std::error::Error for StartupError {}

impl From<LayoutError> for StartupError {
    fn from(error: LayoutError) -> Self {
        Self::Layout(error)
    }
}

/// Why a running session stopped.
#[derive(Debug)]
pub enum LoopError {
    /// The event loop failed. This is a lost socket or a failed dispatch, not a
    /// client's protocol error.
    Stopped(CalloopError),
}

impl std::fmt::Display for LoopError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stopped(error) => write!(formatter, "the compositor loop stopped: {error}"),
        }
    }
}

impl std::error::Error for LoopError {}
