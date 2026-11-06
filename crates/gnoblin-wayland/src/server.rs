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

use gnoblin_log::{debug, info, warn};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::ping::{Ping, make_ping};
use smithay::reexports::calloop::{Error as CalloopError, EventLoop, Interest, LoopSignal, Mode, PostAction};
use smithay::reexports::wayland_server::Display;
use smithay::reexports::wayland_server::backend::InitError;

use crate::clock::{Clock, SystemClock};
use crate::modal::{Modal, ModalEvent};
use gnoblin_backend::render::Image;
use gnoblin_backend::render::cursor::{DEFAULT_CURSOR_SIZE, DEFAULT_CURSOR_THEME, ProviderKind};
use gnoblin_backend::{Backend, BackendEvent, Host, InputEvent};

use crate::output_config::OutputConfigHost;
use gnoblin_config::Protocols;
use gnoblin_output::LayoutError;
use gnoblin_types::{OutputId, Rect, WindowId, WorkspaceId};
use gnoblin_wm::WmOptions;

use crate::access::ClientRole;
use crate::frame::{FrameStats, Screenshots};
use crate::observation::{Observer, SessionSnapshot};
use crate::protocols::ext_background_effect_v1::ExtBackgroundEffectV1ObservedBlurRect;
use crate::protocols::extension::{Extension, ExtensionError, TitlebarActions};
use crate::protocols::gnoblin_session_host_v1::{GnoblinSessionHostV1HostedSession, GnoblinSessionHostV1Request};
use crate::protocols::gnoblin_window_frame_v1::GnoblinWindowFrameV1Frame;
use crate::protocols::wl_compositor::WlCompositorClientState;
use crate::state::State;

/// The file a session binds when no socket path is configured.
///
/// The name is also the display name a client puts in `WAYLAND_DISPLAY`, which is why it is
/// public: the guardian publishes that variable to every later child, and a second spelling
/// here would be a session nothing can connect to.
pub const DEFAULT_SOCKET_NAME: &str = "gnoblin-wayland.sock";

/// How long [`ControlHandle::focus`] waits for the loop's answer.
///
/// A focus request is applied in the loop's next turn, so the wait is a scheduling
/// delay and not work; the limit exists so a stopped loop answers "no" instead of
/// hanging the caller.
pub const FOCUS_TIMEOUT: Duration = Duration::from_secs(2);

/// The pointer cursor a session draws: the theme name, its logical size and the
/// providers searched for it, in order.
///
/// The composition root fills it from the `cursor` section of the configuration. The
/// default is the built-in theme at the default size with the default provider order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorSettings {
    /// The theme name a provider looks up.
    pub theme: String,
    /// The logical size in pixels.
    pub size: u16,
    /// The providers, searched in order. The built-in arrow is always the last resort.
    pub providers: Vec<ProviderKind>,
}

impl Default for CursorSettings {
    fn default() -> Self {
        Self {
            theme: DEFAULT_CURSOR_THEME.to_owned(),
            size: DEFAULT_CURSOR_SIZE,
            providers: ProviderKind::DEFAULT_ORDER.to_vec(),
        }
    }
}

/// The `monitors` table the Wayland host applies: the privacy-screen request and the
/// global integer scale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MonitorSettings {
    /// The privacy-screen request, or `None` to leave the outputs as they are.
    pub privacy_screen: Option<bool>,
    /// The integer scale new outputs start at, or `None` to follow the backend.
    pub global_scale: Option<u32>,
}

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
    /// A connection accepted here is recorded as [`crate::access::ClientRole::Privileged`]
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
    /// How long a client may leave a ping unanswered before its windows are marked
    /// unresponsive. Zero disables the liveness pings.
    pub check_alive_timeout: Duration,
    /// The source of the session clock. The loop reads it once per turn and advances the
    /// session clock by the time elapsed since the previous turn, so idle and a rest focus
    /// fire without a caller advancing it. A test that must not depend on wall time passes a
    /// [`ManualClock`](crate::ManualClock) and moves it instead, or advances the session with
    /// [`Session::advance`].
    pub clock: Arc<dyn Clock>,
    /// A reader outside the loop that is told when the observable state changed.
    ///
    /// It is how the composition root feeds its object graph from the same
    /// snapshot every other reader sees, without a second description of the
    /// window state that could drift from this one.
    pub observer: Option<Arc<dyn Observer>>,
    /// Which globals the session advertises (`gnoblin.configure.protocols`).
    pub protocols: Protocols,
    /// The document's `input-sources`: the keyboard layouts the session can switch
    /// between, and whether a window remembers its own.
    ///
    /// [`None`] leaves the seat's keymap alone, which is what the configuration reference
    /// asks for a document that says nothing about layouts.
    pub input_sources: Option<gnoblin_config::InputSources>,
    /// The document's `input.keyboard`: repeat delay, repeat interval and XKB options.
    pub keyboard: Option<gnoblin_config::Keyboard>,
    /// A test seam: the process every window reports in place of its client's, so a test
    /// can name a process it chose. `None` reads each window's connection.
    pub window_process_override: Option<crate::WindowProcess>,
    /// The titlebar actions `window_management` declares.
    ///
    /// The redirect in `gnoblin-compat-gtk-shell1` maps a titlebar gesture to one of
    /// these three, and the session is what it asks, so the values travel with the
    /// session rather than with the adapter: one session, one set of actions.
    pub titlebar_actions: TitlebarActions,
    /// The pointer cursor the document declares: the theme, its logical size and the
    /// providers searched in order.
    pub cursor: CursorSettings,
    /// The `monitors` table: the privacy-screen request and the global integer scale.
    pub monitors: MonitorSettings,
    /// The redirect adapters this session serves.
    ///
    /// Each one registers its globals before the loop runs. An empty list is a session
    /// with no redirect, which is a working session: no global is advertised and no
    /// adapter's code is linked in.
    pub extensions: Vec<Arc<dyn Extension>>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            socket_path: None,
            privileged_socket_path: None,
            seat_name: "seat0".to_owned(),
            workspaces: 1,
            wm: WmOptions::default(),
            check_alive_timeout: Duration::from_secs(5),
            clock: Arc::new(SystemClock),
            observer: None,
            protocols: Protocols::default(),
            input_sources: None,
            keyboard: None,
            titlebar_actions: TitlebarActions::default(),
            cursor: CursorSettings::default(),
            monitors: MonitorSettings::default(),
            extensions: Vec::new(),
            window_process_override: None,
        }
    }
}

/// Applies `global_scale` to the backend's outputs still at the identity scale and returns
/// the outputs as the backend now reports them.
///
/// The backend is the one source of output scale, so the zwlr heads and the `wl_output`
/// globals agree. An output the backend already scales keeps its scale, so a scale a client
/// set through `zwlr_output_management_v1` or the IPC setter is not overwritten by the global
/// value. A backend that refuses the change keeps the identity scale and says so in the log.
fn apply_global_scale(backend: &mut dyn Backend<State>, global_scale: Option<u32>) -> Vec<gnoblin_backend::Output> {
    if let Some(scale) = global_scale {
        let targets: Vec<_> = backend
            .outputs()
            .iter()
            .filter(|output| output.scale.is_identity())
            .map(|output| (output.id, output.mode))
            .collect();
        for (id, mode) in targets {
            if let Err(error) = backend.configure_output(id, mode, gnoblin_types::Scale::integral(scale)) {
                warn!(%error, "[monitors] global-scale {scale} refused; the output keeps its scale");
            }
        }
    }
    backend.outputs().to_vec()
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
    clock: Arc<dyn Clock>,
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

        let window_process_override = config.window_process_override;
        let outputs = apply_global_scale(backend.as_mut(), config.monitors.global_scale);
        let mut state = State::new(
            display,
            &outputs,
            &config.seat_name,
            config.wm,
            config.workspaces,
            xwayland,
            config.observer,
            &config.protocols,
            config.titlebar_actions,
            config.cursor,
            &config.extensions,
        )?;
        // The host's per-output controls are learnt before any client connects, so an
        // unsupported control's global is removed before it is ever advertised.
        state.start_output_controls(&mut crate::output_control::BackendControl(backend.as_mut()));
        state.liveness = crate::liveness::Liveness::new(config.check_alive_timeout);
        let clock = config.clock.clone();
        let heads = crate::output_config::BackendOutputConfig::new(backend.as_mut(), state.output_positions()).heads();
        state.publish_output_heads(heads);
        // The keyboard is configured before the socket is bound, so the first client that
        // connects already sees the keymap the document asked for.
        if let Some(keyboard) = &config.keyboard {
            state.apply_keyboard_settings(keyboard);
        }
        if let Some(sources) = &config.input_sources {
            state.apply_input_sources(sources);
        }

        backend.register(&event_loop.handle()).map_err(StartupError::Io)?;

        let socket_path = config.socket_path.unwrap_or_else(default_socket_path);
        let listener = bind(&socket_path)?;
        info!(socket = %socket_path.display(), "[wayland] listening");

        event_loop
            .handle()
            .insert_source(
                Generic::new(listener, Interest::READ, Mode::Level),
                |_, listener, state: &mut State| {
                    accept_clients(listener, state, ClientRole::Unprivileged);
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
                        accept_clients(listener, state, ClientRole::Privileged);
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
                            Command::GrantRole { pid, role } => state.grant_role(pid, role),
                            Command::ProtectSession { pid, reply } => {
                                let _ = reply.send(state.protect_hosted_session(pid));
                            }
                            Command::ReleaseSession { pid, reply } => {
                                let _ = reply.send(state.release_hosted_session(pid));
                            }
                            Command::GnoblinSessionHostV1HostedSession { pid, reply } => {
                                let _ = reply.send(state.hosted_session(pid));
                            }
                            Command::HostRequests { reply } => {
                                let _ = reply.send(state.host_requests());
                            }
                            Command::BlurRegion { window, reply } => {
                                let _ = reply.send(state.window_blur_region(window));
                            }
                            Command::GnoblinWindowFrameV1Frame { window, reply } => {
                                let _ = reply.send(state.window_frame(window));
                            }
                            Command::DrawnNode { window, reply } => {
                                let _ = reply.send(state.drawn_node(window));
                            }
                            Command::LayoutFailure { workspace, reply } => {
                                let _ = reply.send(state.layout_failure(workspace));
                            }
                            Command::ReducedMotion(reduced) => state.set_reduced_motion(reduced),
                            Command::Focus { window, reply } => {
                                // The answer is sent after the publish, so a caller that
                                // reads the observation on the strength of it sees what
                                // the focus request did.
                                let _ = reply.send(state.focus_window(window));
                            }
                            Command::Lock { reply } => state.request_lock(reply),
                            Command::SetWorkspaceLayout { workspace, name, reply } => {
                                let _ = reply.send(state.set_workspace_layout(workspace, &name));
                            }
                            Command::SetOutputLayout { output, name, reply } => {
                                let _ = reply.send(state.set_output_layout(&output, &name));
                            }
                            Command::SetFloating {
                                window,
                                floating,
                                reply,
                            } => {
                                let _ = reply.send(state.set_floating(window, floating));
                            }
                            Command::IdleHold { holder, held, reply } => {
                                let _ = reply.send(state.idle_hold(holder, held));
                            }
                            Command::WindowProcess { window, reply } => {
                                let process = state.window_process(window);
                                let process = match (process, window_process_override) {
                                    (Some(_), Some(over)) => Some(over),
                                    (process, _) => process,
                                };
                                let _ = reply.send(process);
                            }
                            Command::Layout {
                                workspace,
                                command,
                                reply,
                            } => {
                                let _ = reply.send(state.layout_command(workspace, command));
                            }
                            Command::Advance(milliseconds) => state.tick_clock(milliseconds),
                            Command::SelectSource(index) => {
                                if !state.select_input_source(index) {
                                    warn!(index, "[keyboard] no such input source");
                                }
                            }
                            Command::SetModal(modal) => state.set_modal(modal),
                            Command::ModalEvents { reply } => {
                                let _ = reply.send(state.take_modal_events());
                            }
                            Command::Bind { name, chord } => {
                                if !state.add_binding(&name, &chord) {
                                    warn!(name, chord, "[input] the binding was refused");
                                }
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
            clock,
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

    /// Promises a role to a process, before it connects.
    ///
    /// The server's own seam for the composition root, which records the promise and
    /// only then starts the renderer or the session.
    pub fn promise_role(&mut self, pid: u32, role: ClientRole) {
        self.state.access().promise(pid, role);
    }

    /// Installs the layout engines, one per workspace, that arrange the windows.
    ///
    /// Called by the composition root before the loop runs. A layout that is never
    /// installed leaves the window manager tiling nothing.
    pub fn install_layouts(&mut self, control: std::rc::Rc<dyn crate::state::LayoutControl>) {
        self.state.install_layouts(control);
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
                clock,
                ..
            } = self;
            // The loop's own handle, so a sandbox listener created during a dispatch
            // can be watched from inside the turn that created it, and so the Xwayland
            // session can be ticked from the loop's own timeout.
            let handle = event_loop.handle();
            let mut last_turn: Option<std::time::Instant> = None;
            event_loop.run(Duration::from_millis(100), &mut state, move |state| {
                // The session clock follows the loop's real time, so idle fires without a
                // caller having to advance it; a caller that advances it as well is jumping
                // the same counter forwards.
                let now = clock.now();
                if let Some(previous) = last_turn {
                    let elapsed = now.duration_since(previous).as_millis();
                    let milliseconds = u64::try_from(elapsed).unwrap_or(u64::MAX);
                    state.tick_clock(milliseconds);
                    state.advance_motion(u32::try_from(elapsed).unwrap_or(u32::MAX));
                }
                last_turn = Some(now);
                state.poll_liveness();
                state.flush();
                // A locker that dies sends no request, so the lock is told to look for
                // a connection that is gone, once per turn.
                state.poll_lock();
                // A renderer or a hosted session that dies sends no request either, and
                // a session request nobody claims fails at its own deadline.
                state.poll_frames();
                state.poll_hosted();
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
                // A layout run that finished since the last turn places its windows before
                // this turn's frame is drawn.
                state.poll_layouts();
                let events = backend.dispatch(state);
                let outputs_changed = events.iter().any(is_output_change);
                if outputs_changed {
                    // A hotplug, a removal or a mode change: the whole layout is
                    // recomputed from the connected set, once, rather than patched
                    // per event.
                    state.sync_outputs(backend.outputs());
                    state.refresh_output_controls(&crate::output_control::BackendControl(backend.as_mut()));
                }
                sync_output_manager(state, backend.as_mut(), outputs_changed);
                state.apply_output_controls(&mut crate::output_control::BackendControl(backend.as_mut()));
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

    /// Locks the session and waits until the lock is `locked`, which is the state the
    /// cover has proven. `Some(true)` is that confirmation, `Some(false)` a refusal or a
    /// cover that failed, and `None` when the loop did not settle within the bound.
    pub fn lock(&self) -> Option<bool> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::Lock { reply });
        answer.recv_timeout(std::time::Duration::from_secs(3)).ok()
    }

    /// Sets a workspace's layout. `None` when the loop did not answer in time; otherwise
    /// the refusal message when `name` is not a loadable layout.
    pub fn set_workspace_layout(&self, workspace: WorkspaceId, name: &str) -> Option<Result<(), String>> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::SetWorkspaceLayout {
            workspace,
            name: name.to_owned(),
            reply,
        });
        answer.recv_timeout(FOCUS_TIMEOUT).ok()
    }

    /// Sets an output's default layout; same answers as [`ControlHandle::set_workspace_layout`].
    pub fn set_output_layout(&self, output: &str, name: &str) -> Option<Result<(), String>> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::SetOutputLayout {
            output: output.to_owned(),
            name: name.to_owned(),
            reply,
        });
        answer.recv_timeout(FOCUS_TIMEOUT).ok()
    }

    /// Sets whether a window floats. `None` when the loop did not answer in time;
    /// otherwise whether the window exists.
    pub fn set_floating(&self, window: WindowId, floating: bool) -> Option<bool> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::SetFloating {
            window,
            floating,
            reply,
        });
        answer.recv_timeout(FOCUS_TIMEOUT).ok()
    }

    /// Adds (`held` true) or removes one named idle holder. `None` when the loop did not
    /// answer in time; otherwise `Err` when adding would exceed the holder cap, and `Ok`
    /// with whether the set changed (adding a held name, or removing one that was held).
    pub fn idle_hold(&self, holder: &str, held: bool) -> Option<Result<bool, String>> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::IdleHold {
            holder: holder.to_owned(),
            held,
            reply,
        });
        answer.recv_timeout(FOCUS_TIMEOUT).ok()
    }

    /// The process behind a window's client. `None` when the loop did not answer in time;
    /// otherwise `None` when no such window exists, and the window's process otherwise.
    pub fn window_process(&self, window: WindowId) -> Option<Option<crate::WindowProcess>> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::WindowProcess { window, reply });
        answer.recv_timeout(FOCUS_TIMEOUT).ok()
    }

    /// Sends a command to a workspace's layout. `None` when the loop did not answer in
    /// time; otherwise `Err` carries the refusal (`unknown layout:` or `refused:`).
    pub fn layout_command(
        &self,
        workspace: WorkspaceId,
        command: gnoblin_wm::layout::LayoutCommand,
    ) -> Option<Result<(), String>> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::Layout {
            workspace,
            command,
            reply,
        });
        answer.recv_timeout(FOCUS_TIMEOUT).ok()
    }

    /// Promises a role to a process the compositor is about to start.
    ///
    /// A frame renderer and a hosted session are ordinary clients of the session
    /// socket, and their registry answer is decided when the backend accepts them. The
    /// promise has to be recorded before the process connects, which is the order the
    /// composition root uses: promise, then spawn. A process that connects before the
    /// promise is recorded is served as an ordinary client.
    pub fn grant_role(&self, pid: u32, role: ClientRole) {
        self.control.push(Command::GrantRole { pid, role });
    }

    /// Asks for a hosted session to be protected, as the manager's `Protect` does.
    ///
    /// [`None`] means the loop did not answer in time; `false` means no session runs
    /// with that process identity.
    #[must_use]
    pub fn protect_session(&self, pid: u32) -> Option<bool> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::ProtectSession { pid, reply });
        answer.recv_timeout(FOCUS_TIMEOUT).ok()
    }

    /// Shows a hosted session again.
    ///
    /// [`None`] means the loop did not answer in time; `false` means the session was
    /// not protected.
    #[must_use]
    pub fn release_session(&self, pid: u32) -> Option<bool> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::ReleaseSession { pid, reply });
        answer.recv_timeout(FOCUS_TIMEOUT).ok()
    }

    /// What the compositor knows about the hosted session a process runs.
    #[must_use]
    pub fn hosted_session(&self, pid: u32) -> Option<Option<GnoblinSessionHostV1HostedSession>> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control
            .push(Command::GnoblinSessionHostV1HostedSession { pid, reply });
        answer.recv_timeout(FOCUS_TIMEOUT).ok()
    }

    /// What a window's frame is doing, as the object graph and a test read it.
    ///
    /// [`None`] means the window has no frame (no renderer, or an unmapped window);
    /// the inner [`None`] means the loop did not answer in time.
    #[must_use]
    pub fn window_frame(&self, window: WindowId) -> Option<Option<GnoblinWindowFrameV1Frame>> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::GnoblinWindowFrameV1Frame { window, reply });
        answer.recv_timeout(FOCUS_TIMEOUT).ok()
    }

    /// The matrix and projected bounds a window's node is drawn with, or `None` when it is
    /// not drawn.
    #[must_use]
    pub fn drawn_node(&self, window: WindowId) -> Option<Option<(gnoblin_backend::render::Mat4, Rect)>> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::DrawnNode { window, reply });
        answer.recv_timeout(FOCUS_TIMEOUT).ok()
    }

    /// The reason a workspace's last layout run failed, or `None` when it has none on record.
    #[must_use]
    pub fn layout_failure(&self, workspace: gnoblin_types::WorkspaceId) -> Option<Option<&'static str>> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::LayoutFailure { workspace, reply });
        answer.recv_timeout(FOCUS_TIMEOUT).ok()
    }

    /// Sets whether window property changes land at once instead of animating.
    pub fn set_reduced_motion(&self, reduced: bool) {
        self.control.push(Command::ReducedMotion(reduced));
    }

    /// The committed blur region a window's surface has, as the renderer's blur pass
    /// reads it.
    ///
    /// [`None`] means the window has no surface or its surface committed no region;
    /// the inner [`None`] means the loop did not answer in time.
    #[must_use]
    pub fn blur_region(&self, window: WindowId) -> Option<Option<Vec<ExtBackgroundEffectV1ObservedBlurRect>>> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::BlurRegion { window, reply });
        answer.recv_timeout(FOCUS_TIMEOUT).ok()
    }

    /// Every hosted session request waiting for the manager.
    #[must_use]
    pub fn host_requests(&self) -> Vec<GnoblinSessionHostV1Request> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::HostRequests { reply });
        answer.recv_timeout(FOCUS_TIMEOUT).unwrap_or_default()
    }

    /// Advances the session clock, in milliseconds.
    ///
    /// Idle is a function of this clock and of input activity, so a caller that advances it
    /// drives idle deterministically. The loop advances it from real elapsed time as well,
    /// so a session with no caller idles on its own; a test that must not depend on wall
    /// time advances it instead.
    pub fn advance(&self, milliseconds: u64) {
        self.control.push(Command::Advance(milliseconds));
    }

    /// Selects a switchable keyboard layout by index, for the focused window.
    pub fn select_input_source(&self, index: usize) {
        self.control.push(Command::SelectSource(index));
    }

    /// Adds a key binding to the session's router.
    ///
    /// The chord is written the way the configuration writes it (`<Super>q`); a chord or
    /// a name the grammar refuses is reported as a warning rather than silently dropped.
    pub fn bind(&self, name: &str, chord: &str) {
        self.control.push(Command::Bind {
            name: name.to_owned(),
            chord: chord.to_owned(),
        });
    }

    /// Sets the modal overlay, or clears it with `None`.
    ///
    /// While a modal is set no key or pointer button reaches a client; each one is
    /// queued as a [`ModalEvent`] for [`ControlHandle::modal_events`] instead.
    pub fn set_modal(&self, modal: Option<Modal>) {
        self.control.push(Command::SetModal(modal));
    }

    /// The modal events queued since the last call, or `None` when the loop did not
    /// answer in time.
    #[must_use]
    pub fn modal_events(&self) -> Option<Vec<ModalEvent>> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::ModalEvents { reply });
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

/// A handle that can only inject input.
///
/// Everything it queues takes the seat's own path, so the session lock's input gate,
/// grabs, bindings and focus apply to it exactly as they do to a device. That is why it
/// is the seam an input producer outside this crate uses: the EIS server in
/// `gnoblin-input` is one, and a test's synthetic click is another. It is deliberately
/// narrower than [`ControlHandle`] — a producer has no reason to move a window, take a
/// screenshot or stop the session, so it never holds the handles that could.
#[derive(Clone)]
pub struct InputSink {
    control: Arc<Control>,
}

impl InputSink {
    /// Queues an event as if the backend had delivered it.
    pub fn inject(&self, event: InputEvent) {
        self.control.push(Command::Input(event));
    }
}

impl std::fmt::Debug for InputSink {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("InputSink").finish_non_exhaustive()
    }
}

impl ControlHandle {
    /// The input-only handle a producer outside this crate delivers through.
    #[must_use]
    pub fn input_sink(&self) -> InputSink {
        InputSink {
            control: Arc::clone(&self.control),
        }
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
    /// A session lock request. The answer is sent once the lock settles: `true` when
    /// the session is `locked`, `false` when the lock is refused or the cover fails.
    Lock { reply: std::sync::mpsc::SyncSender<bool> },
    /// A workspace's layout choice; the answer is the refusal, if any.
    SetWorkspaceLayout {
        workspace: WorkspaceId,
        name: String,
        reply: std::sync::mpsc::SyncSender<Result<(), String>>,
    },
    /// An output's default layout choice; the answer is the refusal, if any.
    SetOutputLayout {
        output: String,
        name: String,
        reply: std::sync::mpsc::SyncSender<Result<(), String>>,
    },
    /// Whether a window floats, and whether the window exists.
    SetFloating {
        window: WindowId,
        floating: bool,
        reply: std::sync::mpsc::SyncSender<bool>,
    },
    /// Adds or removes one idle holder; see `ControlHandle::idle_hold`.
    IdleHold {
        holder: String,
        held: bool,
        reply: std::sync::mpsc::SyncSender<Result<bool, String>>,
    },
    /// The process behind a window's client; see `ControlHandle::window_process`.
    WindowProcess {
        window: WindowId,
        reply: std::sync::mpsc::SyncSender<Option<crate::WindowProcess>>,
    },
    /// A command for a workspace's layout; the answer is the refusal, if any.
    Layout {
        workspace: WorkspaceId,
        command: gnoblin_wm::layout::LayoutCommand,
        reply: std::sync::mpsc::SyncSender<Result<(), String>>,
    },
    /// Promise a role to a process the compositor is about to start.
    GrantRole { pid: u32, role: ClientRole },
    /// Protect a hosted session, and answer whether the process was found.
    ProtectSession {
        pid: u32,
        reply: std::sync::mpsc::SyncSender<bool>,
    },
    /// Show a hosted session again.
    ReleaseSession {
        pid: u32,
        reply: std::sync::mpsc::SyncSender<bool>,
    },
    /// What is known about a hosted session.
    GnoblinSessionHostV1HostedSession {
        pid: u32,
        reply: std::sync::mpsc::SyncSender<Option<GnoblinSessionHostV1HostedSession>>,
    },
    /// Every session request waiting for the manager.
    HostRequests {
        reply: std::sync::mpsc::SyncSender<Vec<GnoblinSessionHostV1Request>>,
    },
    /// What a window's frame is doing.
    GnoblinWindowFrameV1Frame {
        window: WindowId,
        reply: std::sync::mpsc::SyncSender<Option<GnoblinWindowFrameV1Frame>>,
    },
    /// The matrix and projected bounds a window's node is drawn with, read from the scene.
    DrawnNode {
        window: WindowId,
        reply: std::sync::mpsc::SyncSender<Option<(gnoblin_backend::render::Mat4, Rect)>>,
    },
    /// The reason a workspace's last layout run failed.
    LayoutFailure {
        workspace: gnoblin_types::WorkspaceId,
        reply: std::sync::mpsc::SyncSender<Option<&'static str>>,
    },
    /// Whether window property changes land at once instead of animating.
    ReducedMotion(bool),
    /// A tick of the session clock, in milliseconds.
    Advance(u64),
    /// A chosen keyboard layout.
    SelectSource(usize),
    /// A key binding, written as the configuration writes it.
    Bind { name: String, chord: String },
    /// The committed blur region a window's surface has.
    BlurRegion {
        window: WindowId,
        reply: std::sync::mpsc::SyncSender<Option<Vec<ExtBackgroundEffectV1ObservedBlurRect>>>,
    },
    /// Set or clear the modal overlay, which takes input from the clients.
    SetModal(Option<Modal>),
    /// Every modal event queued since the last take, and the channel its answer goes back on.
    ModalEvents {
        reply: std::sync::mpsc::SyncSender<Vec<ModalEvent>>,
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

/// Applies any `zwlr_output_configuration_v1` the clients queued this turn, then publishes
/// the heads when the outputs changed or a configuration did.
fn sync_output_manager(state: &mut State, backend: &mut dyn Backend<State>, outputs_changed: bool) {
    let positions = state.output_positions();
    let mut host = crate::output_config::BackendOutputConfig::new(backend, positions);
    let applied = state.apply_output_configs(&mut host);
    let heads = (applied || outputs_changed).then(|| host.heads());
    drop(host);
    if let Some(heads) = heads {
        state.publish_output_heads(heads);
    }
    if applied {
        state.sync_outputs(backend.outputs());
    }
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
fn accept_clients(listener: &UnixListener, state: &mut State, role: ClientRole) {
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Err(error) =
                    state.insert_client(stream, Arc::new(WlCompositorClientState::new(&state.access())), role)
                {
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
    directory.join(DEFAULT_SOCKET_NAME)
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
    /// A redirect adapter could not register its globals.
    Extension(ExtensionError),
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
            Self::Extension(error) => write!(formatter, "a redirect adapter could not register: {error}"),
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
