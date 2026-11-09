//! The seat and protocol round trips: a real `wayland-client` against the session.
//!
//! Every test starts a session on a socket of its own, connects real connections, drives
//! the input path through [`ControlHandle`] (the same calls a backend's input source
//! makes) and asserts on what each connection received. The failure-mode list each test
//! covers is at the top of the module it exercises:
//!
//! - `a_surface_is_told_the_scale_of_its_output` — `protocols::wp_fractional_scale_v1`
//! - `a_locked_pointer_reports_motion_as_relative_events`,
//!   `a_pointer_lock_is_released_when_focus_moves`,
//!   `a_session_lock_releases_a_pointer_lock` — `protocols::zwp_pointer_constraints_v1`
//! - `idle_notify_fires_after_its_timeout`, `input_resumes_an_idled_notification`,
//!   `an_idle_inhibitor_suppresses_idle_and_never_leaks` — `protocols::ext_idle_notify_v1`
//! - `presentation_feedback_reports_a_present_timestamp` — `protocols::wp_presentation_time`
//! - `a_shortcuts_inhibitor_is_refused_while_locked` — `protocols::zwp_keyboard_shortcuts_inhibit_v1`
//! - `text_input_enter_and_commit_round_trip` — `protocols::zwp_text_input_v3`
//! - `a_sandboxed_client_does_not_see_the_privileged_globals` — `protocols::wp_security_context_v1`
//! - `a_bound_key_is_consumed_and_its_release_is_not_leaked`,
//!   `a_keymap_change_under_a_held_key_keeps_its_target`,
//!   `a_window_remembers_its_layout`,
//!   `a_keyboard_repeat_committed_after_start_reaches_the_client`,
//!   `a_layout_committed_after_start_reaches_the_client` — `keyboard`

use std::io::{Read, Seek};
use std::os::fd::AsFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::{Headless, HeadlessHandle};
use gnoblin_backend::{InputEvent, TouchPhase};
use gnoblin_lock::State as LockState;
use gnoblin_types::Point;
use gnoblin_wayland::{Config, ControlHandle, LoopError, Server, SessionSnapshot};
use gnoblin_wm::{NewWindowFocus, WmOptions};
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_surface, wl_touch,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::idle_notify::v1::client::{ext_idle_notification_v1, ext_idle_notifier_v1};
use wayland_protocols::ext::session_lock::v1::client::{ext_session_lock_manager_v1, ext_session_lock_v1};
use wayland_protocols::wp::fractional_scale::v1::client::{wp_fractional_scale_manager_v1, wp_fractional_scale_v1};
use wayland_protocols::wp::idle_inhibit::zv1::client::{zwp_idle_inhibit_manager_v1, zwp_idle_inhibitor_v1};
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::{
    zwp_keyboard_shortcuts_inhibit_manager_v1, zwp_keyboard_shortcuts_inhibitor_v1,
};
use wayland_protocols::wp::pointer_constraints::zv1::client::{zwp_locked_pointer_v1, zwp_pointer_constraints_v1};
use wayland_protocols::wp::presentation_time::client::{wp_presentation, wp_presentation_feedback};
use wayland_protocols::wp::relative_pointer::zv1::client::{zwp_relative_pointer_manager_v1, zwp_relative_pointer_v1};
use wayland_protocols::wp::security_context::v1::client::{wp_security_context_manager_v1, wp_security_context_v1};
use wayland_protocols::wp::text_input::zv3::client::{zwp_text_input_manager_v3, zwp_text_input_v3};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_misc::zwp_input_method_v2::client::{zwp_input_method_manager_v2, zwp_input_method_v2};

/// How long a test waits for the session to reach a state before failing.
const TIMEOUT: Duration = Duration::from_secs(10);

/// `KEY_A` and `KEY_B` from `linux/input-event-codes.h`.
const KEY_A: u32 = 30;
const KEY_B: u32 = 48;

/// The centre of the buffer `map_window` draws, which sits at the window's frame origin.
///
/// The frame can be larger than the buffer, and only the drawn part takes pointer input, so
/// aiming at the frame's centre would miss the window.
fn centre(geometry: gnoblin_types::Rect) -> Point {
    Point::new(geometry.x + 100, geometry.y + 75)
}

/// The rectangle of the buffer `map_window` draws, at the frame's origin. Only this part of
/// a window takes pointer input.
fn drawn_buffer(frame: gnoblin_types::Rect) -> gnoblin_types::Rect {
    gnoblin_types::Rect::new(frame.x, frame.y, 200, 150)
}

/// What a worker thread can reach of a running session.
#[derive(Clone)]
struct Handle {
    socket: PathBuf,
    control: ControlHandle,
    observation: Arc<Mutex<SessionSnapshot>>,
    #[allow(dead_code)]
    backend: HeadlessHandle,
}

impl Handle {
    fn socket(&self) -> &Path {
        &self.socket
    }

    fn snapshot(&self) -> SessionSnapshot {
        self.observation.lock().expect("the observation lock").clone()
    }

    /// Waits for the observation to satisfy `predicate`, or fails with the last one.
    fn wait_for(&self, predicate: impl Fn(&SessionSnapshot) -> bool) -> SessionSnapshot {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let observation = self.snapshot();
            if predicate(&observation) {
                return observation;
            }
            assert!(
                Instant::now() < deadline,
                "the session never reached the expected state; last state: {observation:#?}"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Waits until the one window is mapped and focused, and answers its geometry.
    fn focused_window(&self) -> gnoblin_types::Rect {
        let observation =
            self.wait_for(|observation| observation.only().is_some_and(|window| window.mapped && window.focused));
        observation.only().expect("the window").geometry
    }

    /// Queues a pointer motion at a global position.
    fn move_pointer(&self, at: Point) {
        let output = self.snapshot().outputs().first().copied().expect("an output");
        self.control
            .inject_input(InputEvent::PointerMotion { output, position: at });
    }

    /// Queues a pointer motion and a click.
    fn click(&self, at: Point) {
        self.move_pointer(at);
        self.control.inject_input(InputEvent::PointerButton {
            button: 0x110,
            pressed: true,
        });
        self.control.inject_input(InputEvent::PointerButton {
            button: 0x110,
            pressed: false,
        });
    }

    /// Queues a key edge.
    fn key(&self, keycode: u32, pressed: bool) {
        self.control.inject_input(InputEvent::Key { keycode, pressed });
    }

    /// Queues a pointer motion that leaves every output, which is how focus moves away
    /// from a locked surface.
    fn pointer_leave(&self) {
        self.control.inject_input(InputEvent::PointerLeave);
    }

    /// Queues a scroll at the pointer, in 1/120 of a step on each axis.
    fn scroll(&self, horizontal: i32, vertical: i32) {
        self.control
            .inject_input(InputEvent::PointerAxis { horizontal, vertical });
    }

    /// Queues a touch point's change at a global position on the first output.
    fn touch(&self, slot: u32, at: Point, phase: TouchPhase) {
        let output = self.snapshot().outputs().first().copied().expect("an output");
        self.control.inject_input(InputEvent::Touch {
            output,
            slot,
            position: at,
            phase,
        });
    }
}

/// Stops the session when the worker finishes, including when it panics.
struct StopOnDrop(ControlHandle);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// A session that has been created but is not running yet.
struct Session {
    /// Kept alive for the session's lifetime: dropping it would unlink the socket.
    #[allow(dead_code)]
    directory: tempfile::TempDir,
    server: Server,
    handle: Handle,
}

impl Session {
    /// Creates a session on a socket of its own.
    fn start() -> Self {
        Self::with_config(Config::default())
    }

    /// Creates a session with a configuration of the test's choosing.
    fn with_config(config: Config) -> Self {
        let backend = Headless::new();
        let backend_handle = backend.handle();
        let directory = tempfile::tempdir().expect("a temporary directory");
        let socket = directory.path().join("wayland-0.sock");
        let config = Config {
            socket_path: Some(socket.clone()),
            ..config
        };
        let server = Server::new(Box::new(backend), config).expect("the session starts");
        let handle = Handle {
            socket,
            control: server.control(),
            observation: server.observation(),
            backend: backend_handle,
        };
        Self {
            directory,
            server,
            handle,
        }
    }

    /// Runs the session on this thread and `body` on a worker, then joins both.
    fn run(self, body: impl FnOnce(&Handle) + Send + 'static) {
        let handle = self.handle.clone();
        let worker = thread::spawn({
            let handle = handle.clone();
            move || {
                let _stop = StopOnDrop(handle.control.clone());
                body(&handle);
            }
        });
        let outcome: Result<(), LoopError> = self.server.run();
        if let Err(payload) = worker.join() {
            std::panic::resume_unwind(payload);
        }
        outcome.expect("the loop stops cleanly");
    }
}

/// What a client records from the events it receives.
#[derive(Default, Debug)]
struct App {
    // Window and focus.
    configures: u32,
    toplevel_configures: u32,
    keyboard_enters: u32,
    keyboard_leaves: u32,
    /// The key codes the client was delivered.
    keys: Vec<u32>,
    /// Every keymap the seat sent, in arrival order.
    keymaps: Vec<String>,
    /// The rate and delay of the last `repeat_info` the seat sent a keyboard.
    repeat_info: Option<(i32, i32)>,
    /// The `wl_keyboard` this connection created, once the seat offered one.
    keyboard: Option<wl_keyboard::WlKeyboard>,
    /// The `wl_pointer` this connection created, once the seat offered one.
    pointer: Option<wl_pointer::WlPointer>,
    /// The preferred scale the server sent for a surface, in 1/120ths.
    preferred_scale: Option<u32>,
    // Pointer.
    pointer_enters: u32,
    pointer_motions: u32,
    pointer_x: f64,
    pointer_y: f64,
    pointer_buttons: u32,
    /// The pointer frames received: one ends each batch of pointer events.
    pointer_frames: u32,
    /// The `wl_pointer.axis` events received, as an axis and its pixels.
    scrolls: Vec<(wl_pointer::Axis, f64)>,
    /// The `wl_pointer.axis_value120` events received, as an axis and its units.
    scroll_steps: Vec<(wl_pointer::Axis, i32)>,
    /// The motion vectors a relative pointer received.
    relative: Vec<(f64, f64)>,
    // Touch.
    /// The `wl_touch` this connection created, once the seat offered touch.
    touch: Option<wl_touch::WlTouch>,
    /// The downs received as an id and the surface position it landed at.
    touch_downs: Vec<(i32, f64, f64)>,
    /// The motions received as an id and the surface position it moved to.
    touch_motions: Vec<(i32, f64, f64)>,
    /// The ids of the touch points lifted.
    touch_ups: Vec<i32>,
    touch_cancels: u32,
    /// The touch frames received: one ends each batch of touch events.
    touch_frames: u32,
    // Pointer constraints.
    locked: u32,
    unlocked: u32,
    // Idle.
    idled: u32,
    resumed: u32,
    // Presentation.
    presented: Vec<(u32, u32, u32, u32)>,
    discarded_feedback: u32,
    // Shortcut inhibitors.
    inhibitor_actives: u32,
    inhibitor_inactives: u32,
    // Text input.
    text_enters: u32,
    text_leaves: u32,
    text_commits: Vec<String>,
    // Input method.
    ime_activates: u32,
    ime_dones: u32,
    // Session lock.
    lock_locked: u32,
}

/// A real Wayland client for one connection.
struct Client {
    /// The client owns its connection so the socket closes when the client is dropped.
    #[allow(dead_code)]
    connection: Connection,
    queue: EventQueue<App>,
    state: App,
    qh: QueueHandle<App>,
    globals: GlobalList,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    seat: wl_seat::WlSeat,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
}

/// The objects of one toplevel.
struct Window {
    surface: wl_surface::WlSurface,
    #[allow(dead_code)]
    xdg_surface: xdg_surface::XdgSurface,
    #[allow(dead_code)]
    toplevel: xdg_toplevel::XdgToplevel,
}

impl Client {
    /// Connects and binds the globals an ordinary application uses.
    fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("the session is listening");
        let connection = Connection::from_socket(stream).expect("a client connection");
        let (globals, queue) = registry_queue_init::<App>(&connection).expect("the global list");
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor = globals
            .bind(&qh, 1..=1, ())
            .expect("the session advertises wl_compositor");
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).expect("the session advertises wl_shm");
        // Version 8 or later, so the pointer sends `axis_value120`.
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=9, ()).expect("the session advertises wl_seat");
        Self {
            connection,
            queue,
            state: App::default(),
            qh,
            globals,
            compositor,
            shm,
            seat,
            wm_base: None,
        }
    }

    /// Connects with the toplevel shell bound.
    fn app(socket: &Path) -> Self {
        let mut client = Self::connect(socket);
        client.wm_base = Some(
            client
                .globals
                .bind(&client.qh, 1..=1, ())
                .expect("the session advertises xdg_wm_base"),
        );
        client
    }

    fn roundtrip(&mut self) {
        self.queue.roundtrip(&mut self.state).expect("a round trip");
    }

    /// The pointer this connection bound when the seat offered it.
    ///
    /// A test must use this one rather than `get_pointer`: a second `wl_pointer` on the
    /// same connection receives the same events, so a count taken across both is double
    /// what the session sent.
    fn pointer(&mut self) -> wl_pointer::WlPointer {
        self.roundtrip();
        self.state.pointer.clone().expect("the seat offered a pointer")
    }

    /// Waits for the seat to offer touch, then checks that this connection bound a `wl_touch`.
    fn touch_device(&mut self) {
        self.roundtrip();
        assert!(self.state.touch.is_some(), "the seat offered no touch device");
    }

    /// Waits for `predicate` over the client's own state, with a round trip per try.
    fn wait_for(&mut self, predicate: impl Fn(&App) -> bool) -> &App {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            self.queue.roundtrip(&mut self.state).expect("a round trip");
            if predicate(&self.state) {
                return &self.state;
            }
            assert!(
                Instant::now() < deadline,
                "the client never received what the test waited for; state: {:#?}",
                self.state
            );
            thread::sleep(Duration::from_millis(2));
        }
    }

    /// Creates a toplevel and commits it once, which is what asks for a configure.
    fn create_window(&mut self, app_id: &str) -> Window {
        let wm_base = self.wm_base.as_ref().expect("an application client");
        let surface = self.compositor.create_surface(&self.qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, &self.qh, ());
        let toplevel = xdg_surface.get_toplevel(&self.qh, ());
        toplevel.set_app_id(app_id.to_owned());
        toplevel.set_title(app_id.to_owned());
        surface.commit();
        self.roundtrip();
        Window {
            surface,
            xdg_surface,
            toplevel,
        }
    }

    /// Attaches a shared-memory buffer and commits.
    fn attach_buffer(&mut self, surface: &wl_surface::WlSurface, width: i32, height: i32) {
        let stride = width * 4;
        let length = stride * height;
        let file = tempfile::tempfile().expect("a shared memory file");
        file.set_len(length as u64).expect("a buffer that big");
        let pool = self.shm.create_pool(file.as_fd(), length, &self.qh, ());
        let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, &self.qh, ());
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, width, height);
        surface.commit();
        self.roundtrip();
    }

    /// Creates a toplevel and maps it with a buffer.
    fn map_window(&mut self, app_id: &str) -> Window {
        let window = self.create_window(app_id);
        self.attach_buffer(&window.surface, 200, 150);
        window
    }
}

/// Every interface a test binds but does not read events from.
///
/// The generated `Dispatch` impls differ only in their event type, so one macro states the
/// rule once: an interface in this list is bound, answers nothing, and is forgotten.
macro_rules! ignored {
    ($($iface:ty),* $(,)?) => {$(
        impl Dispatch<$iface, ()> for App {
            fn event(
                _: &mut Self,
                _: &$iface,
                _: <$iface as Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}

ignored!(
    wl_compositor::WlCompositor,
    wl_surface::WlSurface,
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_buffer::WlBuffer,
    wl_output::WlOutput,
    xdg_toplevel::XdgToplevel,
    zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1,
    zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1,
    zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1,
    zwp_text_input_manager_v3::ZwpTextInputManagerV3,
    zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
    wp_security_context_manager_v1::WpSecurityContextManagerV1,
    wp_security_context_v1::WpSecurityContextV1,
);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for App {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: <wl_registry::WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // `registry_queue_init` keeps the global list in the object data.
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for App {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: <wl_seat::WlSeat as Proxy>::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            // A seat sends `capabilities` again whenever its device set changes, and each
            // `get_keyboard`/`get_pointer` creates a new object: a client that binds on
            // every announcement ends up with two keyboards and reads every key twice.
            if capabilities.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            }
            if capabilities.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            }
            if capabilities.contains(wl_seat::Capability::Touch) && state.touch.is_none() {
                state.touch = Some(seat.get_touch(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for App {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: <wl_keyboard::WlKeyboard as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { .. } => state.keyboard_enters += 1,
            wl_keyboard::Event::Leave { .. } => state.keyboard_leaves += 1,
            wl_keyboard::Event::Key { key, .. } => state.keys.push(key),
            wl_keyboard::Event::Keymap { fd, size, .. } => {
                // The keymap arrives as a sealed file descriptor; reading it is the only
                // way to see what layout the seat installed. The descriptor shares its
                // file position with the writer's, so it has to be rewound before reading:
                // without that every read returns nothing.
                let mut text = String::new();
                let mut file = std::fs::File::from(fd);
                let mut buffer = vec![0; usize::try_from(size).unwrap_or(0)];
                if file.seek(std::io::SeekFrom::Start(0)).is_ok() && file.read_exact(&mut buffer).is_ok() {
                    text.push_str(&String::from_utf8_lossy(&buffer));
                }
                state.keymaps.push(text);
            }
            wl_keyboard::Event::RepeatInfo { rate, delay } => state.repeat_info = Some((rate, delay)),
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for App {
    fn event(
        state: &mut Self,
        _pen: &wl_pointer::WlPointer,
        event: <wl_pointer::WlPointer as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface_x, surface_y, ..
            } => {
                state.pointer_enters += 1;
                state.pointer_x = surface_x;
                state.pointer_y = surface_y;
            }
            wl_pointer::Event::Motion {
                surface_x, surface_y, ..
            } => {
                state.pointer_motions += 1;
                state.pointer_x = surface_x;
                state.pointer_y = surface_y;
            }
            wl_pointer::Event::Button { .. } => state.pointer_buttons += 1,
            wl_pointer::Event::Frame => state.pointer_frames += 1,
            wl_pointer::Event::Axis {
                axis: WEnum::Value(axis),
                value,
                ..
            } => state.scrolls.push((axis, value)),
            wl_pointer::Event::AxisValue120 {
                axis: WEnum::Value(axis),
                value120,
            } => state.scroll_steps.push((axis, value120)),
            _ => {}
        }
    }
}

impl Dispatch<wl_touch::WlTouch, ()> for App {
    fn event(
        state: &mut Self,
        _: &wl_touch::WlTouch,
        event: <wl_touch::WlTouch as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_touch::Event::Down { id, x, y, .. } => state.touch_downs.push((id, x, y)),
            wl_touch::Event::Motion { id, x, y, .. } => state.touch_motions.push((id, x, y)),
            wl_touch::Event::Up { id, .. } => state.touch_ups.push(id),
            wl_touch::Event::Cancel => state.touch_cancels += 1,
            wl_touch::Event::Frame => state.touch_frames += 1,
            _ => {}
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for App {
    fn event(
        state: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: <xdg_wm_base::XdgWmBase as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
            state.configures = state.configures.max(1);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for App {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: <xdg_surface::XdgSurface as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            state.toplevel_configures += 1;
            xdg_surface.ack_configure(serial);
        }
    }
}

impl Dispatch<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
        _: <wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: <wp_fractional_scale_v1::WpFractionalScaleV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            state.preferred_scale = Some(scale);
        }
    }
}

impl Dispatch<zwp_relative_pointer_v1::ZwpRelativePointerV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &zwp_relative_pointer_v1::ZwpRelativePointerV1,
        event: <zwp_relative_pointer_v1::ZwpRelativePointerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_relative_pointer_v1::Event::RelativeMotion { dx, dy, .. } = event {
            state.relative.push((dx, dy));
        }
    }
}

impl Dispatch<zwp_pointer_constraints_v1::ZwpPointerConstraintsV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &zwp_pointer_constraints_v1::ZwpPointerConstraintsV1,
        _: <zwp_pointer_constraints_v1::ZwpPointerConstraintsV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_locked_pointer_v1::ZwpLockedPointerV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &zwp_locked_pointer_v1::ZwpLockedPointerV1,
        event: <zwp_locked_pointer_v1::ZwpLockedPointerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_locked_pointer_v1::Event::Locked => state.locked += 1,
            zwp_locked_pointer_v1::Event::Unlocked => state.unlocked += 1,
            _ => {}
        }
    }
}

impl Dispatch<ext_idle_notifier_v1::ExtIdleNotifierV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &ext_idle_notifier_v1::ExtIdleNotifierV1,
        _: <ext_idle_notifier_v1::ExtIdleNotifierV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ext_idle_notification_v1::ExtIdleNotificationV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &ext_idle_notification_v1::ExtIdleNotificationV1,
        event: <ext_idle_notification_v1::ExtIdleNotificationV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => state.idled += 1,
            ext_idle_notification_v1::Event::Resumed => state.resumed += 1,
            _ => {}
        }
    }
}

impl Dispatch<wp_presentation::WpPresentation, ()> for App {
    fn event(
        _: &mut Self,
        _: &wp_presentation::WpPresentation,
        _: <wp_presentation::WpPresentation as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wp_presentation_feedback::WpPresentationFeedback, ()> for App {
    fn event(
        state: &mut Self,
        _: &wp_presentation_feedback::WpPresentationFeedback,
        event: <wp_presentation_feedback::WpPresentationFeedback as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wp_presentation_feedback::Event::Presented {
                tv_sec_hi,
                tv_sec_lo,
                tv_nsec,
                refresh,
                ..
            } => state.presented.push((tv_sec_hi, tv_sec_lo, tv_nsec, refresh)),
            wp_presentation_feedback::Event::Discarded => state.discarded_feedback += 1,
            _ => {}
        }
    }
}

impl Dispatch<zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1,
        _: <zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_keyboard_shortcuts_inhibitor_v1::ZwpKeyboardShortcutsInhibitorV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &zwp_keyboard_shortcuts_inhibitor_v1::ZwpKeyboardShortcutsInhibitorV1,
        event: <zwp_keyboard_shortcuts_inhibitor_v1::ZwpKeyboardShortcutsInhibitorV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_keyboard_shortcuts_inhibitor_v1::Event::Active => state.inhibitor_actives += 1,
            zwp_keyboard_shortcuts_inhibitor_v1::Event::Inactive => state.inhibitor_inactives += 1,
            _ => {}
        }
    }
}

impl Dispatch<zwp_text_input_v3::ZwpTextInputV3, ()> for App {
    fn event(
        state: &mut Self,
        _: &zwp_text_input_v3::ZwpTextInputV3,
        event: <zwp_text_input_v3::ZwpTextInputV3 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_text_input_v3::Event::Enter { .. } => state.text_enters += 1,
            zwp_text_input_v3::Event::Leave { .. } => state.text_leaves += 1,
            zwp_text_input_v3::Event::CommitString { text: Some(text) } => state.text_commits.push(text),
            _ => {}
        }
    }
}

impl Dispatch<zwp_input_method_v2::ZwpInputMethodV2, ()> for App {
    fn event(
        state: &mut Self,
        _: &zwp_input_method_v2::ZwpInputMethodV2,
        event: <zwp_input_method_v2::ZwpInputMethodV2 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_input_method_v2::Event::Activate => state.ime_activates += 1,
            zwp_input_method_v2::Event::Done => state.ime_dones += 1,
            _ => {}
        }
    }
}

impl Dispatch<ext_session_lock_manager_v1::ExtSessionLockManagerV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &ext_session_lock_manager_v1::ExtSessionLockManagerV1,
        _: <ext_session_lock_manager_v1::ExtSessionLockManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ext_session_lock_v1::ExtSessionLockV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &ext_session_lock_v1::ExtSessionLockV1,
        event: <ext_session_lock_v1::ExtSessionLockV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_v1::Event::Locked = event {
            state.lock_locked += 1;
        }
    }
}

// App's fields the tests above read are written by the impls above; the compiler needs the
// struct to name them all.
impl App {
    /// The last keymap the seat sent, if any.
    fn last_keymap(&self) -> Option<&str> {
        self.keymaps.last().map(String::as_str)
    }
}

#[test]
fn a_surface_is_told_the_scale_of_its_output() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let manager: wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises wp_fractional_scale_manager_v1");
        let surface = client.compositor.create_surface(&client.qh, ());
        let _scale = manager.get_fractional_scale(&surface, &client.qh, ());
        surface.commit();
        let state = client.wait_for(|state| state.preferred_scale.is_some());
        // The preferred scale is in 1/120ths, and the one output draws at scale 1.
        assert_eq!(
            state.preferred_scale,
            Some(120),
            "the surface was told the scale of the output it is on"
        );
    });
}

#[test]
fn a_locked_pointer_reports_motion_as_relative_events() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::app(session.socket());
        let window = client.map_window("org.gnoblin.Locked");
        let geometry = session.focused_window();

        let constraints: zwp_pointer_constraints_v1::ZwpPointerConstraintsV1 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises zwp_pointer_constraints_v1");
        let relatives: zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises zwp_relative_pointer_manager_v1");
        let pointer = client.pointer();
        let _relative = relatives.get_relative_pointer(&pointer, &client.qh, ());

        // The pointer has to be over the surface before a lock may be activated, which is
        // the same rule the compositor applies.
        let inside = centre(geometry);
        session.move_pointer(inside);
        client.wait_for(|state| state.pointer_enters == 1);

        let _lock = constraints.lock_pointer(
            &window.surface,
            &pointer,
            None,
            zwp_pointer_constraints_v1::Lifetime::Persistent,
            &client.qh,
            (),
        );
        client.wait_for(|state| state.locked == 1);

        // Under the lock the pointer does not move; the movement arrives as a vector.
        let moved = Point::new(inside.x + 20, inside.y + 10);
        client.state.pointer_motions = 0;
        client.state.relative.clear();
        session.move_pointer(moved);
        let state = client.wait_for(|state| !state.relative.is_empty());
        assert_eq!(state.pointer_motions, 0, "a locked pointer does not move");
        let (dx, dy) = state.relative[0];
        assert_eq!((dx, dy), (20.0, 10.0), "the movement arrived as a relative vector");
    });
}

#[test]
fn a_pointer_lock_is_released_when_focus_moves() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::app(session.socket());
        let window = client.map_window("org.gnoblin.Released");
        let geometry = session.focused_window();

        let constraints: zwp_pointer_constraints_v1::ZwpPointerConstraintsV1 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises zwp_pointer_constraints_v1");
        let pointer = client.pointer();
        session.move_pointer(centre(geometry));
        client.wait_for(|state| state.pointer_enters == 1);
        // A oneshot lock is the lifetime that proves the release: a persistent one comes
        // back when the pointer re-enters, which is what the protocol asks for and would
        // make "the pointer moves again" unobservable.
        let _lock = constraints.lock_pointer(
            &window.surface,
            &pointer,
            None,
            zwp_pointer_constraints_v1::Lifetime::Oneshot,
            &client.qh,
            (),
        );
        client.wait_for(|state| state.locked == 1);

        // The pointer leaving the surface is a focus change: the constraint must not
        // survive it, or the pointer would stay frozen in a window nobody points at.
        session.pointer_leave();
        client.wait_for(|state| state.unlocked == 1);

        // And the pointer moves again afterwards. The first move into the surface is the
        // `enter` that carries the new position — the protocol sends no motion with it —
        // so a second move inside the same surface is what proves motion is delivered.
        let again = Point::new(geometry.x + 20, geometry.y + 20);
        session.move_pointer(again);
        client.wait_for(|state| state.pointer_enters == 2);
        assert_eq!(client.state.locked, 1, "a released oneshot lock does not come back");
        client.state.pointer_motions = 0;
        let moved = Point::new(again.x + 5, again.y + 5);
        session.move_pointer(moved);
        let state = client.wait_for(|state| state.pointer_motions >= 1);
        assert!(
            (state.pointer_x, state.pointer_y) == (f64::from(moved.x - geometry.x), f64::from(moved.y - geometry.y)),
            "the pointer moved by the second motion, not by the enter"
        );
    });
}

#[test]
fn a_scroll_over_a_window_reaches_the_pointer_in_pixels_and_steps() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::app(session.socket());
        let _window = client.map_window("org.gnoblin.Scrolled");
        let geometry = session.focused_window();
        session.move_pointer(centre(geometry));
        client.wait_for(|state| state.pointer_enters == 1);

        // One wheel notch is 120 units: 15 logical pixels, with the step count alongside.
        session.scroll(0, 120);
        let state = client.wait_for(|state| state.pointer_frames == 1);
        assert_eq!(state.scrolls, vec![(wl_pointer::Axis::VerticalScroll, 15.0)]);
        assert_eq!(state.scroll_steps, vec![(wl_pointer::Axis::VerticalScroll, 120)]);

        // Two notches on the horizontal axis: 30 pixels, and 240 units.
        session.scroll(-240, 0);
        let state = client.wait_for(|state| state.pointer_frames == 2);
        assert_eq!(state.scrolls[1], (wl_pointer::Axis::HorizontalScroll, -30.0));
        assert_eq!(state.scroll_steps[1], (wl_pointer::Axis::HorizontalScroll, -240));
    });
}

#[test]
fn a_scroll_that_moved_nothing_is_not_sent() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::app(session.socket());
        let _window = client.map_window("org.gnoblin.Still");
        let geometry = session.focused_window();
        let at = centre(geometry);
        session.move_pointer(at);
        client.wait_for(|state| state.pointer_enters == 1);

        session.scroll(0, 0);
        session.scroll(0, 120);
        // A motion is ordered after every event before it, so once it arrives the scrolls are in.
        session.move_pointer(Point::new(at.x + 1, at.y));
        let state = client.wait_for(|state| state.pointer_motions >= 1);
        assert_eq!(
            state.scrolls.len(),
            1,
            "only the notch reached the client: {:?}",
            state.scrolls
        );
        assert_eq!(state.scroll_steps, vec![(wl_pointer::Axis::VerticalScroll, 120)]);
    });
}

#[test]
fn a_scroll_with_the_pointer_over_no_window_reaches_nobody() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::app(session.socket());
        let _window = client.map_window("org.gnoblin.Unhovered");
        let geometry = session.focused_window();
        let at = centre(geometry);
        session.move_pointer(at);
        client.wait_for(|state| state.pointer_enters == 1);

        // Leaving every surface clears the pointer's focus, so nothing is under the scroll.
        session.pointer_leave();
        session.scroll(0, 120);
        // Coming back sends a new enter, ordered after the scroll.
        session.move_pointer(at);
        let state = client.wait_for(|state| state.pointer_enters == 2);
        assert!(state.scrolls.is_empty(), "no window was under the scroll");
        assert!(state.scroll_steps.is_empty(), "no window was under the scroll");
    });
}

#[test]
fn a_touch_on_a_window_reaches_it_at_window_local_coordinates() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::app(session.socket());
        let _window = client.map_window("org.gnoblin.Touched");
        let geometry = session.focused_window();
        let at = centre(geometry);
        client.touch_device();

        session.touch(0, at, TouchPhase::Down);
        let state = client.wait_for(|state| state.touch_frames == 1);
        assert_eq!(
            state.touch_downs,
            vec![(0, f64::from(at.x - geometry.x), f64::from(at.y - geometry.y))]
        );

        session.touch(0, at, TouchPhase::Up);
        let state = client.wait_for(|state| state.touch_frames == 2);
        assert_eq!(state.touch_ups, vec![0]);
    });
}

#[test]
fn a_touch_that_moves_off_the_window_stays_with_it() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::app(session.socket());
        let _window = client.map_window("org.gnoblin.Dragged");
        let geometry = session.focused_window();
        let at = centre(geometry);
        client.touch_device();

        session.touch(0, at, TouchPhase::Down);
        client.wait_for(|state| state.touch_frames == 1);

        // The finger is past the right edge, but a touch belongs to the surface it went down on.
        let away = Point::new(geometry.x + geometry.width as i32 + 40, at.y);
        session.touch(0, away, TouchPhase::Motion);
        let state = client.wait_for(|state| state.touch_frames == 2);
        assert_eq!(
            state.touch_motions,
            vec![(0, f64::from(away.x - geometry.x), f64::from(away.y - geometry.y))]
        );
    });
}

#[test]
fn a_touch_over_no_window_reaches_nobody() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::app(session.socket());
        let _window = client.map_window("org.gnoblin.Untouched");
        let geometry = session.focused_window();
        client.touch_device();

        // Slot 0 lands on nothing, so only slot 1, which lands on the window, reaches the client.
        session.touch(0, Point::new(-5000, -5000), TouchPhase::Down);
        session.touch(1, centre(geometry), TouchPhase::Down);
        let state = client.wait_for(|state| state.touch_frames == 1);
        let ids: Vec<i32> = state.touch_downs.iter().map(|down| down.0).collect();
        assert_eq!(ids, vec![1]);
    });
}

#[test]
fn a_touch_up_for_a_point_that_never_went_down_is_not_sent() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::app(session.socket());
        let _window = client.map_window("org.gnoblin.Lifted");
        let geometry = session.focused_window();
        let at = centre(geometry);
        client.touch_device();

        session.touch(7, at, TouchPhase::Up);
        session.touch(0, at, TouchPhase::Down);
        let state = client.wait_for(|state| state.touch_frames == 1);
        assert!(
            state.touch_ups.is_empty(),
            "no point of slot 7 was down: {:?}",
            state.touch_ups
        );
        assert_eq!(state.touch_downs.len(), 1);
    });
}

#[test]
fn a_cancelled_touch_ends_its_point_without_an_up() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::app(session.socket());
        let _window = client.map_window("org.gnoblin.Cancelled");
        let geometry = session.focused_window();
        let at = centre(geometry);
        client.touch_device();

        session.touch(0, at, TouchPhase::Down);
        client.wait_for(|state| state.touch_frames == 1);

        session.touch(0, at, TouchPhase::Cancel);
        // wl_touch.cancel is not part of a frame, so the client sees no frame after it.
        let state = client.wait_for(|state| state.touch_cancels == 1);
        assert_eq!(state.touch_cancels, 1);
        assert!(
            state.touch_ups.is_empty(),
            "a cancel is not an up: {:?}",
            state.touch_ups
        );
    });
}

#[test]
fn each_touch_point_keeps_its_own_id() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::app(session.socket());
        let _window = client.map_window("org.gnoblin.TwoFingers");
        let geometry = session.focused_window();
        let first = centre(geometry);
        let second = Point::new(geometry.x + 5, geometry.y + 5);
        client.touch_device();

        session.touch(0, first, TouchPhase::Down);
        session.touch(1, second, TouchPhase::Down);
        let state = client.wait_for(|state| state.touch_frames == 2);
        let ids: Vec<i32> = state.touch_downs.iter().map(|down| down.0).collect();
        assert_eq!(ids, vec![0, 1]);

        // Only the second finger moves, so only its id is reported.
        let moved = Point::new(second.x + 20, second.y + 20);
        session.touch(1, moved, TouchPhase::Motion);
        let state = client.wait_for(|state| state.touch_frames == 3);
        assert_eq!(
            state.touch_motions,
            vec![(1, f64::from(moved.x - geometry.x), f64::from(moved.y - geometry.y))]
        );

        session.touch(0, first, TouchPhase::Up);
        let state = client.wait_for(|state| state.touch_frames == 4);
        assert_eq!(state.touch_ups, vec![0]);
    });
}

#[test]
fn a_session_lock_releases_a_pointer_lock() {
    let session = Session::start();
    session.run(|session| {
        let mut app = Client::app(session.socket());
        let window = app.map_window("org.gnoblin.Captured");
        let geometry = session.focused_window();

        let constraints: zwp_pointer_constraints_v1::ZwpPointerConstraintsV1 = app
            .globals
            .bind(&app.qh, 1..=1, ())
            .expect("the session advertises zwp_pointer_constraints_v1");
        let pointer = app.pointer();
        session.move_pointer(centre(geometry));
        app.wait_for(|state| state.pointer_enters == 1);
        let _lock = constraints.lock_pointer(
            &window.surface,
            &pointer,
            None,
            zwp_pointer_constraints_v1::Lifetime::Persistent,
            &app.qh,
            (),
        );
        app.wait_for(|state| state.locked == 1);

        let mut locker = Client::connect(session.socket());
        let manager: ext_session_lock_manager_v1::ExtSessionLockManagerV1 = locker
            .globals
            .bind(&locker.qh, 1..=1, ())
            .expect("the session advertises ext_session_lock_manager_v1");
        let _lock = manager.lock(&locker.qh, ());
        locker.wait_for(|state| state.lock_locked == 1);
        session.wait_for(|observation| observation.lock_state() == LockState::Locked);

        // The covered client is told its pointer is free: a lock that survived the cover
        // would keep capturing the pointer for a window underneath it.
        app.wait_for(|state| state.unlocked == 1);
    });
}

#[test]
fn idle_notify_fires_after_its_timeout() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let notifier: ext_idle_notifier_v1::ExtIdleNotifierV1 = client
            .globals
            .bind(&client.qh, 1..=2, ())
            .expect("the session advertises ext_idle_notifier_v1");
        let seat = client.seat.clone();
        let _notification = notifier.get_idle_notification(2_000, &seat, &client.qh, ());
        client.roundtrip();
        assert_eq!(client.state.idled, 0, "the session is not idle yet");

        session.control.advance(2_100);
        client.wait_for(|state| state.idled == 1);
        assert_eq!(client.state.idled, 1, "one `idled` per idle period");

        session.control.advance(2_100);
        client.roundtrip();
        assert_eq!(client.state.idled, 1, "an idle session is not told again");
    });
}

#[test]
fn input_resumes_an_idled_notification() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let notifier: ext_idle_notifier_v1::ExtIdleNotifierV1 = client
            .globals
            .bind(&client.qh, 1..=2, ())
            .expect("the session advertises ext_idle_notifier_v1");
        let seat = client.seat.clone();
        let _notification = notifier.get_idle_notification(1_000, &seat, &client.qh, ());
        session.control.advance(1_100);
        client.wait_for(|state| state.idled == 1);

        session.key(KEY_A, true);
        session.key(KEY_A, false);
        client.wait_for(|state| state.resumed == 1);
        assert_eq!(client.state.resumed, 1, "input resumed the notification");
    });
}

#[test]
fn an_idle_inhibitor_suppresses_idle_and_never_leaks() {
    let session = Session::start();
    session.run(|session| {
        let mut watcher = Client::connect(session.socket());
        let notifier: ext_idle_notifier_v1::ExtIdleNotifierV1 = watcher
            .globals
            .bind(&watcher.qh, 1..=2, ())
            .expect("the session advertises ext_idle_notifier_v1");
        let seat = watcher.seat.clone();
        let _notification = notifier.get_idle_notification(1_000, &seat, &watcher.qh, ());

        // A second client inhibits idle from a surface of its own, then goes away without
        // destroying the inhibitor: a killed client sends no `destroy`, and its claim must
        // stop counting anyway.
        let mut inhibitor = Client::connect(session.socket());
        let manager: zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1 = inhibitor
            .globals
            .bind(&inhibitor.qh, 1..=1, ())
            .expect("the session advertises zwp_idle_inhibit_manager_v1");
        let surface = inhibitor.compositor.create_surface(&inhibitor.qh, ());
        let _inhibitor = manager.create_inhibitor(&surface, &inhibitor.qh, ());
        inhibitor.roundtrip();

        session.control.advance(5_000);
        watcher.roundtrip();
        assert_eq!(watcher.state.idled, 0, "an inhibitor suppresses idle");

        drop(inhibitor);
        let deadline = Instant::now() + TIMEOUT;
        while watcher.state.idled == 0 {
            assert!(
                Instant::now() < deadline,
                "the inhibitor outlived the client that held it"
            );
            session.control.advance(200);
            watcher.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }
    });
}

#[test]
fn presentation_feedback_reports_a_present_timestamp() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::app(session.socket());
        let window = client.map_window("org.gnoblin.Presented");
        let geometry = session.focused_window();

        let presentation: wp_presentation::WpPresentation = client
            .globals
            .bind(&client.qh, 1..=2, ())
            .expect("the session advertises wp_presentation");
        let _feedback = presentation.feedback(&window.surface, &client.qh, ());
        // The feedback is applied on commit, so the request needs one.
        client.attach_buffer(&window.surface, 200, 150);
        // A frame only presents when something changed; moving the window is a change.
        session.control.move_window(
            session.snapshot().only().expect("the window").id,
            gnoblin_types::Rect::new(geometry.x, geometry.y + 1, geometry.width, geometry.height),
        );

        let state = client.wait_for(|state| !state.presented.is_empty());
        let (seconds_hi, seconds_lo, nanoseconds, refresh) = state.presented[0];
        let seconds = (u64::from(seconds_hi) << 32) | u64::from(seconds_lo);
        assert!(
            seconds > 0 || nanoseconds > 0,
            "the feedback carries a real present time"
        );
        assert!(refresh > 0, "the feedback carries the output's refresh period");
        assert_eq!(state.discarded_feedback, 0, "the frame was presented, not discarded");
    });
}

#[test]
fn a_shortcuts_inhibitor_is_refused_while_locked() {
    let session = Session::start();
    session.run(|session| {
        let mut app = Client::app(session.socket());
        let window = app.map_window("org.gnoblin.Inhibited");
        session.focused_window();

        let manager: zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1 = app
            .globals
            .bind(&app.qh, 1..=1, ())
            .expect("the session advertises zwp_keyboard_shortcuts_inhibit_manager_v1");
        let seat = app.seat.clone();
        let _inhibitor = manager.inhibit_shortcuts(&window.surface, &seat, &app.qh, ());
        app.wait_for(|state| state.inhibitor_actives == 1);

        // Covering the session revokes the inhibitor: the lock screen owns the keys.
        let mut locker = Client::connect(session.socket());
        let lock_manager: ext_session_lock_manager_v1::ExtSessionLockManagerV1 = locker
            .globals
            .bind(&locker.qh, 1..=1, ())
            .expect("the session advertises ext_session_lock_manager_v1");
        let _lock = lock_manager.lock(&locker.qh, ());
        locker.wait_for(|state| state.lock_locked == 1);
        app.wait_for(|state| state.inhibitor_inactives == 1);

        // And a request made while covered is never activated.
        let surface = app.compositor.create_surface(&app.qh, ());
        let _second = manager.inhibit_shortcuts(&surface, &seat, &app.qh, ());
        app.roundtrip();
        app.roundtrip();
        assert_eq!(
            app.state.inhibitor_actives, 1,
            "an inhibitor requested under the cover is refused"
        );
    });
}

#[test]
fn a_bound_key_is_consumed_and_its_release_is_not_leaked() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::app(session.socket());
        let _window = client.map_window("org.gnoblin.Typing");
        session.focused_window();
        client.wait_for(|state| state.keyboard_enters == 1);

        session.control.bind("test-key", "a");
        // The binding arrives with the control queue, so it is installed before the keys
        // are routed; one turn of the loop is enough.
        thread::sleep(Duration::from_millis(50));

        session.key(KEY_A, true);
        session.key(KEY_A, false);
        // A key nothing binds still reaches the client, so the test can tell a consumed
        // edge from a broken delivery path.
        session.key(KEY_B, true);
        session.key(KEY_B, false);

        let state = client.wait_for(|state| !state.keys.is_empty());
        assert_eq!(
            state.keys,
            vec![KEY_B, KEY_B],
            "the bound key was consumed with its release, and an unbound one was delivered \
             as a press and a release of the evdev code"
        );
    });
}

#[test]
fn a_key_reaches_the_client_with_its_evdev_code_from_either_source() {
    // A backend names a key by its evdev code and the seat's keymap is indexed by the X11
    // keycode, which is that number plus the eight the protocol reserves. Smithay removes
    // the eight again when it sends `wl_keyboard.key`, so the offset must be applied
    // exactly once, in the seat's key path, and nowhere else: a key shifted twice would
    // arrive as `KEY_A + 8` and one not shifted at all as `KEY_A - 8`, and a client would
    // read a different letter than the user pressed.
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::app(session.socket());
        let _window = client.map_window("org.gnoblin.Keys");
        session.focused_window();
        client.wait_for(|state| state.keyboard_enters == 1);

        // The backend's path: the event a physical device produces.
        session.key(KEY_A, true);
        session.key(KEY_A, false);
        // The input sink's path: what an EIS client, a remote desktop or a test delivers.
        // It is the same event through the same queue, and the whole point is that the two
        // sources cannot answer differently.
        let sink = session.control.input_sink();
        sink.inject(InputEvent::Key {
            keycode: KEY_A,
            pressed: true,
        });
        sink.inject(InputEvent::Key {
            keycode: KEY_A,
            pressed: false,
        });

        let state = client.wait_for(|state| state.keys.len() == 4);
        assert_eq!(
            state.keys,
            vec![KEY_A, KEY_A, KEY_A, KEY_A],
            "every edge must reach the client as the evdev code it was injected as: \
             a double offset would give {} and a missing one {}",
            KEY_A + 8,
            KEY_A.saturating_sub(8)
        );
    });
}

#[test]
fn a_keymap_change_under_a_held_key_keeps_its_target() {
    let config = Config {
        input_sources: Some(gnoblin_config::InputSources {
            sources: Some(vec![
                gnoblin_config::InputSource {
                    r#type: Some(gnoblin_config::InputSourceType::Xkb),
                    id: Some("us".to_owned()),
                },
                gnoblin_config::InputSource {
                    r#type: Some(gnoblin_config::InputSourceType::Xkb),
                    id: Some("gb".to_owned()),
                },
            ]),
            per_window: Some(false),
        }),
        ..Config::default()
    };
    let session = Session::with_config(config);
    session.run(|session| {
        let mut client = Client::app(session.socket());
        let _window = client.map_window("org.gnoblin.Layouts");
        session.focused_window();
        let first = client
            .wait_for(|state| state.keymaps.len() == 1)
            .last_keymap()
            .expect("a keymap")
            .to_owned();

        session.control.bind("test-key", "a");
        thread::sleep(Duration::from_millis(50));

        // The press is consumed by the binding; the layout changes while it is held, and
        // the release must still follow the press rather than reach the client as an
        // orphan edge.
        session.key(KEY_A, true);
        session.control.select_input_source(1);
        let state = client.wait_for(|state| state.keymaps.len() == 2);
        assert_ne!(state.last_keymap(), Some(first.as_str()), "the keymap changed");
        session.key(KEY_A, false);
        // An unbound key still reaches the client, so waiting for it proves the release
        // above was processed and not merely slow: the only keys delivered are its own.
        session.key(KEY_B, true);
        session.key(KEY_B, false);

        let state = client.wait_for(|state| state.keys.len() == 2);
        assert_eq!(
            state.keys,
            vec![KEY_B, KEY_B],
            "the held key's release was consumed with its press and never leaked"
        );
    });
}

/// A point inside `target` that none of `others` covers, scanned from the top left.
fn point_only_in(target: gnoblin_types::Rect, others: &[gnoblin_types::Rect]) -> Point {
    for y in (i64::from(target.y)..target.bottom()).step_by(8) {
        for x in (i64::from(target.x)..target.right()).step_by(8) {
            let covered = others.iter().any(|other| {
                x >= i64::from(other.x) && x < other.right() && y >= i64::from(other.y) && y < other.bottom()
            });
            if !covered {
                return Point::new(x as i32, y as i32);
            }
        }
    }
    panic!("the window has no point that no other window covers");
}

#[test]
fn a_keyboard_repeat_committed_after_start_reaches_the_client() {
    // The runtime commits its document after the session started: a repeat rate and delay
    // committed then must reach the keyboards clients already hold, not wait for a restart.
    let session = Session::with_config(Config::default());
    session.run(|session| {
        let mut client = Client::app(session.socket());
        let _window = client.map_window("org.gnoblin.Repeat");
        client.wait_for(|state| state.repeat_info.is_some());

        session.control.set_keyboard_settings(gnoblin_config::Keyboard {
            delay: Some(gnoblin_config::Inherit::Value(250)),
            repeat_interval: Some(gnoblin_config::Inherit::Value(50)),
            ..Default::default()
        });
        // A 50 ms interval is 20 keys a second.
        client.wait_for(|state| state.repeat_info == Some((20, 250)));

        session.control.set_keyboard_settings(gnoblin_config::Keyboard {
            repeat: Some(gnoblin_config::Inherit::Value(false)),
            delay: Some(gnoblin_config::Inherit::Value(250)),
            ..Default::default()
        });
        // Repeat turned off reaches the client as a rate of zero.
        client.wait_for(|state| state.repeat_info == Some((0, 250)));
    });
}

#[test]
fn a_layout_committed_after_start_reaches_the_client() {
    let source = |id: &str| gnoblin_config::InputSource {
        r#type: Some(gnoblin_config::InputSourceType::Xkb),
        id: Some(id.to_owned()),
    };
    let config = Config {
        input_sources: Some(gnoblin_config::InputSources {
            sources: Some(vec![source("us")]),
            per_window: Some(false),
        }),
        ..Config::default()
    };
    let session = Session::with_config(config);
    session.run(move |session| {
        let mut client = Client::app(session.socket());
        let _window = client.map_window("org.gnoblin.Layouts");
        session.focused_window();
        let first = client
            .wait_for(|state| state.keymaps.len() == 1)
            .last_keymap()
            .expect("a keymap")
            .to_owned();

        session.control.set_input_sources(gnoblin_config::InputSources {
            sources: Some(vec![source("gb")]),
            per_window: Some(false),
        });
        let state = client.wait_for(|state| state.keymaps.len() == 2);
        assert_ne!(
            state.last_keymap(),
            Some(first.as_str()),
            "the committed layout is installed"
        );
    });
}

#[test]
fn a_window_remembers_its_layout() {
    let config = Config {
        input_sources: Some(gnoblin_config::InputSources {
            sources: Some(vec![
                gnoblin_config::InputSource {
                    r#type: Some(gnoblin_config::InputSourceType::Xkb),
                    id: Some("us".to_owned()),
                },
                gnoblin_config::InputSource {
                    r#type: Some(gnoblin_config::InputSourceType::Xkb),
                    id: Some("gb".to_owned()),
                },
            ]),
            per_window: Some(true),
        }),
        wm: WmOptions {
            new_window_focus: NewWindowFocus::Allow,
            ..WmOptions::default()
        },
        ..Config::default()
    };
    let session = Session::with_config(config);
    session.run(|session| {
        let mut client = Client::app(session.socket());
        let first = client.map_window("org.gnoblin.First");
        let geometry = session.focused_window();
        let first_id = session
            .wait_for(|observation| observation.only().is_some())
            .only()
            .expect("the first window")
            .id;
        let us = client
            .wait_for(|state| state.keymaps.len() == 1)
            .last_keymap()
            .expect("a keymap")
            .to_owned();

        // The first window takes the next layout: it has focus while the switch happens.
        session.control.select_input_source(1);
        let gb = client
            .wait_for(|state| state.keymaps.len() == 2)
            .last_keymap()
            .expect("a keymap")
            .to_owned();
        assert_ne!(us, gb, "the two layouts produce different keymaps");

        // A new window has no layout of its own yet, so focusing it keeps the active one
        // and sends no keymap.
        let second = client.map_window("org.gnoblin.Second");
        session.control.move_window(
            first_id,
            gnoblin_types::Rect::new(geometry.x, geometry.y, geometry.width, geometry.height),
        );
        let observation = session.wait_for(|observation| observation.windows().count() == 2);
        let windows: Vec<_> = observation.windows().collect();
        let geometry_of = |app_id: &str| {
            windows
                .iter()
                .find(|window| window.app_id == app_id)
                .expect("the window")
                .geometry
        };
        let first_geometry = geometry_of("org.gnoblin.First");
        let second_geometry = geometry_of("org.gnoblin.Second");
        // The second window is mapped last, so it is topmost and focused. Each click lands
        // on a point only its own window covers, so the click reaches the window it names.
        let first_only = point_only_in(drawn_buffer(first_geometry), &[drawn_buffer(second_geometry)]);
        let second_only = point_only_in(drawn_buffer(second_geometry), &[drawn_buffer(first_geometry)]);

        // Focus the first window, then the second, which has no layout of its own: it keeps
        // the active one and sends no keymap.
        let enters = client.state.keyboard_enters;
        session.click(first_only);
        client.wait_for(|state| state.keyboard_enters > enters);
        let enters = client.state.keyboard_enters;
        session.click(second_only);
        client.wait_for(|state| state.keyboard_enters > enters);
        assert_eq!(
            client.state.last_keymap(),
            Some(gb.as_str()),
            "a window with no layout of its own inherits the active one"
        );

        // Switching while the second window is focused gives it its own layout, and
        // focusing the first window again restores the one it remembered.
        let sent = client.state.keymaps.len();
        session.control.select_input_source(0);
        client.wait_for(|state| state.keymaps.len() > sent);
        let sent = client.state.keymaps.len();
        session.click(first_only);
        let state = client.wait_for(|state| state.keymaps.len() > sent);
        assert_eq!(
            state.last_keymap(),
            Some(gb.as_str()),
            "the first window got its remembered layout back"
        );
        let _ = (first, second);
    });
}

#[test]
fn text_input_enter_and_commit_round_trip() {
    let session = Session::start();
    session.run(|session| {
        // The input method first: a text input's requests are discarded until one exists,
        // which is the protocol's rule and Smithay's.
        let mut ime_client = Client::connect(session.socket());
        let ime_manager: zwp_input_method_manager_v2::ZwpInputMethodManagerV2 = ime_client
            .globals
            .bind(&ime_client.qh, 1..=1, ())
            .expect("the session advertises zwp_input_method_manager_v2");
        let ime_seat = ime_client.seat.clone();
        let method = ime_manager.get_input_method(&ime_seat, &ime_client.qh, ());

        let mut client = Client::app(session.socket());
        let window = client.map_window("org.gnoblin.Typing");
        session.focused_window();

        let manager: zwp_text_input_manager_v3::ZwpTextInputManagerV3 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises zwp_text_input_manager_v3");
        let seat = client.seat.clone();
        let input = manager.get_text_input(&seat, &client.qh, ());
        // A text input created on a surface that already has focus is told it has focus by
        // the session rather than by a focus change. At this point the input method above
        // is still only in this connection's buffer, so the session has no instance of it:
        // an `enter` that arrives now can only be the session's own. It is counted rather
        // than timed, because the input method's arrival greets the input again below.
        client.wait_for(|state| state.text_enters >= 1);
        // The input method reaches the session now, before the client asks to be served.
        ime_client.roundtrip();
        input.enable();
        input.commit();
        // `enable` and `commit` reach the session on a flush, and the wait below is on the
        // other connection: without this round trip the request would still be in this
        // connection's buffer when the test starts waiting.
        client.roundtrip();
        ime_client.wait_for(|state| state.ime_activates == 1);

        // The input method's commit reaches the focused text input.
        let serial = ime_client.state.ime_dones;
        method.commit_string("hello".to_owned());
        method.commit(serial);
        // `commit_string` and `commit` reach the session on a flush of the input method's
        // own connection, and the wait below is on the application's.
        ime_client.roundtrip();
        let state = client.wait_for(|state| !state.text_commits.is_empty());
        assert_eq!(state.text_commits, vec!["hello".to_owned()]);
        let _ = window;
    });
}

#[test]
fn a_sandboxed_client_does_not_see_the_privileged_globals() {
    let session = Session::start();
    session.run(|session| {
        let mut host = Client::connect(session.socket());
        let manager: wp_security_context_manager_v1::WpSecurityContextManagerV1 = host
            .globals
            .bind(&host.qh, 1..=1, ())
            .expect("the session advertises wp_security_context_manager_v1");

        // The sandbox's own socket: the compositor accepts from it and marks what it
        // accepts.
        let directory = tempfile::tempdir().expect("a directory");
        let path = directory.path().join("sandbox.sock");
        let listener = UnixListener::bind(&path).expect("a listening socket");
        let (close_client, close_server) = UnixStream::pair().expect("a pair");
        let context = manager.create_listener(listener.as_fd(), close_server.as_fd(), &host.qh, ());
        context.set_sandbox_engine("test".to_owned());
        context.set_app_id("org.gnoblin.Sandboxed".to_owned());
        context.commit();
        host.roundtrip();

        let stream = UnixStream::connect(&path).expect("the sandbox's socket is served");
        let connection = Connection::from_socket(stream).expect("a sandboxed connection");
        let (globals, queue) = registry_queue_init::<App>(&connection).expect("the global list");
        let names: Vec<String> = globals
            .contents()
            .clone_list()
            .iter()
            .map(|global| global.interface.clone())
            .collect();
        assert!(
            names.iter().any(|name| name == "wl_compositor"),
            "a sandboxed client is a real client: {names:?}"
        );
        for privileged in [
            "zwp_virtual_keyboard_manager_v1",
            "zwp_input_method_manager_v2",
            "wp_security_context_manager_v1",
        ] {
            assert!(
                !names.iter().any(|name| name == privileged),
                "a sandboxed client must not see {privileged}: {names:?}"
            );
        }
        drop(queue);
        drop(close_client);
        let _ = context;
    });
}
