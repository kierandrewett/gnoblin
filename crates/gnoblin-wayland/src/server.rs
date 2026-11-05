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
use std::time::Duration;

use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::ping::{Ping, make_ping};
use smithay::reexports::calloop::{Error as CalloopError, EventLoop, Interest, LoopSignal, Mode, PostAction};
use smithay::reexports::wayland_server::Display;
use smithay::reexports::wayland_server::backend::InitError;
use tracing::{debug, info, warn};

use gnoblin_backend::{Backend, BackendEvent, Host, InputEvent};
use gnoblin_output::LayoutError;
use gnoblin_wm::WmOptions;

use crate::observation::SessionSnapshot;
use crate::protocols::compositor::ClientState;
use crate::state::State;

/// How a session is configured.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Config {
    /// Where to listen for clients.
    ///
    /// [`None`] means `gnoblin-wayland.sock` inside `$XDG_RUNTIME_DIR`, which is
    /// where a client looks when it has no explicit socket. A test passes its own
    /// path so the run touches no shared directory.
    pub socket_path: Option<PathBuf>,
    /// The seat name clients see in `wl_seat.name`.
    pub seat_name: String,
    /// How many workspaces to declare. The first one is active.
    pub workspaces: u32,
    /// The window management options from the configuration document.
    pub wm: WmOptions,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            socket_path: None,
            seat_name: "seat0".to_owned(),
            workspaces: 1,
            wm: WmOptions::default(),
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
    pub fn new(mut backend: Box<dyn Backend<State>>, config: Config) -> Result<Self, StartupError> {
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
                    accept_clients(listener, state);
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|error| StartupError::EventLoop(error.into()))?;

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
        });
        event_loop
            .handle()
            .insert_source(source, {
                let control = Arc::clone(&control);
                move |(), _, state: &mut State| {
                    for event in control.drain() {
                        state.input(event);
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

    /// A handle that stops the loop and injects input.
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
    /// changes, and draws one frame per output. Rendering every iteration is what
    /// this slice does because nothing reports damage yet; a frame on the headless
    /// target presents nothing, so the cost is a counter.
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
                ..
            } = self;
            event_loop.run(Duration::from_millis(100), &mut state, |state| {
                state.flush();
                let events = backend.dispatch(state);
                if events.iter().any(is_output_change) {
                    // A hotplug, a removal or a mode change: the whole layout is
                    // recomputed from the connected set, once, rather than patched
                    // per event.
                    state.sync_outputs(backend.outputs());
                }
                let outputs: Vec<_> = backend.outputs().iter().map(|output| output.id).collect();
                for output in outputs {
                    if let Err(error) = backend.render(state, output) {
                        warn!(?error, "[frame] a frame could not be presented");
                    }
                }
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
        self.control.push(event);
    }
}

impl std::fmt::Debug for ControlHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ControlHandle").finish_non_exhaustive()
    }
}

/// The input a control handle queued, and the ping that wakes the loop for it.
struct Control {
    queue: Mutex<VecDeque<InputEvent>>,
    ping: Ping,
}

impl Control {
    fn push(&self, event: InputEvent) {
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(event);
        self.ping.ping();
    }

    fn drain(&self) -> Vec<InputEvent> {
        let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        queue.drain(..).collect()
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
        BackendEvent::OutputConnected(_) | BackendEvent::OutputDisconnected(_) | BackendEvent::OutputResized { .. }
    )
}

/// Accepts every client that connected since the last wake.
fn accept_clients(listener: &UnixListener, state: &mut State) {
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Err(error) = state.insert_client_data(stream, Arc::new(ClientState::default())) {
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
