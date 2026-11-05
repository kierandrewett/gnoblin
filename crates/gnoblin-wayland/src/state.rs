//! The compositor state: the Smithay protocol state, the window manager and the
//! translation between them.
//!
//! Every protocol handler in [`crate::protocols`] turns a request into an event for
//! [`gnoblin_wm::Wm::apply`] and hands the returned effects back here, where they
//! become client configures, seat focus and output enter/leave. Policy never sees a
//! Smithay type and this module never decides policy: the same split the plan draws
//! between the composition root and the pure crates.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use smithay::backend::input::{ButtonState, InputTime, KeyState};
use smithay::backend::renderer::utils::with_renderer_surface_state;
use smithay::desktop::PopupManager;
use smithay::input::keyboard::{FilterResult, KeyboardHandle, ModifiersState};
use smithay::input::pointer::{ButtonEvent, MotionEvent, PointerHandle};
use smithay::input::{Seat, SeatState};
use smithay::output::{Mode, Output, PhysicalProperties, Scale as OutputScale, Subpixel};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::Display;
use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{DisplayHandle, Resource};
use smithay::utils::{Logical, Point as SmithayPoint, SERIAL_COUNTER, Size as SmithaySize, Transform};
use smithay::wayland::compositor::{CompositorState, SurfaceAttributes, with_states};
use smithay::wayland::output::OutputManagerState;
use smithay::wayland::session_lock::SessionLockManagerState;
use smithay::wayland::shell::xdg::decoration::XdgDecorationState;
use smithay::wayland::shell::xdg::{ToplevelStateSet, ToplevelSurface, XdgShellState, XdgToplevelSurfaceData};
use smithay::wayland::shm::ShmState;
use smithay::wayland::viewporter::ViewporterState;
use tracing::{debug, warn};

use gnoblin_backend::render::{Composer, Content, OutputFrame};
use gnoblin_backend::{Host, InputEvent};
use gnoblin_lock::{Embargo, Lock, State as LockState};
use gnoblin_output::{LayoutConfig, compute};
use gnoblin_types::{OutputId, Rect, RectF, Size, WindowId, WorkspaceId};
use gnoblin_wm::{ActivationDecision, Effect, Event, WindowSpec, Wm, WmOptions};

use crate::observation::{LockObservation, ObservedWindow, SessionSnapshot};
use crate::protocols::session_lock::LockTables;
use crate::server::StartupError;

/// How many workspaces a session declares when its configuration names none.
const DEFAULT_WORKSPACES: u32 = 1;

/// The keyboard repeat settings a session starts with, in milliseconds and
/// repeats per second: the values the configuration reference defaults to.
const KEY_REPEAT_DELAY: i32 = 200;
const KEY_REPEAT_RATE: i32 = 25;

/// One toplevel the server is managing.
struct WindowEntry {
    /// The protocol object the client owns.
    surface: ToplevelSurface,
    /// Whether the client has committed a buffer, which is what "mapped" means for
    /// a toplevel.
    mapped: bool,
    /// The application id the client last sent.
    app_id: String,
    /// The title the client last sent.
    title: String,
}

/// One output the server presents on.
struct OutputEntry {
    /// The backend's identity for the output.
    backend: OutputId,
    /// The window manager's identity for it.
    policy: OutputId,
    /// The Smithay output, which owns the `wl_output` global.
    output: Output,
    /// The global the output created.
    global: GlobalId,
    /// The logical rectangle the layout assigned.
    rect: Rect,
}

/// The compositor state every protocol handler receives.
///
/// The fields are private: a handler asks for what it needs through the accessor
/// methods below, so a protocol module cannot reach into another one's table.
pub struct State {
    /// The handle every protocol handler needs to create objects and globals.
    pub(crate) display_handle: DisplayHandle,
    /// The Wayland display, taken out for the duration of a dispatch.
    ///
    /// `Display::dispatch_clients` needs `&mut Display` and `&mut State` at the
    /// same time, and the state is the type the display is generic over, so the
    /// two cannot be borrowed at once through `&mut self`. Taking the display out
    /// and putting it back is the safe way to hand both to the dispatcher; the
    /// alternative is an `unsafe` `get_mut` on an event source, which this
    /// workspace forbids.
    display: Option<Display<State>>,

    // The Smithay protocol state, one field per protocol module.
    pub(crate) compositor_state: CompositorState,
    pub(crate) xdg_shell_state: XdgShellState,
    pub(crate) shm_state: ShmState,
    pub(crate) seat_state: SeatState<State>,
    pub(crate) popups: PopupManager,
    /// Kept alive so the `zxdg_output_manager_v1` global stays registered.
    #[allow(dead_code)]
    pub(crate) output_manager_state: OutputManagerState,
    /// Kept alive so the `wp_viewporter` global stays registered. The protocol
    /// needs no handler trait: Smithay validates a viewport on commit.
    #[allow(dead_code)]
    pub(crate) viewporter_state: ViewporterState,
    /// Kept alive so the `zxdg_decoration_manager_v1` global stays registered; the
    /// handler in [`crate::protocols::decoration`] answers its requests.
    #[allow(dead_code)]
    pub(crate) decoration_state: XdgDecorationState,
    /// Kept alive so the `ext_session_lock_manager_v1` global stays registered; the
    /// handler in [`crate::protocols::session_lock`] answers its requests.
    pub(crate) session_lock_state: SessionLockManagerState,

    /// Keyboard and pointer handles for the one seat this slice creates.
    pub(crate) keyboard: KeyboardHandle<State>,
    pub(crate) pointer: PointerHandle<State>,

    /// The window management policy.
    wm: Wm,
    /// The protocol objects of the managed windows, keyed by policy identity.
    windows: BTreeMap<WindowId, WindowEntry>,
    /// The outputs, in connection order.
    outputs: Vec<OutputEntry>,
    /// The next window identity to hand out.
    next_window: u32,
    /// The last pointer position the backend reported, in global logical
    /// coordinates.
    pointer_position: SmithayPoint<f64, Logical>,

    /// The scene the frame path composes from: window nodes, stacking and damage.
    composer: Composer,
    /// The output whose frame the host described last, and its damage, so the frame
    /// hook can record what was drawn.
    last_frame: Option<(OutputId, Vec<Rect>)>,

    /// The session lock and protection-cover policy. Every request, output change
    /// and committed lock surface is an event for it, and the effects it returns are
    /// what the protocol side in [`crate::protocols::session_lock`] performs.
    pub(crate) lock: Lock,
    /// The embargo the lock last announced. It is what the capture and selection
    /// queries answer, so a consumer reads what the lock decided rather than a second
    /// opinion about it.
    pub(crate) lock_embargo: Embargo,
    /// The protocol bookkeeping the lock needs: held confirmations, lock surfaces, the
    /// cover generation and the presentation barrier.
    pub(crate) lock_tables: LockTables,

    /// The snapshot tests read.
    observation: Arc<Mutex<SessionSnapshot>>,
}

impl State {
    /// Builds the state, its globals and its outputs.
    ///
    /// # Errors
    ///
    /// [`StartupError::Keyboard`] when xkbcommon cannot build a keymap — a machine
    /// with no XKB data cannot run a seat with a keyboard.
    pub(crate) fn new(
        display: Display<State>,
        backend_outputs: &[gnoblin_backend::Output],
        seat_name: &str,
        options: WmOptions,
        workspaces: u32,
    ) -> Result<Self, StartupError> {
        let display_handle = display.handle();

        let compositor_state = CompositorState::new::<Self>(&display_handle);
        let xdg_shell_state = XdgShellState::new::<Self>(&display_handle);
        let shm_state = ShmState::new::<Self>(&display_handle, Vec::new());
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&display_handle);
        let viewporter_state = crate::protocols::viewporter::global(&display_handle);
        let decoration_state = XdgDecorationState::new::<Self>(&display_handle);
        // The lock global is advertised to every client: the standard protocol makes
        // the socket the admission boundary, and `docs/session-lock.md` states that
        // the first request owns the lock.
        let session_lock_state = SessionLockManagerState::new::<Self, _>(&display_handle, |_| true);

        let mut seat_state = SeatState::new();
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&display_handle, seat_name);
        let keyboard = seat
            .add_keyboard(Default::default(), KEY_REPEAT_DELAY, KEY_REPEAT_RATE)
            .map_err(StartupError::Keyboard)?;
        let pointer = seat.add_pointer();

        let mut state = Self {
            display_handle,
            display: Some(display),
            compositor_state,
            xdg_shell_state,
            shm_state,
            seat_state,
            popups: PopupManager::default(),
            output_manager_state,
            viewporter_state,
            decoration_state,
            session_lock_state,
            keyboard,
            pointer,
            wm: Wm::new(options),
            windows: BTreeMap::new(),
            outputs: Vec::new(),
            next_window: 1,
            pointer_position: SmithayPoint::from((0.0, 0.0)),
            composer: Composer::new(),
            last_frame: None,
            lock: Lock::new(),
            lock_embargo: Embargo::RELEASED,
            lock_tables: LockTables::default(),
            observation: Arc::new(Mutex::new(SessionSnapshot::default())),
        };

        for raw in 0..workspaces.max(DEFAULT_WORKSPACES) {
            state.wm.apply(Event::WorkspaceAdded {
                workspace: WorkspaceId::new(u64::from(raw) + 1),
            });
        }
        state.sync_outputs(backend_outputs);
        state.publish();
        Ok(state)
    }

    /// The snapshot of the window state the tests and a diagnostics reader copy.
    #[must_use]
    pub fn observation(&self) -> Arc<Mutex<SessionSnapshot>> {
        Arc::clone(&self.observation)
    }

    /// The window management policy, for a caller that wants to read it.
    #[must_use]
    pub fn wm(&self) -> &Wm {
        &self.wm
    }

    /// The outputs the server presents on, as the backend numbered them.
    #[must_use]
    pub fn output_ids(&self) -> Vec<OutputId> {
        self.outputs.iter().map(|entry| entry.backend).collect()
    }

    /// Inserts a connected client with the data its handlers expect.
    pub(crate) fn insert_client_data(
        &mut self,
        stream: std::os::unix::net::UnixStream,
        data: Arc<dyn smithay::reexports::wayland_server::backend::ClientData>,
    ) -> std::io::Result<()> {
        self.display_handle.insert_client(stream, data).map(|_| ())
    }

    /// The display, taken out for the caller that drives a dispatch.
    pub(crate) fn take_display(&mut self) -> Option<Display<State>> {
        self.display.take()
    }

    /// Puts the display back after a dispatch.
    pub(crate) fn restore_display(&mut self, display: Display<State>) {
        self.display = Some(display);
    }

    /// Reconciles the outputs with what the backend reports.
    ///
    /// The whole layout is recomputed from the connected set, so a hotplug, a
    /// removal and a mode change all end in the same place: one declarative layout
    /// from `gnoblin-output`, applied to the Smithay outputs and to the window
    /// manager. A layout the engine refuses leaves the previous one in place and is
    /// logged, because a display configuration mistake must not end the session.
    pub(crate) fn sync_outputs(&mut self, outputs: &[gnoblin_backend::Output]) {
        let connected: Vec<&gnoblin_backend::Output> = outputs
            .iter()
            .filter(|output| output.mode.width > 0 && output.mode.height > 0)
            .collect();
        if connected.is_empty() {
            self.remove_outputs_unless(&[]);
            self.report_outputs();
            return;
        }
        let descriptors: Vec<_> = connected.iter().map(|output| output.descriptor()).collect();
        let connectors: Vec<String> = connected.iter().map(|output| output.connector.clone()).collect();
        let layout = match compute(&descriptors, &LayoutConfig::row_over(&connectors)) {
            Ok(layout) => layout,
            Err(error) => {
                warn!(?error, "[output] the layout was refused; keeping the previous one");
                return;
            }
        };

        self.remove_outputs_unless(&connected.iter().map(|output| output.id).collect::<Vec<_>>());
        for output in &connected {
            let Some(monitor) = layout.monitor_of(output.id) else {
                warn!(connector = %output.connector, "[output] the layout does not place this output");
                continue;
            };
            match self.outputs.iter().position(|entry| entry.backend == output.id) {
                Some(index) => self.move_output(index, monitor.rect),
                None => self.add_output(output, monitor.rect),
            }
        }
        // The lock hears the whole layout once, after every change landed: it restarts
        // its presentation barrier from this set, so a hotplugged output is covered
        // before it shows anything.
        self.report_outputs();
    }

    /// Removes every output the connected set no longer names.
    fn remove_outputs_unless(&mut self, keep: &[OutputId]) {
        let gone: Vec<usize> = self
            .outputs
            .iter()
            .enumerate()
            .filter(|(_, entry)| !keep.contains(&entry.backend))
            .map(|(index, _)| index)
            .collect();
        for index in gone.into_iter().rev() {
            let entry = self.outputs.remove(index);
            self.display_handle.remove_global::<Self>(entry.global);
            self.composer.remove_output(entry.backend);
            debug!(connector = %entry.output.name(), "[output] disconnected");
            let effects = self.wm.apply(Event::OutputDisconnected { output: entry.policy });
            self.apply(effects);
        }
    }

    /// Creates one output's Smithay object, global and window-manager entry.
    fn add_output(&mut self, output: &gnoblin_backend::Output, rect: Rect) {
        let smithay_output = Output::new(
            output.connector.clone(),
            PhysicalProperties {
                size: SmithaySize::from((0, 0)),
                subpixel: Subpixel::Unknown,
                make: "Gnoblin".to_owned(),
                model: output.connector.clone(),
                serial_number: String::new(),
            },
        );
        let mode = smithay_mode(output);
        smithay_output.set_preferred(mode);
        smithay_output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            Some(OutputScale::Integer(1)),
            Some(SmithayPoint::from((rect.x, rect.y))),
        );
        let global = smithay_output.create_global::<Self>(&self.display_handle);

        let policy = OutputId::new(self.outputs.len() as u64);
        let policy_rect = rect;
        let effects = self.wm.apply(Event::OutputConnected {
            output: policy,
            rect: policy_rect,
        });
        self.apply(effects);
        self.composer.set_output(output.id, rect);
        debug!(connector = %output.connector, ?rect, "[output] connected");
        self.outputs.push(OutputEntry {
            backend: output.id,
            policy,
            output: smithay_output,
            global,
            rect: policy_rect,
        });
    }

    /// Applies a new layout rectangle to an output that is already connected.
    fn move_output(&mut self, index: usize, rect: Rect) {
        let Some(entry) = self.outputs.get_mut(index) else {
            return;
        };
        let policy = entry.policy;
        let policy_rect = rect;
        if entry.rect == policy_rect {
            return;
        }
        entry.rect = policy_rect;
        entry
            .output
            .change_current_state(None, None, None, Some(SmithayPoint::from((rect.x, rect.y))));
        self.composer.set_output(entry.backend, policy_rect);
        let effects = self.wm.apply(Event::OutputMoved {
            output: policy,
            rect: policy_rect,
        });
        self.apply(effects);
    }

    /// Applies the effects the window manager returned.
    ///
    /// This is the only place a policy answer becomes observable behaviour, so
    /// every arm says what a window manager command means to a client.
    pub(crate) fn apply(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Configure {
                    window,
                    geometry,
                    state,
                } => {
                    self.composer.place_window(window, geometry);
                    self.configure(window, geometry, state);
                }
                Effect::SetVisible { window, visible } => {
                    self.composer.set_window_visible(window, visible);
                    self.set_visible(window, visible);
                }
                Effect::Stack { order } => {
                    // The policy states the order topmost first; the scene keeps its
                    // paint order bottom first.
                    self.composer.set_stacking(&order);
                    debug!(log = ?order, "[stack]");
                }
                Effect::Focus { focused, .. } => self.move_focus(focused),
                Effect::ActivationDenied { window } => {
                    debug!(%window, "[focus] activation refused by the caller");
                }
                Effect::WorkspaceShown { workspace } => debug!(%workspace, "[workspace] shown"),
                Effect::RuleRejected { window, rule, reason } => {
                    warn!(%window, rule, ?reason, "[rules] rule did not apply")
                }
                Effect::Rejected { reason } => debug!(?reason, "[wm] the event changed nothing"),
            }
        }
    }

    /// Applies a configure to a toplevel and sends it if the client is ready for it.
    ///
    /// The state is written to the surface's pending state either way; a toplevel
    /// that has not had its initial configure yet carries this state into that
    /// configure, which the client's first commit triggers.
    fn configure(&mut self, window: WindowId, geometry: Rect, state: gnoblin_wm::WindowState) {
        let Some(surface) = self.windows.get(&window).map(|entry| entry.surface.clone()) else {
            // The window closed between the policy call and this action, which is
            // the race the policy's rejections describe.
            return;
        };
        surface.with_pending_state(|pending| {
            pending.bounds = None;
            pending.size = Some(SmithaySize::from((geometry.width as i32, geometry.height as i32)));
            set_flag(&mut pending.states, xdg_toplevel::State::Maximized, state.maximised);
            set_flag(&mut pending.states, xdg_toplevel::State::Fullscreen, state.fullscreen);
        });
        if surface.is_initial_configure_sent() {
            surface.send_configure();
        }
        debug!(%window, width = geometry.width, height = geometry.height, "[wm] configured");
    }

    /// Shows or hides a window's surface.
    fn set_visible(&mut self, window: WindowId, visible: bool) {
        let Some(entry) = self.windows.get(&window) else {
            return;
        };
        let surface = entry.surface.wl_surface().clone();
        for output in &self.outputs {
            if visible {
                output.output.enter(&surface);
            } else {
                output.output.leave(&surface);
            }
        }
    }

    /// Moves keyboard focus to a window, or to nothing.
    ///
    /// While the session is covered nothing moves: the lock's gate routes every key to
    /// the owner's lock surface or to nothing, and the window manager's answer is
    /// applied again when the reveal completes.
    fn move_focus(&mut self, focused: Option<WindowId>) {
        if self.lock.state() != LockState::Unlocked {
            debug!(?focused, "[focus] withheld while the session is covered");
            return;
        }
        let target = focused
            .and_then(|window| self.windows.get(&window))
            .map(|entry| entry.surface.wl_surface().clone());
        self.set_keyboard_focus(target);
    }

    /// Installs the keyboard focus target. This is the only place focus is set.
    pub(crate) fn set_keyboard_focus(&mut self, target: Option<WlSurface>) {
        let serial = SERIAL_COUNTER.next_serial();
        let keyboard = self.keyboard.clone();
        keyboard.set_focus(self, target, serial);
    }

    /// Drops the pointer's focus, so a covered client cannot receive a button.
    pub(crate) fn clear_pointer_focus(&mut self) {
        let serial = SERIAL_COUNTER.next_serial();
        let position = self.pointer_position;
        let time = InputTime::now();
        let pointer = self.pointer.clone();
        pointer.motion(
            self,
            None,
            &MotionEvent {
                location: position,
                serial,
                time,
            },
        );
    }

    /// The surface of the window the window manager has focused.
    pub(crate) fn focused_window_surface(&self) -> Option<WlSurface> {
        let window = self.wm.focused()?;
        self.windows
            .get(&window)
            .map(|entry| entry.surface.wl_surface().clone())
    }

    /// The identity of a `wl_output` global this session created.
    pub(crate) fn output_id_of(&self, output: &Output) -> Option<OutputId> {
        self.outputs
            .iter()
            .find(|entry| entry.output == *output)
            .map(|entry| entry.backend)
    }

    /// The logical rectangle an output occupies.
    pub(crate) fn output_rect(&self, output: OutputId) -> Option<Rect> {
        self.outputs
            .iter()
            .find(|entry| entry.backend == output)
            .map(|entry| entry.rect)
    }

    /// The output whose rectangle contains a global position.
    pub(crate) fn output_under(&self, position: SmithayPoint<f64, Logical>) -> Option<OutputId> {
        self.outputs
            .iter()
            .find(|entry| {
                let rect = entry.rect;
                position.x >= f64::from(rect.x)
                    && position.y >= f64::from(rect.y)
                    && position.x < rect.right() as f64
                    && position.y < rect.bottom() as f64
            })
            .map(|entry| entry.backend)
    }

    /// A client created a toplevel.
    pub(crate) fn toplevel_created(&mut self, surface: ToplevelSurface) {
        if self
            .windows
            .values()
            .any(|entry| entry.surface.wl_surface() == surface.wl_surface())
        {
            return;
        }
        let window = WindowId::new(u64::from(self.next_window));
        self.next_window = self.next_window.saturating_add(1);
        let wl_surface = surface.wl_surface().clone();
        self.windows.insert(
            window,
            WindowEntry {
                surface,
                mapped: false,
                app_id: String::new(),
                title: String::new(),
            },
        );
        // The window is placed now and takes focus when it first maps; the
        // activation event is sent from `toplevel_committed` for that reason.
        let effects = self.wm.apply(Event::WindowOpened(WindowSpec::normal(window, "")));
        // The scene learns the window before the effects are applied, so the initial
        // configure's geometry has a window to place.
        if let Some(managed) = self.wm.window(window) {
            self.composer.insert_window(window, managed.geometry);
        }
        self.apply(effects);
        debug!(%window, surface = ?wl_surface.id(), "[window] opened");
        self.publish();
    }

    /// A toplevel is gone, whether the client destroyed it or disconnected.
    pub(crate) fn toplevel_destroyed(&mut self, surface: &ToplevelSurface) {
        let Some(window) = self.window_of(surface.wl_surface()) else {
            return;
        };
        if let Some(entry) = self.windows.remove(&window) {
            let wl_surface = entry.surface.wl_surface().clone();
            for output in &self.outputs {
                output.output.leave(&wl_surface);
            }
            self.composer.remove_window(window);
            debug!(%window, surface = ?wl_surface.id(), "[window] closed");
        }
        let effects = self.wm.apply(Event::WindowClosed { window });
        self.apply(effects);
        self.publish();
    }

    /// A toplevel committed: record its title, application id and buffer.
    pub(crate) fn toplevel_committed(&mut self, surface: &WlSurface) {
        let Some(window) = self.window_of(surface) else {
            return;
        };
        let (app_id, title) = with_states(surface, |states| {
            let attributes = states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .expect("a toplevel always has its attributes")
                .lock()
                .expect("the attributes lock is never poisoned");
            (
                attributes.app_id.clone().unwrap_or_default(),
                attributes.title.clone().unwrap_or_default(),
            )
        });
        let mut effects = Vec::new();
        let mut newly_mapped = false;
        let mut initial_configure = None;
        if let Some(entry) = self.windows.get_mut(&window) {
            if entry.app_id != app_id {
                entry.app_id.clone_from(&app_id);
                effects.push(Event::WindowAppIdChanged { window, app_id });
            }
            if entry.title != title {
                entry.title.clone_from(&title);
                effects.push(Event::WindowTitleChanged { window, title });
            }
            let mapped = has_buffer(surface);
            newly_mapped = mapped && !entry.mapped;
            entry.mapped = mapped;
            if !entry.surface.is_initial_configure_sent() {
                initial_configure = Some(entry.surface.clone());
            }
        }
        if let Some(surface) = initial_configure {
            // The client's first commit is answered with the initial configure,
            // carrying whatever the window manager wrote to the pending state.
            surface.send_configure();
        }
        if newly_mapped {
            // Attribution is not part of this slice: a mapping toplevel is a
            // locally launched client and `window_management.focus_new_windows`
            // defaults to taking focus, so the caller's decision is `Allow`.
            effects.push(Event::ActivationRequested {
                window,
                decision: ActivationDecision::Allow,
            });
        }
        self.apply_events(effects);
        self.sync_content(window, surface);
        self.publish();
    }

    /// Records what a window's client last committed into the scene.
    ///
    /// The buffer and its view come from Smithay's renderer surface state rather than
    /// from `SurfaceAttributes`: `on_commit_buffer_handler` takes the buffer out of the
    /// attributes when it applies a commit, so the renderer's copy is the only one that
    /// still knows what the client drew.
    fn sync_content(&mut self, window: WindowId, surface: &WlSurface) {
        let committed = with_renderer_surface_state(surface, |state| {
            let buffer = state.buffer()?.clone();
            let view = state.view()?;
            let size = state.buffer_size()?;
            let scale = f64::from(state.buffer_scale());
            let transform = state.buffer_transform();
            let buffer_size = SmithaySize::from((f64::from(size.w), f64::from(size.h)));
            let src = view.src.to_buffer(scale, transform, &buffer_size);
            Some((
                buffer,
                Content {
                    src: RectF::new(src.loc.x, src.loc.y, src.size.w, src.size.h),
                    size: Size::new(view.dst.w.max(0).unsigned_abs(), view.dst.h.max(0).unsigned_abs()),
                    transform,
                },
            ))
        })
        .flatten();
        match committed {
            Some((buffer, content)) => {
                self.composer.set_window_content(window, Some(content));
                // `Buffer` derefs to the protocol object; the scene holds the handle and
                // Smithay keeps the object alive for as long as a frame can read it.
                self.composer.set_window_buffer(window, Some((*buffer).clone()));
            }
            None => {
                self.composer.set_window_content(window, None);
                self.composer.set_window_buffer(window, None);
            }
        }
    }

    /// Applies a batch of events to the policy and then their effects.
    fn apply_events(&mut self, events: Vec<Event>) {
        let mut effects = Vec::new();
        for event in events {
            effects.extend(self.wm.apply(event));
        }
        self.apply(effects);
    }

    /// The policy identity of the window owning a surface.
    pub(crate) fn window_of(&self, surface: &WlSurface) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|(_, entry)| entry.surface.wl_surface() == surface)
            .map(|(window, _)| *window)
    }

    /// Applies one event about the window a toplevel belongs to.
    pub(crate) fn window_event(&mut self, surface: &ToplevelSurface, event: impl FnOnce(WindowId) -> Event) {
        let Some(window) = self.window_of(surface.wl_surface()) else {
            return;
        };
        let effects = self.wm.apply(event(window));
        self.apply(effects);
        self.publish();
    }

    /// The window manager's state for a toplevel.
    pub(crate) fn window_state(&self, surface: &ToplevelSurface) -> Option<gnoblin_wm::WindowState> {
        let window = self.window_of(surface.wl_surface())?;
        Some(self.wm.window(window)?.state)
    }

    /// Dispatches every pending client request.
    ///
    /// The display is taken out for the call so the dispatcher can be handed
    /// `&mut State` at the same time: see the field's documentation.
    ///
    /// # Errors
    ///
    /// Any error the Wayland backend reports. A protocol error raised by a client
    /// is not one of them — it is reported on that client's connection and this
    /// call still succeeds, which is what keeps one bad client from ending the
    /// session.
    pub(crate) fn dispatch(&mut self) -> std::io::Result<usize> {
        let Some(mut display) = self.take_display() else {
            return Ok(0);
        };
        let result = display.dispatch_clients(self);
        self.restore_display(display);
        result
    }

    /// Writes every queued event to its client.
    pub(crate) fn flush(&mut self) {
        let Some(mut display) = self.take_display() else {
            return;
        };
        let result = display.flush_clients();
        self.restore_display(display);
        if let Err(error) = result {
            warn!(%error, "[wayland] flushing a client failed");
        }
    }

    /// The window's geometry, for popup positioning. `None` when it closed.
    pub(crate) fn geometry_of(&self, window: WindowId) -> Option<Rect> {
        Some(self.wm.window(window)?.geometry)
    }

    /// The monitor rectangle of the output the window is on.
    pub(crate) fn monitor_of(&self, window: WindowId) -> Option<Rect> {
        let output = self.wm.window(window)?.output?;
        Some(self.wm.output(output)?.rect())
    }

    /// Fires the frame callback every mapped window is waiting for.
    ///
    /// A client asks for a callback with `wl_surface.frame` and gets exactly one answer
    /// per frame the compositor shows; anything that paces itself by that answer draws
    /// one frame and then waits for ever without it. That is every well-behaved client,
    /// and it is above all a nested session, whose whole output *is* one of these
    /// surfaces: without this call a session on the nested backend would show its first
    /// frame and then stall.
    pub(crate) fn send_frame_callbacks(&self, time: u32) {
        for entry in self.windows.values() {
            // A window with no buffer has nothing on screen to have drawn, and Smithay's
            // own helper refuses those for the same reason.
            if !entry.mapped {
                continue;
            }
            let surface = entry.surface.wl_surface().clone();
            with_states(&surface, |states| {
                for callback in states
                    .cached_state
                    .get::<SurfaceAttributes>()
                    .current()
                    .frame_callbacks
                    .drain(..)
                {
                    callback.done(time);
                }
            });
        }
    }

    /// Ends the iteration the frame hook ran, dropping the damage it consumed.
    pub(crate) fn end_frame(&mut self) {
        self.composer.end_frame();
    }

    /// The damage of the frame the host described for `output`, if it described one.
    ///
    /// The frame hook asks immediately after rendering that output, which is why one
    /// remembered frame is enough.
    /// Forces every output to repaint, for a change the scene graph cannot see.
    ///
    /// Taking the cover down is exactly that: the composer was not asked to draw
    /// anything while the session was covered, so its damage for those outputs was
    /// consumed and the desktop has to be painted again from scratch.
    pub(crate) fn damage_outputs(&mut self) {
        for output in self.output_ids() {
            self.composer.damage_output(output);
        }
    }

    pub(crate) fn frame_damage(&self, output: OutputId) -> Option<&[Rect]> {
        match &self.last_frame {
            Some((described, damage)) if *described == output => Some(damage),
            _ => None,
        }
    }

    /// Moves a window through the window manager, as a drag would.
    ///
    /// It is the seam a test and the parity harness drive window movement with until
    /// the input layer produces drag rectangles of its own: the event takes the same
    /// path a drag's would, including the work-area constraint and the configure the
    /// client receives.
    pub(crate) fn move_window(&mut self, window: WindowId, geometry: Rect) {
        let effects = self.wm.apply(Event::MoveResize { window, geometry });
        self.apply(effects);
        self.publish();
    }

    /// Sets a window's scene opacity, as an animation would.
    pub(crate) fn set_window_opacity(&mut self, window: WindowId, opacity: f32) {
        self.composer.set_window_opacity(window, opacity);
    }

    /// Publishes the window state for the tests and for a diagnostics reader.
    pub(crate) fn publish(&mut self) {
        let active = self.wm.active_workspace();
        // A covered window keeps the window manager's focus so the reveal can restore
        // it, but it holds no keyboard focus: the lock gate routes keys to the lock
        // surface or to nothing, so the snapshot says nothing is focused.
        let focused = if self.lock.state() == LockState::Unlocked {
            self.wm.focused()
        } else {
            None
        };
        let mut snapshot = SessionSnapshot {
            outputs: self.outputs.iter().map(|entry| entry.backend).collect(),
            focus: focused,
            lock: LockObservation {
                state: self.lock.state(),
                capture_allowed: !self.lock_embargo.capture,
            },
            ..SessionSnapshot::default()
        };
        for window in self.wm.windows() {
            let mapped = self.windows.get(&window.id).is_some_and(|entry| entry.mapped);
            snapshot.windows.insert(
                window.id,
                ObservedWindow {
                    id: window.id,
                    title: window.title.clone(),
                    app_id: window.app_id.clone(),
                    mapped,
                    visible: window.is_visible(active),
                    focused: focused == Some(window.id),
                    geometry: window.geometry,
                },
            );
        }
        if let Ok(mut guard) = self.observation.lock() {
            *guard = snapshot;
        }
    }

    /// Asks the window manager to focus the window under the pointer.
    fn activate_under_pointer(&mut self) {
        let Some((surface, _)) = self.surface_under(self.pointer_position) else {
            return;
        };
        let Some(window) = self.window_of(&surface) else {
            return;
        };
        let effects = self.wm.apply(Event::ActivationRequested {
            window,
            decision: ActivationDecision::Allow,
        });
        self.apply(effects);
        self.publish();
    }

    /// The surface under a global position, topmost first, with the surface's own
    /// origin in global logical coordinates.
    ///
    /// While the session is covered the lock answers instead: the only surface a
    /// pointer event may reach is the owner's lock surface, and a miss reaches nothing
    /// at all.
    ///
    /// The origin is what Smithay's pointer needs, not the position inside the surface:
    /// its `MotionEvent` carries the global location and every `PointerTarget` makes it
    /// surface-relative by subtracting this. Handing it the local position instead
    /// makes every client's pointer coordinates relative to the wrong corner, which for
    /// a window at the origin is a pointer that never moves.
    pub(crate) fn surface_under(
        &self,
        position: SmithayPoint<f64, Logical>,
    ) -> Option<(WlSurface, SmithayPoint<f64, Logical>)> {
        if self.lock.state() != LockState::Unlocked {
            return self.lock_surface_under(position);
        }
        let active = self.wm.active_workspace();
        for window in self.wm.stack().iter().rev() {
            let (Some(entry), Some(managed)) = (self.windows.get(window), self.wm.window(*window)) else {
                continue;
            };
            if !managed.is_visible(active) {
                continue;
            }
            let geometry = managed.geometry;
            let inside = position.x >= f64::from(geometry.x)
                && position.y >= f64::from(geometry.y)
                && position.x < geometry.right() as f64
                && position.y < geometry.bottom() as f64;
            if inside {
                let origin = SmithayPoint::from((f64::from(geometry.x), f64::from(geometry.y)));
                return Some((entry.surface.wl_surface().clone(), origin));
            }
        }
        None
    }
}

/// The Smithay mode for a backend output.
fn smithay_mode(output: &gnoblin_backend::Output) -> Mode {
    Mode {
        size: SmithaySize::from((output.mode.width as i32, output.mode.height as i32)),
        refresh: output.mode.refresh_mhz as i32,
    }
}

/// A layout rectangle as the window manager states it.
/// Sets or clears an xdg toplevel state flag.
fn set_flag(states: &mut ToplevelStateSet, state: xdg_toplevel::State, enabled: bool) {
    if enabled {
        states.set(state);
    } else {
        states.unset(state);
    }
}

/// Whether a surface has a committed buffer, which is what makes a toplevel
/// mapped.
///
/// The buffer is read from the renderer's surface state rather than from
/// `SurfaceAttributes`: `on_commit_buffer_handler` takes the buffer out of the
/// attributes when it applies a commit, so the attributes are empty by the time this
/// runs and only the renderer's copy records that the client has content.
pub(crate) fn has_buffer(surface: &WlSurface) -> bool {
    with_renderer_surface_state(surface, |state| state.buffer().is_some()).unwrap_or(false)
}

impl Host for State {
    fn begin_frame(&mut self) {
        // The damage is taken once for every output, because the backend renders one
        // output at a time and a per-output take would clear the others' damage.
        self.composer.begin_frame();
        self.last_frame = None;
    }

    fn frame(&mut self, output: OutputId, frame: &mut OutputFrame) -> bool {
        let changed = if self.lock.state() == LockState::Unlocked {
            self.composer.compose(output, frame)
        } else {
            self.cover_frame(output, frame)
        };
        if changed {
            self.last_frame = Some((output, frame.damage().to_vec()));
        }
        changed
    }

    fn input(&mut self, event: InputEvent) {
        let time = InputTime::now();
        match event {
            InputEvent::PointerMotion { output, position } => {
                let Some(entry) = self.outputs.iter().find(|entry| entry.backend == output) else {
                    return;
                };
                let position = SmithayPoint::from((
                    f64::from(position.x + entry.rect.x),
                    f64::from(position.y + entry.rect.y),
                ));
                self.pointer_position = position;
                let focus = self.surface_under(position);
                let pointer = self.pointer.clone();
                let serial = SERIAL_COUNTER.next_serial();
                pointer.motion(
                    self,
                    focus,
                    &MotionEvent {
                        location: position,
                        serial,
                        time,
                    },
                );
            }
            InputEvent::PointerLeave => {
                let pointer = self.pointer.clone();
                let serial = SERIAL_COUNTER.next_serial();
                pointer.motion(
                    self,
                    None,
                    &MotionEvent {
                        location: self.pointer_position,
                        serial,
                        time,
                    },
                );
            }
            InputEvent::PointerButton { button, pressed } => {
                if pressed {
                    // A press on a window is the activation request the window
                    // manager expects from the caller: attribution decides whether
                    // it may take focus in a later slice, and a trusted input
                    // context is exactly this case.
                    self.activate_under_pointer();
                }
                let pointer = self.pointer.clone();
                let serial = SERIAL_COUNTER.next_serial();
                pointer.button(
                    self,
                    &ButtonEvent {
                        serial,
                        time,
                        button,
                        state: if pressed {
                            ButtonState::Pressed
                        } else {
                            ButtonState::Released
                        },
                    },
                );
            }
            InputEvent::Key { keycode, pressed } => {
                let keyboard = self.keyboard.clone();
                let serial = SERIAL_COUNTER.next_serial();
                let key_state = if pressed { KeyState::Pressed } else { KeyState::Released };
                keyboard.input::<(), _>(
                    self,
                    keycode.into(),
                    key_state,
                    serial,
                    time,
                    |_, _modifiers: &ModifiersState, _keysym| FilterResult::Forward,
                );
            }
            // A scroll needs the axis source and the pointer's focus serial, which the
            // input slice adds; until then a scroll reaches the seat and stops here
            // rather than scrolling a client by the wrong amount.
            InputEvent::PointerAxis { .. } => {}
            // A touch point needs a seat that tracks one point per slot per output,
            // which is the input slice's; nothing is invented about it here.
            InputEvent::Touch { .. } => {}
        }
    }
}

impl std::fmt::Debug for State {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("State")
            .field("windows", &self.windows.len())
            .field("outputs", &self.outputs.len())
            .finish_non_exhaustive()
    }
}
