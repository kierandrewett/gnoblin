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
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::ping::{Ping, make_ping};
use smithay::reexports::calloop::{Error as CalloopError, EventLoop, Interest, LoopSignal, Mode, PostAction};
use smithay::reexports::wayland_server::Display;
use smithay::reexports::wayland_server::backend::InitError;

use crate::clock::{Clock, SystemClock};
use crate::modal::{Modal, ModalEvent};
use gnoblin_backend::{Backend, BackendEvent, Host, InputEvent};
use gnoblin_cursor::{DEFAULT_CURSOR_SIZE, DEFAULT_CURSOR_THEME, ProviderKind};
use gnoblin_render_frame::image::Image;

use crate::output_config::OutputConfigHost;
use gnoblin_capture::{SessionId, Target};
use gnoblin_config::Protocols;
use gnoblin_output::LayoutError;
use gnoblin_types::{ClientId, OutputId, Rect, WindowId};

use crate::access::ClientRole;
use crate::frame::{FrameStats, Screenshots};
use crate::observation::{
    MenuAnchor, MenuSource, Observer, PresentAnswer, SessionSnapshot, WindowChange, WindowChangeRefusal, WindowVisual,
    WindowVisualRefusal,
};
use crate::policy::WmOptions;
use crate::protocols::capture_shared::{CastRefusal, CastStream};
use crate::protocols::ext_background_effect_v1::ExtBackgroundEffectV1ObservedBlurRect;
use crate::protocols::extension::{Extension, ExtensionError, TitlebarActions};
use crate::protocols::gnoblin_session_host_v1::{GnoblinSessionHostV1HostedSession, GnoblinSessionHostV1Request};
use crate::protocols::wl_compositor::WlCompositorClientState;
use crate::state::State;

use crate::log;

/// The file a session binds when no socket path is configured.
///
/// The name is also the display name a client puts in `WAYLAND_DISPLAY`, which is why it is
/// public: the guardian publishes that variable to every later child, and a second spelling
/// here would be a session nothing can connect to.
pub const DEFAULT_SOCKET_NAME: &str = "gnoblin-wayland.sock";

/// The file name of the privileged socket, in the runtime directory.
///
/// Exported to the session's children as `GNOBLIN_PRIVILEGED_DISPLAY` so a tool the user
/// declares can name it as `WAYLAND_DISPLAY`. It is a separate socket from
/// [`DEFAULT_SOCKET_NAME`] so that the two cannot be confused in a `WAYLAND_DISPLAY`.
pub const DEFAULT_PRIVILEGED_SOCKET_NAME: &str = "gnoblin-wayland-privileged.sock";

/// How long [`ControlHandle::focus`] waits for the loop's answer.
///
/// A focus request is applied in the loop's next turn, so the wait is a scheduling
/// delay and not work; the limit exists so a stopped loop answers "no" instead of
/// hanging the caller.
pub const FOCUS_TIMEOUT: Duration = Duration::from_secs(2);

/// How long [`ControlHandle::start_cast`] waits for the loop's answer.
///
/// Starting a stream connects to the PipeWire daemon and waits for it to give the stream a node,
/// which the export bounds at five seconds, so the wait is a little longer than that and no
/// longer than a caller should hold a consent answer.
pub const CAST_TIMEOUT: Duration = Duration::from_secs(8);

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

/// Where a visual bell flashes: the whole screen, or the focused window alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VisualBellScope {
    /// Flash every output the session draws.
    #[default]
    Fullscreen,
    /// Flash the window the window manager has focused.
    Frame,
}

/// The visual bell the document declares: whether a bell flashes the screen, and where.
///
/// The default is off, so a session flashes nothing until the document asks for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VisualBellSettings {
    /// Whether a rung bell flashes at all.
    pub enabled: bool,
    /// The part of the screen a flash covers.
    pub scope: VisualBellScope,
}

/// The visual accessibility a session applies to every output.
///
/// The default is off: no magnification and no colour transform, so a frame is drawn as
/// the clients painted it. Each output takes its own view, so a magnifier centred on one
/// output never moves another.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VisualAccessibilitySettings {
    /// The magnification factor, from 1 to 32, or `None` for no magnifier. A factor of 1
    /// is the identity and is treated as no magnifier.
    pub magnifier_zoom: Option<f32>,
    /// The colour transform rows, or `None` for colours left unchanged.
    pub colour_matrix: Option<[[f32; 5]; 4]>,
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
    /// The declared workspaces, one `id:name` item each, published on the main seat for the shell to build from.
    pub workspace_names: Vec<String>,
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
    /// The tree's queue, schema and publish hook. `TreeHooks::default` refuses every class
    /// and publishes nowhere.
    pub tree: crate::tree_frame::TreeHooks,
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
    /// The document's `shortcuts`: each named binding and the command it runs.
    pub shortcuts: Option<gnoblin_config::Shortcuts>,
    /// The document's `touchpad-gestures`, in declaration order.
    pub touchpad_gestures: Vec<gnoblin_config::Gesture>,
    /// A test seam: the process every window reports in place of its client's, so a test
    /// can name a process it chose. `None` reads each window's connection.
    pub window_process_override: Option<crate::WindowProcess>,
    /// The titlebar actions `window_management` declares.
    ///
    /// The redirect in `gnoblin-compat-wayland-gtk-shell1` maps a titlebar gesture to one of
    /// these three, and the session is what it asks, so the values travel with the
    /// session rather than with the adapter: one session, one set of actions.
    pub titlebar_actions: TitlebarActions,
    /// The pointer cursor the document declares: the theme, its logical size and the
    /// providers searched in order.
    pub cursor: CursorSettings,
    /// The `monitors` table: the privacy-screen request and the global integer scale.
    pub monitors: MonitorSettings,
    /// The visual bell the document declares: whether a bell flashes the screen, and where.
    /// Off unless the document asks for it.
    pub visual_bell: VisualBellSettings,
    /// The visual accessibility the document declares: a magnifier and a colour filter,
    /// both applied to every output. Off unless the document asks for it.
    pub visual_accessibility: VisualAccessibilitySettings,
    /// The redirect adapters this session serves.
    ///
    /// Each one registers its globals before the loop runs. An empty list is a session
    /// with no redirect, which is a working session: no global is advertised and no
    /// adapter's code is linked in.
    pub extensions: Vec<Arc<dyn Extension>>,
    /// The PipeWire daemon's socket a screen cast connects to, or [`None`] for the daemon the
    /// environment names (`PIPEWIRE_REMOTE`, then `$XDG_RUNTIME_DIR/pipewire-0`). A test names
    /// its private daemon here, so it never reaches the user's own.
    pub pipewire_socket: Option<PathBuf>,
    /// What checks the lock's password, or [`None`] for PAM's `gnoblin-lock` service against the session user. A test
    /// names a scripted authenticator and the user it is asked about, so the lock is exercised without PAM.
    pub lock_check: Option<LockCheck>,
}

/// An authenticator and the user it checks the lock's password for.
#[derive(Clone)]
pub struct LockCheck {
    /// Answers each password check.
    pub authenticator: Arc<dyn gnoblin_lock_auth::Authenticator>,
    /// The user the password is checked for; with none every attempt fails closed.
    pub user: Option<String>,
}

impl std::fmt::Debug for LockCheck {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LockCheck")
            .field("user", &self.user)
            .finish_non_exhaustive()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            socket_path: None,
            privileged_socket_path: None,
            seat_name: "seat0".to_owned(),
            workspace_names: Vec::new(),
            wm: WmOptions::default(),
            check_alive_timeout: Duration::from_secs(5),
            clock: Arc::new(SystemClock),
            observer: None,
            tree: crate::tree_frame::TreeHooks::default(),
            protocols: Protocols::default(),
            input_sources: None,
            keyboard: None,
            shortcuts: None,
            touchpad_gestures: Vec::new(),
            titlebar_actions: TitlebarActions::default(),
            cursor: CursorSettings::default(),
            monitors: MonitorSettings::default(),
            extensions: Vec::new(),
            visual_bell: VisualBellSettings::default(),
            visual_accessibility: VisualAccessibilitySettings::default(),
            window_process_override: None,
            pipewire_socket: None,
            lock_check: None,
        }
    }
}

/// Moves the backend's outputs from the `previous` global scale to the `next` one and returns
/// whether anything changed. `None` is the identity scale, which is what an output has when no
/// global scale is set.
///
/// Only outputs still at the previous scale move, so a scale a client set through
/// `zwlr_output_management_v1` or the IPC setter is not overwritten by a global change. The
/// backend is the one source of output scale, so the zwlr heads and the `wl_output` globals
/// agree. A backend that refuses the change keeps its scale and says so in the log.
///
/// The session start and a live commit both run this, so a document committed after the
/// compositor started takes the same path as one read before it.
fn apply_global_scale(backend: &mut dyn Backend<State>, previous: Option<u32>, next: Option<u32>) -> bool {
    if previous == next {
        return false;
    }
    let from = gnoblin_types::Scale::integral(previous.unwrap_or(1));
    let to = gnoblin_types::Scale::integral(next.unwrap_or(1));
    let targets: Vec<_> = backend
        .outputs()
        .iter()
        .filter(|output| output.scale == from)
        .map(|output| (output.id, output.mode))
        .collect();
    for (id, mode) in targets {
        if let Err(error) = backend.configure_output(id, mode, to) {
            log::warn!(area = "monitors", %error, "global-scale {} refused; the output keeps its scale", next.unwrap_or(1));
        }
    }
    true
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
    /// The global scale the outputs were last moved to, so a change moves them from the
    /// scale they have now.
    global_scale: Option<u32>,
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
        mut config: Config,
        xwayland: Option<crate::xwayland::XwaylandSession>,
    ) -> Result<Self, StartupError> {
        let event_loop: EventLoop<'static, State> = EventLoop::try_new().map_err(StartupError::EventLoop)?;
        let display: Display<State> = Display::new().map_err(StartupError::Display)?;

        // A duplicate of the poll file descriptor, so the loop source can be
        // `'static` and the display can stay in the state.
        let poll_fd = display.as_fd().try_clone_to_owned().map_err(StartupError::Io)?;

        let workspace_names = config.workspace_names;
        let global_scale = config.monitors.global_scale;
        apply_global_scale(backend.as_mut(), None, global_scale);
        let outputs = backend.outputs().to_vec();
        let (ping, source) = make_ping().map_err(StartupError::Io)?;
        // A shell's submission wakes the loop, so a frame drains it even when nothing else
        // would draw. The queue is shared by every clone, so the hook reaches the frame's copy.
        config.tree.queue.on_submit({
            let ping = ping.clone();
            move || ping.ping()
        });
        // The lock's password check runs on a worker thread against the session user; with no
        // user in the environment every attempt fails closed.
        let (lock_authenticator, lock_user): (Arc<dyn gnoblin_lock_auth::Authenticator>, Option<String>) =
            match config.lock_check.take() {
                Some(check) => (check.authenticator, check.user),
                None => (
                    Arc::new(gnoblin_lock_auth::PamAuthenticator::new(
                        gnoblin_lock_auth::DEFAULT_SERVICE,
                    )),
                    std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).ok(),
                ),
            };
        let (lock_auth, lock_results) = crate::lock_auth::LockAuth::new(lock_authenticator, lock_user);
        let mut state = State::new(
            display,
            &outputs,
            &config.seat_name,
            config.wm,
            xwayland,
            config.observer,
            config.tree,
            &config.protocols,
            config.titlebar_actions,
            config.cursor,
            config.visual_bell,
            config.visual_accessibility,
            &config.extensions,
            lock_auth,
        )?;
        // The host's per-output controls are learnt before any client connects, so an
        // unsupported control's global is removed before it is ever advertised.
        state.start_output_controls(&mut crate::output_control::BackendControl(backend.as_mut()));
        state.loop_handle = Some(event_loop.handle());
        // The renderer's answer to "can you import a client's GPU buffer" decides whether
        // `zwp_linux_dmabuf_v1` exists at all, so it is asked before any client connects.
        let dmabuf = backend.dmabuf();
        let display_handle = state.display_handle.clone();
        state.start_dmabuf(dmabuf.as_ref(), &display_handle);
        // The same for a capture: a session offers `dma_buf` only when the backend can write into a client's GPU
        // buffer, which only the backend knows.
        state.refresh_dmabuf_capture(backend.as_mut());
        // A View's capture picture replaces a secure node the way the backend replaces a secure layer of an output.
        state.composer.set_secure_treatment(backend.secure_treatment());
        // A node asks for effects the renderer may not draw: the composer names those in its `skipped` list.
        state.composer.set_effect_kinds(backend.effect_kinds());
        // A renderer that draws a View's subtree into a texture takes the View's description instead of its pixels.
        state.composer.set_gpu_views(backend.gpu_views());
        state.liveness = crate::liveness::Liveness::new(config.check_alive_timeout);
        state.window_process_override = config.window_process_override;
        let clock = config.clock.clone();
        // A worker's answer lands here, so the lock settles on the loop that owns the state.
        event_loop
            .handle()
            .insert_source(lock_results, |event, (), state: &mut State| {
                if let smithay::reexports::calloop::channel::Event::Msg(done) = event {
                    state.finish_authentication(done);
                }
            })
            .map_err(|error| StartupError::EventLoop(error.into()))?;
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
        if let Some(shortcuts) = &config.shortcuts {
            state.apply_shortcuts(shortcuts);
        }
        state.workspace_names = workspace_names;
        state.use_pipewire_socket(config.pipewire_socket);
        state.publish_seat_policy();
        match crate::touchpad::TouchpadGestures::from_config(&config.touchpad_gestures) {
            Ok(gestures) => state.touchpad_gestures = gestures,
            Err(error) => log::error!(area = "touchpad", "touchpad gestures refused, none will run: {error}"),
        }

        backend.register(&event_loop.handle()).map_err(StartupError::Io)?;

        // A View still may be written under the directory the socket is in, which is the runtime directory unless a
        // caller chose another; with no runtime directory the temporary one is shared, and stills go nowhere.
        let still_root = match (&config.socket_path, std::env::var_os("XDG_RUNTIME_DIR")) {
            (Some(path), _) => path.parent().map(Path::to_path_buf),
            (None, Some(runtime)) => Some(PathBuf::from(runtime)),
            (None, None) => None,
        };
        state.capture.set_still_root(still_root);
        let socket_path = config.socket_path.unwrap_or_else(default_socket_path);
        let listener = bind(&socket_path)?;
        log::info!(socket = %socket_path.display(), "listening");

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
            let privileged = bind_owner_only(path)?;
            log::info!(socket = %path.display(), "listening for privileged clients");
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

        let control = Arc::new(Control {
            queue: Mutex::new(VecDeque::new()),
            ping,
            screenshots: Screenshots::default(),
            global_scale: Mutex::new(None),
            logout: std::sync::atomic::AtomicBool::new(false),
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
                            Command::AdoptView { stream } => {
                                if let Err(error) = state.adopt_view(stream) {
                                    log::warn!(%error, "a view connection could not be adopted");
                                }
                            }
                            Command::AdoptClient { stream, reply } => {
                                let tag = state.adopt_client(stream);
                                if let Err(error) = &tag {
                                    log::warn!(%error, "an application connection could not be adopted");
                                }
                                let _ = reply.send(tag.ok());
                            }
                            Command::PlaceSurface {
                                client,
                                surface,
                                at,
                                reply,
                            } => {
                                let _ = reply.send(state.place_surface(client, surface, at));
                            }
                            Command::Sync { reply } => {
                                let _ = reply.send(());
                            }
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
                            Command::DrawnNode { window, reply } => {
                                let _ = reply.send(state.drawn_node(window));
                            }
                            Command::ReducedMotion(reduced) => state.set_reduced_motion(reduced),
                            Command::VisualBell(settings) => state.apply_visual_bell_settings(settings),
                            Command::VisualAccessibility(settings) => {
                                state.apply_visual_accessibility_settings(*settings)
                            }
                            Command::WindowManagement {
                                options,
                                titlebar,
                                check_alive_timeout,
                                reply,
                            } => {
                                state.apply_window_management(options, titlebar, check_alive_timeout);
                                let _ = reply.send(());
                            }
                            Command::Xwayland { session, reply } => {
                                let _ = reply.send(state.reconcile_xwayland(session));
                            }
                            Command::KeyboardSettings(keyboard) => state.apply_keyboard_settings(&keyboard),
                            Command::CursorSettings(cursor) => state.apply_cursor_settings(&cursor),
                            Command::InputSources(sources) => state.apply_input_sources(&sources),
                            Command::Shortcuts(shortcuts) => state.apply_shortcuts(&shortcuts),
                            Command::TouchpadGestures(gestures) => state.apply_touchpad_gestures(&gestures),
                            Command::Focus { window, reply } => {
                                // The answer is sent after the publish, so a caller that
                                // reads the observation on the strength of it sees what
                                // the focus request did.
                                let _ = reply.send(state.focus_window(window));
                            }
                            Command::Present { window, reply } => {
                                // The answer follows the publish inside the state, so a caller
                                // that reads the observation on the strength of it sees the focus.
                                let _ = reply.send(state.present_window(window));
                            }
                            Command::RequestMenu { window, anchor, reply } => {
                                // The source is the extension's: the window menu is asked
                                // for by a client or script, not by the keyboard or a
                                // client's own serial. The answer follows the publish, so
                                // the caller's sync sees the request.
                                let _ = reply.send(state.request_window_menu(window, anchor, MenuSource::Extension));
                            }
                            Command::WindowControl { window, change, reply } => {
                                // The answer follows the request's effect, so a caller that
                                // reads the observation on the strength of it sees the change.
                                let _ = reply.send(state.window_control(window, change));
                            }
                            Command::WindowVisual { window, change, reply } => {
                                // The answer follows the drawing change, so the caller's next
                                // read of the window's pose sees it.
                                let _ = reply.send(state.window_visual(window, change));
                            }
                            Command::IdleHold { holder, held, reply } => {
                                let _ = reply.send(state.idle_hold(holder, held));
                            }
                            Command::WindowProcess { window, reply } => {
                                let _ = reply.send(state.window_process_for_control(window));
                            }
                            Command::ExportedWindow { handle, reply } => {
                                let _ = reply.send(state.exported_window(&handle));
                            }
                            Command::CastStart { client, target, reply } => {
                                let _ = reply.send(state.start_cast(client, target));
                            }
                            Command::CastStop { session } => state.stop_cast(session),
                            Command::CastStopAll { client } => state.stop_casts_of(client),
                            Command::Advance(milliseconds) => state.tick_clock(milliseconds),
                            Command::SelectSource(index) => {
                                if !state.select_input_source(index) {
                                    log::warn!(area = "keyboard", index, "no such input source");
                                }
                            }
                            Command::SetModal(modal) => state.set_modal(modal),
                            Command::ModalEvents { reply } => {
                                let _ = reply.send(state.take_modal_events());
                            }
                            Command::InputStats { reply } => {
                                let _ = reply.send(state.keyboard_tables.router.stats());
                            }
                            Command::Bind { name, chord } => {
                                if !state.add_binding(&name, &chord) {
                                    log::warn!(area = "input", name, chord, "the binding was refused");
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
            global_scale,
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
    /// only then starts the session.
    pub fn promise_role(&mut self, pid: u32, role: ClientRole) {
        self.state.access().promise(pid, role);
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
                mut global_scale,
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
                // A global scale a control handle asked for since the last turn moves the
                // outputs before this turn's dispatch, so the heads it publishes are the ones
                // this turn reports.
                let scale_changed = match control.take_global_scale() {
                    Some(next) => {
                        let changed = apply_global_scale(backend.as_mut(), global_scale, next);
                        global_scale = next;
                        changed
                    }
                    None => false,
                };
                state.poll_liveness();
                state.flush();
                // A locker that dies sends no request, so the lock is told to look for
                // a connection that is gone, once per turn.
                state.poll_lock();
                // A hosted session that dies sends no request either, and
                // a session request nobody claims fails at its own deadline.
                state.poll_hosted();
                // The Xwayland session is ticked here: the loop's own timeout is the only
                // clock its process needs, and a dead or hung instance is noticed within
                // one turn.
                state.poll_xwayland(&handle, Instant::now());
                state.sync_authority_lock();
                for source in state.take_pending_contexts() {
                    let inserted = handle.insert_source(source, |stream, (), state: &mut State| {
                        if let Err(error) = state.accept_sandbox(stream) {
                            log::warn!(area = "access", %error, "a sandboxed client could not be inserted");
                        }
                    });
                    if let Err(error) = inserted {
                        log::warn!(area = "access", ?error, "a sandbox listener could not be watched");
                    }
                }
                let events = backend.dispatch(state);
                let outputs_changed = scale_changed || events.iter().any(is_output_change);
                if outputs_changed {
                    // A hotplug, a removal or a mode change: the whole layout is
                    // recomputed from the connected set, once, rather than patched
                    // per event.
                    state.sync_outputs(backend.outputs());
                    state.refresh_output_controls(&crate::output_control::BackendControl(backend.as_mut()));
                }
                sync_output_manager(state, backend.as_mut(), outputs_changed);
                state.apply_output_controls(&mut crate::output_control::BackendControl(backend.as_mut()));
                state.apply_dmabuf_imports(backend.as_mut());
                // What a capture may be written into on the GPU is learnt as outputs appear, so a session made in
                // the next turn offers `dma_buf` exactly when the backend can fill one.
                state.refresh_dmabuf_capture(backend.as_mut());
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

    /// Ends the session by logout: the loop stops after the current iteration, and
    /// [`ControlHandle::logged_out`] reports the stop as a logout.
    pub fn logout(&self) {
        self.control.logout.store(true, std::sync::atomic::Ordering::SeqCst);
        self.stop.stop();
    }

    /// Whether the loop was stopped by [`ControlHandle::logout`].
    #[must_use]
    pub fn logged_out(&self) -> bool {
        self.control.logout.load(std::sync::atomic::Ordering::SeqCst)
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

    /// Asks the loop to present a window, as the IPC's `window.present` does, and returns
    /// the loop's answer for the state it found.
    ///
    /// Unlike [`Self::focus`], this does not wait out the quiet period or the lock itself.
    /// It answers at once, so the caller can retry `Typing` and `NotShown` on its own
    /// schedule. [`None`] means the loop did not answer within [`FOCUS_TIMEOUT`] because it
    /// is stopping.
    #[must_use]
    pub fn present(&self, window: WindowId) -> Option<PresentAnswer> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::Present { window, reply });
        answer.recv_timeout(FOCUS_TIMEOUT).ok()
    }

    /// Asks for a window's menu at an anchor, as the IPC's `window.request_menu` does.
    ///
    /// The loop publishes the request with the extension as its source, so the IPC
    /// emits `menu_requested` for it. `Some(true)` means the menu was published,
    /// `Some(false)` means the window is gone, and [`None`] means the loop did not answer
    /// within [`FOCUS_TIMEOUT`] because it is stopping.
    #[must_use]
    pub fn request_menu(&self, window: WindowId, anchor: MenuAnchor) -> Option<bool> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::RequestMenu { window, anchor, reply });
        answer.recv_timeout(FOCUS_TIMEOUT).ok()
    }

    /// Applies one window-control request, as the IPC's `window.close` and `window.set_*`
    /// do. The loop applies it through the same effect a taskbar produces. `Some(Ok(()))`
    /// means the change was applied, `Some(Err(reason))` means the window is gone or the
    /// window manager refused it, and [`None`] means the loop did not answer within
    /// [`FOCUS_TIMEOUT`].
    #[must_use]
    pub fn window_control(&self, window: WindowId, change: WindowChange) -> Option<Result<(), WindowChangeRefusal>> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::WindowControl { window, change, reply });
        answer.recv_timeout(FOCUS_TIMEOUT).ok()
    }

    /// Applies one shell visual request, as the IPC's `window.set_opacity`, `set_scale`,
    /// `set_offset` and `reset_visuals` do. `Some(Ok(()))` means the drawing changed,
    /// `Some(Err(reason))` means the window is gone, the session is locked or the value is
    /// out of range, and [`None`] means the loop did not answer within [`FOCUS_TIMEOUT`].
    #[must_use]
    pub fn window_visual(&self, window: WindowId, change: WindowVisual) -> Option<Result<(), WindowVisualRefusal>> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::WindowVisual { window, change, reply });
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

    /// The window an xdg-foreign handle names. `None` when the loop did not answer in time;
    /// otherwise `None` when the handle is unknown, revoked, or its surface has no window.
    /// The portal `parent_window` mapping will call this to resolve `wayland:<handle>`; no
    /// caller exists yet.
    pub fn exported_window(&self, handle: &str) -> Option<Option<WindowId>> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::ExportedWindow {
            handle: handle.to_owned(),
            reply,
        });
        answer.recv_timeout(FOCUS_TIMEOUT).ok()
    }

    /// Starts the PipeWire stream of a screen cast the user approved, for `client`.
    ///
    /// `client` is the IPC connection that holds the consent: it owns the stream, and the
    /// stream ends with it ([`ControlHandle::stop_casts_of`]). The loop refuses a locked
    /// session and a target it does not have, and answers with the node the app reads.
    /// [`None`] means the loop did not answer within [`CAST_TIMEOUT`].
    #[must_use]
    pub fn start_cast(&self, client: ClientId, target: Target) -> Option<Result<CastStream, CastRefusal>> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::CastStart { client, target, reply });
        answer.recv_timeout(CAST_TIMEOUT).ok()
    }

    /// Ends the stream of one cast session: its PipeWire node goes away on the loop's next turn.
    pub fn stop_cast(&self, session: SessionId) {
        self.control.push(Command::CastStop { session });
    }

    /// Ends every stream `client` started, as when the connection that held the consent is gone.
    pub fn stop_casts_of(&self, client: ClientId) {
        self.control.push(Command::CastStopAll { client });
    }

    /// Promises a role to a process the compositor is about to start.
    ///
    /// A view extension and a hosted session are ordinary clients of the session
    /// socket, and their registry answer is decided when the backend accepts them. The
    /// promise has to be recorded before the process connects, which is the order the
    /// composition root uses: promise, then spawn. A process that connects before the
    /// promise is recorded is served as an ordinary client.
    pub fn grant_role(&self, pid: u32, role: ClientRole) {
        self.control.push(Command::GrantRole { pid, role });
    }

    /// Adopts `stream` as a Wayland client with the `view` role: the capture globals and nothing
    /// else (`design/xr-extension.md`, G7).
    ///
    /// The guardian made the stream's other end and gave it to the extension process, so the
    /// role belongs to the connection and nothing the process does can change it. A stream the
    /// loop cannot insert is closed and logged. Returns once the command is queued; the loop
    /// accepts it before it reads the stream's first request.
    pub fn adopt_view(&self, stream: std::os::unix::net::UnixStream) {
        self.control.push(Command::AdoptView { stream });
    }

    /// Adopts `stream` as an ordinary application and returns the tag that names the client afterwards.
    ///
    /// For an embedder that made both ends of the pair and cannot tell its clients apart by process (the wlcs
    /// integration is one: every conformance client shares its pid). The tag is what [`Self::place_surface`] takes.
    /// [`None`] means the loop did not answer within [`FOCUS_TIMEOUT`] or could not insert the stream.
    #[must_use]
    pub fn adopt_client(&self, stream: std::os::unix::net::UnixStream) -> Option<u64> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::AdoptClient { stream, reply });
        answer.recv_timeout(FOCUS_TIMEOUT).ok().flatten()
    }

    /// Places the window of the surface an adopted client names by its protocol object id, keeping its size.
    ///
    /// The position is exact: it is not clamped to the work area. `true` when the window was placed, `false` when
    /// the tag or the id names nothing, the surface is no managed window, or the window is maximised or fullscreen.
    /// [`None`] means the loop did not answer within [`FOCUS_TIMEOUT`].
    #[must_use]
    pub fn place_surface(&self, client: u64, surface: u32, at: gnoblin_types::Point) -> Option<bool> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::PlaceSurface {
            client,
            surface,
            at,
            reply,
        });
        answer.recv_timeout(FOCUS_TIMEOUT).ok()
    }

    /// Waits until the loop has run every command queued before this call, such as injected input.
    ///
    /// Injection returns once the command is queued, so a client that asks for a round trip right after it can be
    /// answered before the loop has applied the input. A caller that must see the effect on the wire first calls this
    /// after injecting. `false` means the loop did not answer within [`FOCUS_TIMEOUT`] because it is stopping.
    pub fn sync(&self) -> bool {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::Sync { reply });
        answer.recv_timeout(FOCUS_TIMEOUT).is_ok()
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

    /// The matrix and projected bounds a window's node is drawn with, or `None` when it is
    /// not drawn.
    #[must_use]
    pub fn drawn_node(&self, window: WindowId) -> Option<Option<(gnoblin_render::Mat4, Rect)>> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::DrawnNode { window, reply });
        answer.recv_timeout(FOCUS_TIMEOUT).ok()
    }

    /// Applies the accepted window management options, titlebar gestures and check-alive
    /// timeout (zero stops the pings), and republishes the seat policy the stock shell reads.
    /// Like the rules, it waits for the change to be applied, so a reload is confirmed before
    /// the reply is sent.
    pub fn set_window_management(&self, options: WmOptions, titlebar: TitlebarActions, check_alive_timeout: Duration) {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::WindowManagement {
            options,
            titlebar,
            check_alive_timeout,
            reply,
        });
        let _ = answer.recv_timeout(FOCUS_TIMEOUT);
    }

    /// Replaces the X11 session from the loop: `None` stops the running server, a session
    /// starts a fresh one. Waits for the loop's answer, so the caller's reply to a commit is
    /// ordered after the server is up or stopped.
    ///
    /// # Errors
    ///
    /// The reason the loop refused it: X11 windows are open, or the loop did not answer
    /// within [`FOCUS_TIMEOUT`] because it is stopping.
    pub fn set_xwayland(&self, session: Option<crate::xwayland::XwaylandSession>) -> Result<(), String> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::Xwayland { session, reply });
        answer
            .recv_timeout(FOCUS_TIMEOUT)
            .unwrap_or_else(|_| Err("the compositor loop did not answer".to_owned()))
    }

    /// Sets whether window property changes land at once instead of animating.
    pub fn set_reduced_motion(&self, reduced: bool) {
        self.control.push(Command::ReducedMotion(reduced));
    }

    /// Sets the global output scale to `scale`, or to the identity scale when `None`, from the
    /// next loop turn. The outputs are moved there in place, so a document committed after
    /// the session started takes effect without a restart.
    pub fn set_global_scale(&self, scale: Option<u32>) {
        self.control.request_global_scale(scale);
    }

    /// Replaces the magnifier and colour filter from the next loop turn, as a committed
    /// document does. Every output is repainted, so the next frame shows the new filter.
    pub fn set_visual_accessibility(&self, settings: VisualAccessibilitySettings) {
        self.control.push(Command::VisualAccessibility(Box::new(settings)));
    }

    /// Replaces the visual bell from the next loop turn, as a committed document does. A
    /// ring after this call flashes, or does not, by the new settings.
    pub fn set_visual_bell(&self, settings: VisualBellSettings) {
        self.control.push(Command::VisualBell(settings));
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

    /// Applies the keyboard repeat and XKB options from the next loop turn. Clients that hold
    /// the seat's keyboard see the new repeat rate and keymap without reconnecting.
    pub fn set_keyboard_settings(&self, keyboard: gnoblin_config::Keyboard) {
        self.control.push(Command::KeyboardSettings(Box::new(keyboard)));
    }

    /// Applies the cursor theme, size and providers from the next loop turn, as a committed
    /// document does.
    pub fn set_cursor_settings(&self, cursor: CursorSettings) {
        self.control.push(Command::CursorSettings(Box::new(cursor)));
    }

    /// Replaces the switchable layouts from the next loop turn, as a committed document does.
    pub fn set_input_sources(&self, sources: gnoblin_config::InputSources) {
        self.control.push(Command::InputSources(Box::new(sources)));
    }

    /// Replaces the shortcuts that run a command from the next loop turn, as a committed document
    /// does. A table the router refuses (an unknown key, a chord another shortcut owns) leaves
    /// the previous shortcuts in force.
    pub fn set_shortcuts(&self, shortcuts: gnoblin_config::Shortcuts) {
        self.control.push(Command::Shortcuts(Box::new(shortcuts)));
    }

    /// Replaces the touchpad gesture bindings from the next loop turn, as a committed document
    /// does. A table that cannot be built leaves the previous bindings in force.
    pub fn set_touchpad_gestures(&self, gestures: Vec<gnoblin_config::Gesture>) {
        self.control.push(Command::TouchpadGestures(gestures));
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

    /// The router's counters: how many key and button edges it has routed, and how many it found out of
    /// order. `None` when the loop did not answer in time.
    #[must_use]
    pub fn input_stats(&self) -> Option<gnoblin_input::Stats> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.control.push(Command::InputStats { reply });
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
    /// A present request, and the channel its answer goes back on. The loop answers from
    /// the state it is in, so the caller decides whether to ask again.
    Present {
        window: WindowId,
        reply: std::sync::mpsc::SyncSender<PresentAnswer>,
    },
    /// A window-menu request from the extension path, and the channel its answer goes
    /// back on: `Some(true)` when the menu was published, `Some(false)` when the window
    /// is gone and nothing was.
    RequestMenu {
        window: WindowId,
        anchor: MenuAnchor,
        reply: std::sync::mpsc::SyncSender<bool>,
    },
    /// A window-control request from the IPC, and the channel its answer goes back on:
    /// `Ok(())` when the change was applied, `Err` with the reason when it was not.
    WindowControl {
        window: WindowId,
        change: WindowChange,
        reply: std::sync::mpsc::SyncSender<Result<(), WindowChangeRefusal>>,
    },
    /// A shell visual request from the IPC, and the channel its answer goes back on: `Ok(())`
    /// when the drawing changed, `Err` with the reason when it did not.
    WindowVisual {
        window: WindowId,
        change: WindowVisual,
        reply: std::sync::mpsc::SyncSender<Result<(), WindowVisualRefusal>>,
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
    /// The window an xdg-foreign handle names; see `ControlHandle::exported_window`.
    ExportedWindow {
        handle: String,
        reply: std::sync::mpsc::SyncSender<Option<WindowId>>,
    },
    /// Promise a role to a process the compositor is about to start.
    GrantRole { pid: u32, role: ClientRole },
    /// Adopt a connected socket as a client with the `view` role; see `ControlHandle::adopt_view`.
    AdoptView { stream: std::os::unix::net::UnixStream },
    /// Adopt a connected socket as an unprivileged client and name it; see `ControlHandle::adopt_client`.
    AdoptClient {
        stream: std::os::unix::net::UnixStream,
        reply: std::sync::mpsc::SyncSender<Option<u64>>,
    },
    /// Place the window of a surface an adopted client names; see `ControlHandle::place_surface`.
    PlaceSurface {
        client: u64,
        surface: u32,
        at: gnoblin_types::Point,
        reply: std::sync::mpsc::SyncSender<bool>,
    },
    /// A barrier: answered once every command queued before it has run; see `ControlHandle::sync`.
    Sync { reply: std::sync::mpsc::SyncSender<()> },
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
    /// The matrix and projected bounds a window's node is drawn with, read from the scene.
    DrawnNode {
        window: WindowId,
        reply: std::sync::mpsc::SyncSender<Option<(gnoblin_render::Mat4, Rect)>>,
    },
    /// Whether window property changes land at once instead of animating.
    ReducedMotion(bool),
    /// The window management options, titlebar gestures and check-alive timeout from an accepted
    /// document.
    WindowManagement {
        options: WmOptions,
        titlebar: TitlebarActions,
        check_alive_timeout: Duration,
        reply: std::sync::mpsc::SyncSender<()>,
    },
    /// The X11 session the configuration accepted, replacing the running one (`None` stops
    /// it). The loop answers once it is applied, or with the reason it was refused.
    Xwayland {
        session: Option<crate::xwayland::XwaylandSession>,
        reply: std::sync::mpsc::SyncSender<Result<(), String>>,
    },
    /// The keyboard repeat and XKB options the configuration accepted, applied in place.
    KeyboardSettings(Box<gnoblin_config::Keyboard>),
    /// The cursor theme, size and providers the configuration accepted, applied in place.
    CursorSettings(Box<CursorSettings>),
    /// The visual bell the configuration accepted, replacing the one in force.
    VisualBell(VisualBellSettings),
    /// The magnifier and colour filter the configuration accepted, replacing those in force.
    VisualAccessibility(Box<VisualAccessibilitySettings>),
    /// The switchable layouts the configuration accepted, applied in place.
    InputSources(Box<gnoblin_config::InputSources>),
    /// The shortcuts the configuration accepted, replacing the table in force.
    Shortcuts(Box<gnoblin_config::Shortcuts>),
    /// The touchpad gesture bindings the configuration accepted, replacing the table in force.
    TouchpadGestures(Vec<gnoblin_config::Gesture>),
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
    /// The router's counters, and the channel the answer goes back on.
    InputStats {
        reply: std::sync::mpsc::SyncSender<gnoblin_input::Stats>,
    },
    /// Starts the PipeWire stream of a screen cast the user approved, and the channel its
    /// answer goes back on; see [`ControlHandle::start_cast`].
    CastStart {
        client: ClientId,
        target: Target,
        reply: std::sync::mpsc::SyncSender<Result<CastStream, CastRefusal>>,
    },
    /// Ends one screen cast stream; see [`ControlHandle::stop_cast`].
    CastStop { session: SessionId },
    /// Ends every screen cast stream one connection holds; see [`ControlHandle::stop_casts_of`].
    CastStopAll { client: ClientId },
}

/// The commands a control handle queued, and the ping that wakes the loop for them.
struct Control {
    queue: Mutex<VecDeque<Command>>,
    ping: Ping,
    screenshots: Screenshots,
    /// The global scale a control handle asked for and the loop has not applied yet. The
    /// outer [`Option`] is whether there is a request; the inner one is the scale, where
    /// [`None`] follows the system setting. The backend is owned by the loop, so a scale
    /// cannot be applied from the command queue, which has no backend.
    global_scale: Mutex<Option<Option<u32>>>,
    /// Set when a logout ends the loop, so the caller can tell a logout from a stop.
    logout: std::sync::atomic::AtomicBool,
}

impl Control {
    fn push(&self, command: Command) {
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(command);
        self.ping.ping();
    }

    fn take_global_scale(&self) -> Option<Option<u32>> {
        self.global_scale.lock().unwrap_or_else(PoisonError::into_inner).take()
    }

    fn request_global_scale(&self, scale: Option<u32>) {
        *self.global_scale.lock().unwrap_or_else(PoisonError::into_inner) = Some(scale);
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
        Ok(()) => log::debug!(socket = %path.display(), "removed a stale socket"),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(StartupError::Socket(error)),
    }
    let listener = UnixListener::bind(path).map_err(StartupError::Socket)?;
    listener.set_nonblocking(true).map_err(StartupError::Socket)?;
    Ok(listener)
}

/// Binds the privileged socket and restricts it to its owner.
///
/// The privileged socket is the only door to screencopy and the other privileged globals.
/// The socket file is created under the process umask, so the mode is tightened straight
/// after the bind. The default location is `XDG_RUNTIME_DIR`, already owner-only, so this
/// is a second wall rather than the only one.
fn bind_owner_only(path: &Path) -> Result<UnixListener, StartupError> {
    let listener = bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(StartupError::Socket)?;
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
                    log::warn!(%error, "a client could not be inserted");
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                log::warn!(%error, "accept failed");
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

/// The privileged socket a session listens on, or [`None`] when there is no runtime
/// directory to put it in.
///
/// Unlike the ordinary socket this never falls back to the temporary directory: that
/// directory is shared between users, and a privileged socket there would be reachable by
/// every local account. No runtime directory means no privileged socket at all.
#[must_use]
pub fn privileged_socket_path_in(runtime_dir: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    runtime_dir.map(|directory| PathBuf::from(directory).join(DEFAULT_PRIVILEGED_SOCKET_NAME))
}

/// The privileged socket path for the current user, from `XDG_RUNTIME_DIR`.
#[must_use]
pub fn default_privileged_socket_path() -> Option<PathBuf> {
    privileged_socket_path_in(std::env::var_os("XDG_RUNTIME_DIR").as_deref())
}

#[cfg(test)]
mod privileged_path_tests {
    use super::*;

    #[test]
    fn the_privileged_socket_lives_in_the_runtime_directory() {
        let path = privileged_socket_path_in(Some(std::ffi::OsStr::new("/run/user/1000")));
        assert_eq!(
            path,
            Some(PathBuf::from("/run/user/1000/gnoblin-wayland-privileged.sock"))
        );
    }

    #[test]
    fn no_runtime_directory_means_no_privileged_socket_not_a_shared_one() {
        assert_eq!(privileged_socket_path_in(None), None);
    }
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
