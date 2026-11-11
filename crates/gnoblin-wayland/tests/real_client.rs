//! The real-client round trip: a wayland-client connection against the server.
//!
//! Each test starts a session on a socket in its own temporary directory, connects a
//! real `wayland-client`, binds the globals, creates an `xdg_toplevel`, receives a
//! configure, acknowledges it, commits a `wl_shm` buffer and then reads the window
//! state the server published. Nothing here needs a display, a seat device or a
//! compositor from the environment: the socket, the buffers and the input are all
//! the test's own.
//!
//! ## Two threads, on purpose
//!
//! A calloop loop is not `Send` and neither is the backend it drives, so the session
//! runs on the test's own thread and the client choreography runs on a worker. That
//! is also closer to the real arrangement: the compositor is one process, the client
//! is the other one.
//!
//! The tests below are the crate's "failure modes first" list, one to an item; the
//! crate documentation names which test covers which mode.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::InputEvent;
use gnoblin_backend::headless::Headless;
use gnoblin_layers::InputPolicy;
use gnoblin_render_frame::frame::Overlay;
use gnoblin_render_frame::image::Image;
use gnoblin_types::{Point, Rect, WindowId};
use gnoblin_wayland::{
    Clock, Config, ControlHandle, LoopError, ManualClock, Modal, ModalEvent, Server, SessionSnapshot, SystemClock,
};
use gnoblin_wm::{FocusMode, NewWindowFocus, POINTER_REST, WmOptions};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::xdg::shell::client::{xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base};

/// How long a test waits for the session to reach a state before failing.
const TIMEOUT: Duration = Duration::from_secs(10);

/// `BTN_LEFT` from `linux/input-event-codes.h`.
const BTN_LEFT: u32 = 0x110;

/// What a worker thread can reach of a running session.
#[derive(Clone)]
struct Handle {
    socket: PathBuf,
    control: ControlHandle,
    observation: Arc<Mutex<SessionSnapshot>>,
}

impl Handle {
    fn socket(&self) -> &Path {
        &self.socket
    }

    /// The window state right now.
    fn windows(&self) -> SessionSnapshot {
        self.observation.lock().expect("the observation lock").clone()
    }

    /// Waits for the window state to satisfy `predicate`, or fails with the last one.
    fn wait_for(&self, predicate: impl Fn(&SessionSnapshot) -> bool) -> SessionSnapshot {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let observation = self.windows();
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
}

/// Stops the session when the worker finishes, including when it panics: without
/// this a failing assertion would leave the loop running and hang the test.
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
    fn start(backend: Headless) -> Self {
        Self::start_with_timeout(backend, Config::default().check_alive_timeout)
    }

    /// Creates a session whose clients must answer a ping within `timeout`.
    fn start_with_timeout(backend: Headless, timeout: Duration) -> Self {
        Self::start_configured(
            backend,
            Config {
                check_alive_timeout: timeout,
                ..Config::default()
            },
        )
    }

    /// Creates a session where a new window takes focus, as `focus_new_windows = allow`
    /// configures it. The default session refuses that, which is the documented default.
    fn start_allowing_focus(backend: Headless) -> Self {
        Self::start_configured(
            backend,
            Config {
                wm: WmOptions {
                    new_window_focus: NewWindowFocus::Allow,
                    ..WmOptions::default()
                },
                ..Config::default()
            },
        )
    }

    /// Creates a session on the socket described by `config`, with a socket of its own.
    fn start_configured(backend: Headless, config: Config) -> Self {
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
        };
        Self {
            directory,
            server,
            handle,
        }
    }

    /// Runs the session on this thread and `body` on a worker, then joins both.
    ///
    /// The socket is bound before `start` returns, so the worker may connect at
    /// once: a connection accepted before the loop runs waits in the backlog.
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
            // A failed assertion in the worker is the test's failure.
            std::panic::resume_unwind(payload);
        }
        outcome.expect("the loop stops cleanly");
    }
}

/// What a test's client records from the events it receives.
#[derive(Default)]
struct App {
    /// How many `xdg_surface.configure` events arrived.
    configures: u32,
    /// How many of them the client acknowledged.
    acks: u32,
    /// The last serial the server sent, for a grab.
    last_serial: u32,
    /// Do not acknowledge a configure, which is the client that never acks.
    no_ack: bool,
    /// How many `xdg_toplevel.configure` events arrived.
    toplevel_configures: u32,
    /// The size the server last suggested.
    width: i32,
    height: i32,
    /// The `xdg_toplevel.states` of the last configure, as raw protocol values.
    states: Vec<xdg_toplevel::State>,
    /// The bounds from the last `xdg_toplevel.configure_bounds`, if one arrived.
    bounds: Option<(i32, i32)>,
    /// Whether the server asked the window to close.
    closed: bool,
    /// How many times the keyboard entered this client's surface.
    keyboard_enters: u32,
    /// The surface the keyboard entered last, cleared when it leaves.
    keyboard_surface: Option<wl_surface::WlSurface>,
    /// How many times the pointer entered this client's surface.
    pointer_enters: u32,
    /// How many times the pointer left this client's surface.
    pointer_leaves: u32,
    /// How many pointer button events the client received.
    pointer_buttons: u32,
    /// The surface point of the last pointer enter or motion, cleared when the pointer leaves.
    pointer_at: Option<(f64, f64)>,
    /// How many pings the server sent.
    pings: u32,
    /// How many popup configures arrived.
    popup_configures: u32,
    /// Whether the server dismissed a popup.
    popup_done: bool,
    /// How many key edges the keyboard delivered.
    keys: u32,
    /// Whether the client holds its pings back instead of answering them.
    silent: bool,
    /// The pings held back while `silent`, with the `xdg_wm_base` that received each.
    held: Vec<(xdg_wm_base::XdgWmBase, u32)>,
}

/// A real Wayland client for one connection.
struct Client {
    /// The client owns its connection so the socket closes when the client is
    /// dropped, which is what the ghost-window test relies on.
    #[allow(dead_code)]
    connection: Connection,
    queue: EventQueue<App>,
    state: App,
    qh: QueueHandle<App>,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    wm_base: xdg_wm_base::XdgWmBase,
    seat: wl_seat::WlSeat,
}

/// The objects of one toplevel.
struct Window {
    surface: wl_surface::WlSurface,
    xdg_surface: xdg_surface::XdgSurface,
    toplevel: xdg_toplevel::XdgToplevel,
}

impl Client {
    /// Connects and binds the globals the session advertises.
    fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("the session is listening");
        let connection = Connection::from_socket(stream).expect("a client connection");
        let (globals, queue) = registry_queue_init::<App>(&connection).expect("the global list");
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor = globals
            .bind(&qh, 1..=1, ())
            .expect("the session advertises wl_compositor");
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).expect("the session advertises wl_shm");
        let wm_base: xdg_wm_base::XdgWmBase = globals
            .bind(&qh, 1..=6, ())
            .expect("the session advertises xdg_wm_base");
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).expect("the session advertises wl_seat");
        Self {
            connection,
            queue,
            state: App::default(),
            qh,
            compositor,
            shm,
            wm_base,
            seat,
        }
    }

    /// Processes every event the server has sent.
    fn roundtrip(&mut self) {
        self.queue.roundtrip(&mut self.state).expect("a round trip");
    }

    /// Creates a toplevel and commits it once, which is what asks for a configure.
    fn create_window(&mut self, app_id: &str, title: &str) -> Window {
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
    fn attach_buffer(&mut self, window: &Window, width: i32, height: i32) {
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
    fn map_window(&mut self, app_id: &str, title: &str) -> Window {
        let window = self.create_window(app_id, title);
        self.attach_buffer(&window, 200, 150);
        window
    }

    /// Destroys a toplevel the way a well-behaved client does.
    fn destroy(&mut self, window: Window) {
        window.toplevel.destroy();
        window.xdg_surface.destroy();
        window.surface.destroy();
        self.roundtrip();
    }

    /// Sends a pointer press at a global position, as the backend would.
    fn click(&mut self, session: &Handle, at: Point) {
        let output = session.windows().outputs().first().copied().expect("an output");
        session
            .control
            .inject_input(InputEvent::PointerMotion { output, position: at });
        session.control.inject_input(InputEvent::PointerButton {
            button: BTN_LEFT,
            pressed: true,
        });
        session.control.inject_input(InputEvent::PointerButton {
            button: BTN_LEFT,
            pressed: false,
        });
        // The press is delivered by the loop, so read the events it caused.
        self.roundtrip();
    }

    /// Sends a pointer motion to a global position, as the backend would, without waiting.
    fn inject_motion(&self, session: &Handle, at: Point) {
        let output = session.windows().outputs().first().copied().expect("an output");
        session
            .control
            .inject_input(InputEvent::PointerMotion { output, position: at });
    }

    /// Moves the pointer to `at` and waits until it crosses into or out of a surface, which is
    /// the event the window manager's focus decision follows.
    fn move_pointer(&mut self, session: &Handle, at: Point) {
        let crossings = self.crossings();
        self.inject_motion(session, at);
        self.wait_until(&format!("the pointer to cross a surface edge at {at:?}"), |seen| {
            seen.crossings() != crossings
        });
    }

    /// Sends a left button press or release and waits until the client receives it.
    fn left_button(&mut self, session: &Handle, pressed: bool) {
        let buttons = self.state.pointer_buttons;
        session.control.inject_input(InputEvent::PointerButton {
            button: BTN_LEFT,
            pressed,
        });
        self.wait_until("the button event", |seen| seen.state.pointer_buttons != buttons);
    }

    /// Pointer enters and leaves seen so far.
    fn crossings(&self) -> u32 {
        self.state.pointer_enters + self.state.pointer_leaves
    }

    /// Reads events until `done` holds, failing after `TIMEOUT`.
    fn wait_until(&mut self, what: &str, done: impl Fn(&Client) -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        while !done(&*self) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            self.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Whether the keyboard is on `window`'s surface, as of the last round trip.
    fn keyboard_is_on(&self, window: &Window) -> bool {
        self.state.keyboard_surface.as_ref() == Some(&window.surface)
    }
}

/// The rectangle a window's drawn content covers on the output. Only drawn content takes
/// pointer input, so a window's frame, which the client may not fill, is not the target.
fn drawn_box(session: &Handle, window: WindowId) -> Rect {
    let (matrix, bounds) = drawn(session, window);
    let corners = [
        (0.0, 0.0),
        (f64::from(bounds.width), 0.0),
        (0.0, f64::from(bounds.height)),
        (f64::from(bounds.width), f64::from(bounds.height)),
    ]
    .map(|local| drawn_at(&matrix, local));
    let left = corners.iter().map(|p| p.x).min().expect("four corners");
    let top = corners.iter().map(|p| p.y).min().expect("four corners");
    let right = corners.iter().map(|p| p.x).max().expect("four corners");
    let bottom = corners.iter().map(|p| p.y).max().expect("four corners");
    Rect::new(left, top, (right - left) as u32, (bottom - top) as u32)
}

/// A point inside `title`'s drawn content that no other window's drawn content covers, so a
/// press there reaches it.
///
/// Windows overlap and a newer one can cover the centre of an older one; a press must land
/// where only the wanted window is.
fn only_point_of(session: &Handle, title: &str) -> Point {
    let observation = session.wait_for(|observation| observation.titled(title).is_some());
    let window = observation.titled(title).expect("the window").id;
    let target = drawn_box(session, window);
    let others: Vec<_> = observation
        .windows()
        .filter(|other| other.title != title)
        .map(|other| drawn_box(session, other.id))
        .collect();
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
    panic!("the window {title} has no point that no other window covers");
}

/// The logical size of the default headless output.
fn headless_screen() -> (i32, i32) {
    (
        gnoblin_backend::headless::DEFAULT_MODE.width as i32,
        gnoblin_backend::headless::DEFAULT_MODE.height as i32,
    )
}

/// A session that takes focus under `focus_mode`, on the system clock.
fn focus_session(focus_mode: FocusMode) -> Session {
    session_on(
        WmOptions {
            focus_mode,
            ..WmOptions::default()
        },
        Arc::new(SystemClock),
    )
}

/// A hover session whose pointer rests before a hover focus change: the change waits
/// `POINTER_REST` on the new window, and only a test advances the clock that measures it.
fn rest_session() -> Session {
    rest_session_on(Arc::new(ManualClock::new()))
}

/// The rest session on a clock the test owns, so the test decides when wall time passes.
fn rest_session_on(clock: Arc<ManualClock>) -> Session {
    session_on(
        WmOptions {
            focus_mode: FocusMode::Hover,
            focus_change_on_pointer_rest: true,
            ..WmOptions::default()
        },
        clock,
    )
}

/// A session with the given window manager options on the clock the caller chooses. A
/// [`ManualClock`] moves only when its owner moves it, so a rest timer never expires during a
/// client round trip that the test did not ask for.
fn session_on(wm: WmOptions, clock: Arc<dyn Clock>) -> Session {
    Session::start_configured(
        Headless::new(),
        Config {
            wm,
            clock,
            ..Config::default()
        },
    )
}

/// Maps `Left` and `Right` on the left and right halves of the output and waits until both are
/// drawn there. The two halves touch, so the output has no empty space between the windows.
fn map_split_pair(session: &Handle, client: &mut Client) -> (Window, Window) {
    map_pair(session, client, SPLIT_LEFT, SPLIT_RIGHT)
}

/// A point outside both windows of the split pair and outside the output. The two windows
/// cover the whole output, so no point inside it is clear of both.
fn outside_output_point() -> Point {
    let (width, height) = headless_screen();
    Point::new(width + 50, height / 2)
}

#[test]
fn a_real_client_maps_a_window_and_gets_focus() {
    let session = Session::start_allowing_focus(Headless::new());
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.gnoblin.Test", "First");

        let observation = session
            .wait_for(|observation| observation.len() == 1 && observation.only().is_some_and(|window| window.mapped));
        let managed = observation.only().expect("one managed window");
        assert_eq!(managed.title, "First", "the title the client sent");
        assert_eq!(managed.app_id, "org.gnoblin.Test", "the app id the client sent");
        assert!(managed.visible, "the window is on the only output and workspace");
        assert!(managed.focused, "the first mapped window takes focus");
        assert!(client.state.acks >= 1, "the client acknowledged a configure");
        assert!(
            client.state.toplevel_configures >= 1,
            "the client received a toplevel configure"
        );
        assert!(
            client.state.keyboard_enters >= 1,
            "the focused window received keyboard focus"
        );

        // The window is inside the only output, not at a position of its own.
        let (width, height) = headless_screen();
        assert!(
            managed.geometry.x >= 0
                && managed.geometry.y >= 0
                && managed.geometry.right() <= i64::from(width)
                && managed.geometry.bottom() <= i64::from(height),
            "the window is inside the output: {:?}",
            managed.geometry
        );
        assert_eq!(observation.outputs().len(), 1, "the session publishes one output");

        client.destroy(window);
        let observation = session.wait_for(|observation| observation.is_empty());
        assert!(observation.focused().is_none(), "nothing holds focus afterwards");
    });
}

#[test]
fn a_window_reports_the_pid_of_its_client_and_not_its_app_id() {
    let session = Session::start(Headless::new());
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        // The app id names another program. The pid must still be this process, the one
        // on the other end of the socket: attribution comes from the connection.
        let window = client.map_window("org.gnoblin.NotThisProcess", "Named");
        let observation = session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        let managed = observation.only().expect("one managed window");
        assert_eq!(
            managed.pid,
            Some(std::process::id()),
            "the pid is the client's, read from the socket"
        );
        assert_eq!(managed.app_id, "org.gnoblin.NotThisProcess");

        client.destroy(window);
        session.wait_for(|observation| observation.is_empty());
    });
}

#[test]
fn a_new_window_does_not_take_focus_under_the_default_policy() {
    let session = Session::start(Headless::new());
    session.run(|session| {
        let mut first = Client::connect(session.socket());
        let _alpha = first.map_window("org.gnoblin.Alpha", "Alpha");
        session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped && window.focused));

        // `focus_new_windows` defaults to `prevent`: a window that maps on its own
        // initiative is left unfocused, and the window that had focus keeps it.
        let mut second = Client::connect(session.socket());
        let _beta = second.map_window("org.gnoblin.Beta", "Beta");
        let observation = session.wait_for(|observation| {
            observation.len() == 2 && observation.titled("Beta").is_some_and(|window| window.mapped)
        });
        assert_eq!(
            observation.titled("Alpha").map(|window| window.focused),
            Some(true),
            "the focused window kept focus"
        );
        assert_eq!(
            observation.titled("Beta").map(|window| window.focused),
            Some(false),
            "the new window stayed unfocused"
        );
    });
}

#[test]
fn a_second_client_takes_focus_and_a_click_switches_it_back() {
    let session = Session::start_allowing_focus(Headless::new());
    session.run(|session| {
        let mut first = Client::connect(session.socket());
        let _alpha = first.map_window("org.gnoblin.Alpha", "Alpha");
        let observation =
            session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped && window.focused));
        let alpha_geometry = observation.only().expect("the first window").geometry;

        let mut second = Client::connect(session.socket());
        let _beta = second.map_window("org.gnoblin.Beta", "Beta");
        let observation = session.wait_for(|observation| {
            observation.len() == 2 && observation.focused().is_some_and(|window| window.title == "Beta")
        });
        assert_eq!(
            observation.titled("Alpha").map(|window| window.focused),
            Some(false),
            "the newer window took focus"
        );

        // A press on the first window is the activation request that moves focus back.
        let alpha = only_point_of(session, "Alpha");
        first.click(session, alpha);
        let observation =
            session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Alpha"));
        assert_eq!(
            observation.titled("Beta").map(|window| window.focused),
            Some(false),
            "the previous window lost focus"
        );
        assert_eq!(
            observation.focused().map(|window| window.geometry),
            Some(alpha_geometry),
            "the clicked window kept its geometry"
        );
        assert!(
            first.state.pointer_enters >= 1,
            "the pointer entered the clicked window"
        );
    });
}

#[test]
fn a_client_that_disconnects_leaves_no_ghost_window() {
    let session = Session::start(Headless::new());
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.gnoblin.Ghost", "Ghost");
        session
            .wait_for(|observation| observation.len() == 1 && observation.only().is_some_and(|window| window.mapped));
        // Drop the connection without destroying anything: the resources go with
        // the socket, which is what a crashed client does.
        drop(window);
        drop(client);

        let observation = session.wait_for(|observation| observation.is_empty());
        assert!(observation.focused().is_none(), "nothing holds focus");
    });
}

#[test]
fn a_configure_that_is_never_acknowledged_does_not_stall_the_session() {
    let session = Session::start_allowing_focus(Headless::new());
    session.run(|session| {
        let mut slow = Client::connect(session.socket());
        slow.state.no_ack = true;
        let slow_window = slow.create_window("org.gnoblin.Slow", "Slow");
        assert!(
            slow.state.toplevel_configures >= 1,
            "the server configured the window even though the client will not ack"
        );

        // Another client maps and takes focus while the first never acknowledges.
        let mut fast = Client::connect(session.socket());
        let _mapped = fast.map_window("org.gnoblin.Fast", "Fast");
        let observation = session.wait_for(|observation| {
            observation.len() == 2 && observation.focused().is_some_and(|window| window.title == "Fast")
        });
        let slow_entry = observation.titled("Slow").expect("the unacknowledging window");
        assert!(!slow_entry.mapped, "a window that never acked has no buffer");
        assert!(!slow_entry.focused, "it did not take focus from the mapped window");

        // A later state change still reaches the client that ignored the first
        // configure, so nothing is waiting on an acknowledgement.
        slow_window.toplevel.set_maximized();
        slow.roundtrip();
        let observation = session.wait_for(|observation| {
            observation
                .titled("Slow")
                .is_some_and(|window| window.geometry.width as i32 == headless_screen().0)
        });
        assert!(
            slow.state.toplevel_configures >= 2,
            "the second configure arrived without the first being acknowledged, got {}",
            slow.state.toplevel_configures
        );
        assert_eq!(
            observation.titled("Slow").map(|window| window.geometry.height as i32),
            Some(headless_screen().1),
            "the maximised window covers the work area"
        );
    });
}

#[test]
fn a_window_with_no_output_to_land_on_is_not_managed() {
    let session = Session::start(Headless::empty());
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let _window = client.create_window("org.gnoblin.Lost", "Lost");

        let observation = session.windows();
        assert!(
            observation.is_empty(),
            "a window with no output to be placed on is refused: {observation:#?}"
        );
        assert!(observation.focused().is_none(), "nothing holds focus");
        assert!(observation.outputs().is_empty());

        // The session is still serving: another client connects and completes a
        // round trip, which proves the loop and the display survived the refusal.
        let mut other = Client::connect(session.socket());
        other.roundtrip();
    });
}

#[test]
fn a_popup_grab_on_a_tracked_parent_keeps_the_popup() {
    let session = Session::start(Headless::new());
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.gnoblin.Menu", "Menu");
        session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));

        let surface = client.compositor.create_surface(&client.qh, ());
        let xdg_surface = client.wm_base.get_xdg_surface(&surface, &client.qh, ());
        let positioner = client.wm_base.create_positioner(&client.qh, ());
        positioner.set_size(120, 60);
        positioner.set_anchor_rect(0, 0, 10, 10);
        // A popup of a managed window, grabbing before it maps: Smithay hands the
        // grab to the handler on the commit, and this slice has no pointer-grab
        // machinery, so the popup stays up rather than being half-dismissed.
        let popup = xdg_surface.get_popup(Some(&window.xdg_surface), &positioner, &client.qh, ());
        popup.grab(&client.seat, client.state.last_serial);
        surface.commit();
        client.roundtrip();

        assert!(
            client.state.popup_configures >= 1,
            "the popup was configured, so it is tracked under its parent"
        );
        assert!(!client.state.popup_done, "the popup was not dismissed");
    });
}

#[test]
fn a_popup_grab_without_a_parent_does_not_stop_the_session() {
    let session = Session::start_allowing_focus(Headless::new());
    session.run(|session| {
        let mut client = Client::connect(session.socket());

        // A popup with no parent at all, asking for a grab and then mapping. A
        // parentless popup that maps is a protocol error, which Smithay raises on
        // this client before any grab can be decided.
        let surface = client.compositor.create_surface(&client.qh, ());
        let xdg_surface = client.wm_base.get_xdg_surface(&surface, &client.qh, ());
        let positioner = client.wm_base.create_positioner(&client.qh, ());
        positioner.set_size(120, 60);
        positioner.set_anchor_rect(0, 0, 10, 10);
        let popup = xdg_surface.get_popup(None, &positioner, &client.qh, ());
        popup.grab(&client.seat, client.state.last_serial);
        surface.commit();
        let first = client.queue.roundtrip(&mut client.state);
        let mut disconnected = first.is_err();
        if !disconnected {
            disconnected = client.queue.roundtrip(&mut client.state).is_err();
        }
        assert!(
            disconnected,
            "the parentless popup was refused on this client only: {first:?}"
        );
        drop(client);

        // The session keeps serving another client.
        let mut other = Client::connect(session.socket());
        let _window = other.map_window("org.gnoblin.After", "After");
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "After"));
    });
}

#[test]
fn a_protocol_error_from_a_client_does_not_stop_the_compositor() {
    let session = Session::start_allowing_focus(Headless::new());
    session.run(|session| {
        let mut bad = Client::connect(session.socket());
        let _window = bad.map_window("org.gnoblin.Bad", "Bad");
        session
            .wait_for(|observation| observation.len() == 1 && observation.only().is_some_and(|window| window.mapped));

        // A client bug: a stride shorter than one row of pixels. The server answers
        // with a `wl_shm` protocol error, which ends this client's connection.
        let file = tempfile::tempfile().expect("a shared memory file");
        file.set_len(4096).expect("a pool that big");
        let pool = bad.shm.create_pool(file.as_fd(), 4096, &bad.qh, ());
        let _buffer = pool.create_buffer(0, 100, 100, 10, wl_shm::Format::Argb8888, &bad.qh, ());
        let first = bad.queue.roundtrip(&mut bad.state);
        let mut disconnected = first.is_err();
        if !disconnected {
            // The error may arrive just after the round trip's sync callback.
            disconnected = bad.queue.roundtrip(&mut bad.state).is_err();
        }
        assert!(disconnected, "the protocol error reached the client: {first:?}");
        drop(bad);

        // Another client maps, is focused and is the only window: the failed
        // client's toplevel went with its connection.
        let mut good = Client::connect(session.socket());
        let _window = good.map_window("org.gnoblin.Good", "Good");
        let observation = session.wait_for(|observation| {
            observation.len() == 1 && observation.focused().is_some_and(|window| window.title == "Good")
        });
        assert_eq!(
            observation.only().map(|window| window.title.as_str()),
            Some("Good"),
            "the failed client left nothing behind"
        );
    });
}

#[test]
fn a_socket_left_by_a_crashed_session_is_replaced() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let socket: PathBuf = directory.path().join("wayland-0.sock");
    // A plain file where the socket belongs: binding fails unless the stale path is
    // removed first.
    std::fs::write(&socket, b"stale").expect("a stale file");
    let config = Config {
        socket_path: Some(socket.clone()),
        ..Config::default()
    };
    let server = Server::new(Box::new(Headless::new()), config).expect("the stale socket is replaced");
    let control = server.control();
    let worker = thread::spawn({
        let socket = socket.clone();
        move || {
            let _stop = StopOnDrop(control);
            let mut client = Client::connect(&socket);
            client.roundtrip();
        }
    });
    server.run().expect("the loop stops cleanly");
    worker.join().expect("the client connected and left");
    assert!(!socket.exists(), "the socket is removed when the session stops");
}

#[test]
fn a_client_that_never_pongs_is_unresponsive_until_it_answers() {
    let session = Session::start_with_timeout(Headless::new(), Duration::from_millis(1000));
    session.run(|session| {
        let mut silent = Client::connect(session.socket());
        silent.state.silent = true;
        let _window = silent.map_window("org.gnoblin.Silent", "Silent");
        session.wait_for(|observation| observation.titled("Silent").is_some_and(|window| window.mapped));
        for _ in 0..5 {
            session.control.advance(600);
            silent.roundtrip();
        }
        let observation =
            session.wait_for(|observation| observation.titled("Silent").is_some_and(|window| window.unresponsive));
        assert!(observation.titled("Silent").is_some_and(|window| window.unresponsive));

        // Answering the held pings clears the mark.
        silent.state.silent = false;
        for (wm_base, serial) in std::mem::take(&mut silent.state.held) {
            wm_base.pong(serial);
        }
        silent.roundtrip();
        session.wait_for(|observation| observation.titled("Silent").is_some_and(|window| !window.unresponsive));
    });
}

#[test]
fn a_client_that_answers_its_pings_is_never_unresponsive() {
    let session = Session::start_with_timeout(Headless::new(), Duration::from_millis(1000));
    session.run(|session| {
        let mut prompt = Client::connect(session.socket());
        let _window = prompt.map_window("org.gnoblin.Prompt", "Prompt");
        session.wait_for(|observation| observation.titled("Prompt").is_some_and(|window| window.mapped));
        for _ in 0..10 {
            session.control.advance(300);
            prompt.roundtrip();
        }
        assert!(
            prompt.state.pings >= 2,
            "the client was pinged more than once, got {}",
            prompt.state.pings
        );
        assert!(
            !session
                .windows()
                .titled("Prompt")
                .is_some_and(|window| window.unresponsive),
            "a client that answers is never marked"
        );
    });
}

#[test]
fn a_zero_timeout_turns_the_pings_off() {
    let session = Session::start_with_timeout(Headless::new(), Duration::ZERO);
    session.run(|session| {
        let mut silent = Client::connect(session.socket());
        silent.state.silent = true;
        let _window = silent.map_window("org.gnoblin.Quiet", "Quiet");
        session.wait_for(|observation| observation.titled("Quiet").is_some_and(|window| window.mapped));
        session.control.advance(60_000);
        silent.roundtrip();
        assert_eq!(silent.state.pings, 0, "no ping is sent when the timeout is zero");
        assert!(
            !session
                .windows()
                .titled("Quiet")
                .is_some_and(|window| window.unresponsive)
        );
    });
}

// The interfaces this client binds but only ever listens to.

wayland_client::delegate_noop!(App: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(App: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(App: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(App: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(App: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(App: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(App: ignore xdg_positioner::XdgPositioner);

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
        _: &mut Self,
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
            if capabilities.contains(wl_seat::Capability::Keyboard) {
                seat.get_keyboard(qh, ());
            }
            if capabilities.contains(wl_seat::Capability::Pointer) {
                seat.get_pointer(qh, ());
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
            wl_keyboard::Event::Enter { surface, .. } => {
                state.keyboard_enters += 1;
                state.keyboard_surface = Some(surface);
            }
            wl_keyboard::Event::Leave { .. } => state.keyboard_surface = None,
            wl_keyboard::Event::Key { .. } => state.keys += 1,
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for App {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
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
                state.pointer_at = Some((surface_x, surface_y));
            }
            wl_pointer::Event::Leave { .. } => {
                state.pointer_leaves += 1;
                state.pointer_at = None;
            }
            wl_pointer::Event::Button { .. } => state.pointer_buttons += 1,
            wl_pointer::Event::Motion {
                surface_x, surface_y, ..
            } => state.pointer_at = Some((surface_x, surface_y)),
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
            state.pings += 1;
            if state.silent {
                state.held.push((wm_base.clone(), serial));
            } else {
                wm_base.pong(serial);
            }
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
            state.configures += 1;
            state.last_serial = serial;
            if !state.no_ack {
                xdg_surface.ack_configure(serial);
                state.acks += 1;
            }
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for App {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: <xdg_toplevel::XdgToplevel as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure { width, height, states } => {
                state.toplevel_configures += 1;
                state.width = width;
                state.height = height;
                // Each state is a native-endian u32 in the byte array.
                state.states = states
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter_map(|raw| {
                        let value = u32::from_ne_bytes([raw[0], raw[1], raw[2], raw[3]]);
                        xdg_toplevel::State::try_from(value).ok()
                    })
                    .collect();
            }
            xdg_toplevel::Event::ConfigureBounds { width, height } => {
                state.bounds = Some((width, height));
            }
            xdg_toplevel::Event::Close => state.closed = true,
            _ => {}
        }
    }
}

impl Dispatch<xdg_popup::XdgPopup, ()> for App {
    fn event(
        state: &mut Self,
        _: &xdg_popup::XdgPopup,
        event: <xdg_popup::XdgPopup as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_popup::Event::Configure { .. } => state.popup_configures += 1,
            xdg_popup::Event::PopupDone => state.popup_done = true,
            _ => {}
        }
    }
}

/// The rectangles the pair tests place their windows at: two windows with a gap between them.
const PAIR_LEFT: Rect = Rect::new(100, 100, 300, 200);
/// The rectangle the pair tests place `Right` at.
const PAIR_RIGHT: Rect = Rect::new(600, 100, 300, 200);
/// The left half of the output, which the master tests tile.
const SPLIT_LEFT: Rect = Rect::new(0, 0, 512, 768);
/// The right half of the output.
const SPLIT_RIGHT: Rect = Rect::new(512, 0, 512, 768);
/// Two overlapping windows: `Right` covers the right part of `Left`.
const OVERLAP_LOWER: Rect = Rect::new(100, 100, 300, 200);
/// The window above `OVERLAP_LOWER`, shifted right by 100.
const OVERLAP_UPPER: Rect = Rect::new(200, 100, 300, 200);

/// Pumps the client until the session observes a window titled `title`.
fn wait_titled(session: &Handle, client: &mut Client, title: &str) {
    let deadline = Instant::now() + TIMEOUT;
    while session.windows().titled(title).is_none() {
        assert!(Instant::now() < deadline, "the window {title} is never observed");
        thread::sleep(Duration::from_millis(5));
        client.roundtrip();
    }
}

/// Creates a window, places it at `rect` through the session's control, and configures and
/// draws it at that size. The placement is drawn at `rect` before this returns.
fn place_window(session: &Handle, client: &mut Client, app_id: &str, title: &str, rect: Rect) -> Window {
    let window = client.create_window(app_id, title);
    wait_titled(session, client, title);
    session.control.move_window(window_id(session, title), rect);
    settle_size(client, (rect.width as i32, rect.height as i32));
    draw_configured(client, &window);
    let deadline = Instant::now() + TIMEOUT;
    while drawn(session, window_id(session, title)).1 != rect {
        assert!(
            Instant::now() < deadline,
            "the window {title} is never drawn at {rect:?}"
        );
        thread::sleep(Duration::from_millis(5));
    }
    window
}

/// Places `Left` and `Right` on one client at the given rectangles.
fn map_pair(session: &Handle, client: &mut Client, left: Rect, right: Rect) -> (Window, Window) {
    let left_window = place_window(session, client, "org.gnoblin.Left", "Left", left);
    let right_window = place_window(session, client, "org.gnoblin.Right", "Right", right);
    (left_window, right_window)
}

/// Processes the client's events until its toplevel is configured to `size`.
fn settle_size(client: &mut Client, size: (i32, i32)) {
    let deadline = Instant::now() + TIMEOUT;
    while (client.state.width, client.state.height) != size {
        assert!(
            Instant::now() < deadline,
            "the client never received the configure {size:?}; last {:?}",
            (client.state.width, client.state.height)
        );
        thread::sleep(Duration::from_millis(5));
        client.roundtrip();
    }
}

/// Attaches a buffer of exactly the size the server configured, so the drawn node
/// has the placement's bounds.
fn draw_configured(client: &mut Client, window: &Window) {
    client.roundtrip();
    let (width, height) = (client.state.width, client.state.height);
    client.attach_buffer(window, width, height);
}

/// The window the session observes under `title`.
fn window_id(session: &Handle, title: &str) -> WindowId {
    session.windows().titled(title).expect("the window is observed").id
}

/// The bounds a window is drawn at, once it is drawn.
fn drawn(session: &Handle, window: WindowId) -> (gnoblin_render::Mat4, Rect) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(Some(node)) = session.control.drawn_node(window) {
            return node;
        }
        assert!(Instant::now() < deadline, "the window {window:?} is never drawn");
        thread::sleep(Duration::from_millis(5));
    }
}

/// Moves the pointer to `at`, lets the loop process it, and returns the surface point the client
/// received, or `None` when the pointer is over no surface of the client.
fn point_at(session: &Handle, client: &mut Client, at: Point) -> Option<(f64, f64)> {
    client.inject_motion(session, at);
    settle(session);
    client.roundtrip();
    client.state.pointer_at
}

/// Fails unless the client's pointer was at `got` and `want` is within two pixels of it.
fn assert_pointer_at(got: Option<(f64, f64)>, want: (f64, f64), what: &str) {
    let got = got.unwrap_or_else(|| panic!("{what}: the pointer reached no surface"));
    assert!(
        (got.0 - want.0).abs() <= 2.0 && (got.1 - want.1).abs() <= 2.0,
        "{what}: the pointer reached {got:?}, the drawn geometry predicts {want:?}"
    );
}

#[test]
fn overlapping_windows_at_equal_depth_go_to_the_last_drawn_and_a_raise_moves_the_winner() {
    let session = session_on(WmOptions::default(), Arc::new(ManualClock::new()));
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let _pair = map_pair(session, &mut client, OVERLAP_LOWER, OVERLAP_UPPER);
        session.wait_for(|observation| topmost(observation) == Some("Right"));

        // Both windows are flat at the same depth and cover (250, 150). Right is drawn last, so it
        // gets the pointer: its surface point is (50, 50), where Left's would be (150, 50).
        let overlap = Point::new(250, 150);
        assert_pointer_at(
            point_at(session, &mut client, overlap),
            (50.0, 50.0),
            "the topmost window",
        );

        // Raising Left by a press on its exposed part makes it the last drawn, so the same point
        // now reaches Left.
        client.click(session, only_point_of(session, "Left"));
        session.wait_for(|observation| topmost(observation) == Some("Left"));
        assert_pointer_at(
            point_at(session, &mut client, overlap),
            (150.0, 50.0),
            "the window raised above the other",
        );
    });
}

#[test]
fn a_modal_takes_the_keys_from_the_client_until_it_is_cleared() {
    let session = Session::start(Headless::new());
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.gnoblin.Test", "Modal");
        session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));

        let output = session.windows().outputs().first().copied().expect("an output");
        let image = Image::new(gnoblin_types::Size { width: 2, height: 2 }, 8, vec![0; 16]).expect("a 2x2 image");
        let overlay = Overlay::new(Rect::from_size(gnoblin_types::Size { width: 2, height: 2 }), &image)
            .expect("a non-empty overlay");
        session
            .control
            .set_modal(Some(Modal::new(InputPolicy::ModalToOutput, output, overlay)));

        session.control.inject_input(InputEvent::Key {
            keycode: 30,
            pressed: true,
        });
        assert_eq!(
            session.control.modal_events(),
            Some(vec![ModalEvent::Key {
                keycode: 30,
                state: gnoblin_types::KeyState::Pressed,
            }]),
            "the modal queued the key instead of the client"
        );
        client.roundtrip();
        assert_eq!(client.state.keys, 0, "the client received no key while the modal stood");

        session.control.set_modal(None);
        session.control.inject_input(InputEvent::Key {
            keycode: 30,
            pressed: true,
        });
        let deadline = Instant::now() + TIMEOUT;
        while client.state.keys == 0 {
            assert!(
                Instant::now() < deadline,
                "the client never received the key after the modal"
            );
            client.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            session.control.modal_events(),
            Some(Vec::new()),
            "no modal event after clearing"
        );

        client.destroy(window);
    });
}

#[test]
fn a_click_in_an_overlap_goes_to_the_topmost_window() {
    let session = Session::start_allowing_focus(Headless::new());
    session.run(|session| {
        let mut older = Client::connect(session.socket());
        older.map_window("org.gnoblin.Test", "Older");
        let mut newer = Client::connect(session.socket());
        newer.map_window("org.gnoblin.Test", "Newer");
        let observation = session.wait_for(|observation| {
            observation.titled("Older").is_some_and(|window| window.mapped)
                && observation.titled("Newer").is_some_and(|window| window.mapped)
        });
        let older_geometry = observation.titled("Older").expect("the older window").geometry;
        let newer_geometry = observation.titled("Newer").expect("the newer window").geometry;

        // The first point of the newer window that the older one also covers.
        let mut overlap = None;
        'search: for y in (i64::from(newer_geometry.y)..newer_geometry.bottom()).step_by(8) {
            for x in (i64::from(newer_geometry.x)..newer_geometry.right()).step_by(8) {
                let inside_older = x >= i64::from(older_geometry.x)
                    && x < older_geometry.right()
                    && y >= i64::from(older_geometry.y)
                    && y < older_geometry.bottom();
                if inside_older {
                    overlap = Some(Point::new(x as i32, y as i32));
                    break 'search;
                }
            }
        }
        let overlap = overlap.expect("the test windows overlap");

        newer.click(session, overlap);
        newer.roundtrip();
        older.roundtrip();
        assert!(newer.state.pointer_enters >= 1, "the topmost window received the click");
        assert_eq!(older.state.pointer_enters, 0, "the window underneath did not");
    });
}

#[test]
fn click_mode_leaves_focus_where_the_pointer_passes() {
    let session = focus_session(FocusMode::Click);
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let (first, second) = map_pair(session, &mut client, PAIR_LEFT, PAIR_RIGHT);
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Left"));
        client.roundtrip();
        assert!(client.keyboard_is_on(&first), "the first window holds the keyboard");

        let entered = client.state.pointer_enters;
        client.move_pointer(session, only_point_of(session, "Right"));
        assert!(
            client.state.pointer_enters > entered,
            "the pointer entered the second window"
        );
        assert!(client.keyboard_is_on(&first), "click mode does not follow the pointer");
        assert!(
            !client.keyboard_is_on(&second),
            "the second window did not take the keyboard"
        );
    });
}

#[test]
fn hover_focuses_the_window_the_pointer_enters() {
    let session = focus_session(FocusMode::Hover);
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let (first, second) = map_pair(session, &mut client, PAIR_LEFT, PAIR_RIGHT);
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Left"));

        client.move_pointer(session, only_point_of(session, "Right"));
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Right"));
        client.roundtrip();
        assert!(
            client.keyboard_is_on(&second),
            "the keyboard entered the window the pointer entered"
        );
        assert!(
            !client.keyboard_is_on(&first),
            "the window the pointer left lost the keyboard"
        );
    });
}

#[test]
fn hover_keeps_focus_when_the_pointer_crosses_a_gap() {
    let session = focus_session(FocusMode::Hover);
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let (_first, second) = map_split_pair(session, &mut client);
        client.move_pointer(session, only_point_of(session, "Right"));
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Right"));

        let left = client.state.pointer_leaves;
        client.move_pointer(session, outside_output_point());
        assert!(client.state.pointer_leaves > left, "the pointer left the second window");
        assert!(
            session
                .windows()
                .focused()
                .is_some_and(|window| window.title == "Right"),
            "a gap keeps focus under hover"
        );
        assert!(
            client.keyboard_is_on(&second),
            "the keyboard stayed on the second window"
        );
    });
}

#[test]
fn hover_strict_clears_focus_when_the_pointer_crosses_a_gap() {
    let session = focus_session(FocusMode::HoverStrict);
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let (_first, second) = map_split_pair(session, &mut client);
        client.move_pointer(session, only_point_of(session, "Right"));
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Right"));

        let left = client.state.pointer_leaves;
        client.move_pointer(session, outside_output_point());
        assert!(client.state.pointer_leaves > left, "the pointer left the second window");
        session.wait_for(|observation| observation.focused().is_none());
        client.roundtrip();
        assert!(!client.keyboard_is_on(&second), "the keyboard left the second window");
        assert!(client.state.keyboard_surface.is_none(), "nothing holds the keyboard");
    });
}

#[test]
fn a_held_button_stops_hover_from_moving_focus() {
    let session = focus_session(FocusMode::Hover);
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let (first, second) = map_split_pair(session, &mut client);
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Left"));
        client.move_pointer(session, only_point_of(session, "Left"));
        client.left_button(session, true);

        // The press holds the pointer on the first window, so motion into the second does not
        // move focus. The release that follows is the barrier proving the motion was handled.
        client.inject_motion(session, only_point_of(session, "Right"));
        client.left_button(session, false);
        assert!(client.keyboard_is_on(&first), "a held button keeps focus where it is");
        assert!(
            !client.keyboard_is_on(&second),
            "hover did not move focus during the press"
        );

        // Leave through the gap, then re-enter the second window to hover-focus it.
        client.move_pointer(session, outside_output_point());
        client.move_pointer(session, only_point_of(session, "Right"));
        client.wait_until("the keyboard on the second window", |seen| seen.keyboard_is_on(&second));
    });
}

/// Waits until the loop has run every command queued before this call. Commands and input
/// share one queue, drained in order, so the reply to this query marks that point.
fn settle(session: &Handle) {
    let _ = session.control.modal_events();
}

/// The title of the window with keyboard focus, as of the last settle.
fn focused_title(session: &Handle) -> Option<String> {
    session.windows().focused().map(|window| window.title.clone())
}

#[test]
fn rest_focus_moves_to_the_window_the_pointer_rests_on() {
    let session = rest_session();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let _pair = map_split_pair(session, &mut client);
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Left"));
        client.move_pointer(session, only_point_of(session, "Right"));
        session.control.advance(POINTER_REST);
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Right"));
    });
}

#[test]
fn rest_focus_waits_the_full_rest_before_moving() {
    let session = rest_session();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let _pair = map_split_pair(session, &mut client);
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Left"));
        client.move_pointer(session, only_point_of(session, "Right"));
        session.control.advance(POINTER_REST - 1);
        settle(session);
        assert_eq!(
            focused_title(session).as_deref(),
            Some("Left"),
            "focus moved before the rest was up"
        );
        session.control.advance(1);
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Right"));
    });
}

#[test]
fn motion_restarts_the_rest_before_focus_moves() {
    let session = rest_session();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let _pair = map_split_pair(session, &mut client);
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Left"));
        let inside = only_point_of(session, "Right");
        client.move_pointer(session, inside);
        session.control.advance(POINTER_REST - 1);
        // Motion inside the same window re-arms the rest, so the old deadline no longer counts.
        client.inject_motion(session, Point::new(inside.x, inside.y + 1));
        settle(session);
        session.control.advance(POINTER_REST - 1);
        settle(session);
        assert_eq!(
            focused_title(session).as_deref(),
            Some("Left"),
            "motion did not restart the rest"
        );
        session.control.advance(1);
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Right"));
    });
}

#[test]
fn leaving_the_window_cancels_a_pending_rest_focus() {
    let session = rest_session();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let _pair = map_split_pair(session, &mut client);
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Left"));
        client.move_pointer(session, only_point_of(session, "Right"));
        client.move_pointer(session, outside_output_point());
        session.control.advance(POINTER_REST);
        settle(session);
        assert_eq!(
            focused_title(session).as_deref(),
            Some("Left"),
            "a window left behind took focus"
        );
    });
}

#[test]
fn closing_the_window_cancels_a_pending_rest_focus() {
    let session = rest_session();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let (_first, second) = map_split_pair(session, &mut client);
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Left"));
        client.move_pointer(session, only_point_of(session, "Right"));
        client.destroy(second);
        session.control.advance(POINTER_REST);
        settle(session);
        assert!(
            session.windows().titled("Right").is_none(),
            "the closed window is still observed"
        );
        assert_eq!(
            focused_title(session).as_deref(),
            Some("Left"),
            "a closed window took focus"
        );
    });
}

#[test]
fn a_held_button_ignores_a_pending_rest_focus() {
    let session = rest_session();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let _pair = map_split_pair(session, &mut client);
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Left"));
        client.move_pointer(session, only_point_of(session, "Left"));
        client.left_button(session, true);
        client.inject_motion(session, only_point_of(session, "Right"));
        session.control.advance(POINTER_REST);
        settle(session);
        assert_eq!(
            focused_title(session).as_deref(),
            Some("Left"),
            "a held button let the rest move focus"
        );
        client.left_button(session, false);
        session.control.advance(POINTER_REST);
        settle(session);
        assert_eq!(
            focused_title(session).as_deref(),
            Some("Left"),
            "the release re-armed a rest focus"
        );
    });
}

#[test]
fn a_client_round_trip_does_not_advance_the_session_clock() {
    let clock = Arc::new(ManualClock::new());
    let session = rest_session_on(clock.clone());
    session.run(move |session| {
        let mut client = Client::connect(session.socket());
        let _pair = map_split_pair(session, &mut client);
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Left"));
        client.move_pointer(session, only_point_of(session, "Right"));
        // The clock only moves when the test moves it, so the round trip must leave the rest
        // where it was.
        client.roundtrip();
        settle(session);
        assert_eq!(
            focused_title(session).as_deref(),
            Some("Left"),
            "wall time advanced the rest"
        );
        clock.advance(Duration::from_millis(POINTER_REST));
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Right"));
    });
}

/// The auto-raise delay the session's window manager uses, in milliseconds.
const AUTO_RAISE_DELAY: u64 = 500;

/// A session where the pointer focuses the window it enters, and an auto-raise follows after
/// `AUTO_RAISE_DELAY` on a clock the test moves.
fn auto_raise_session() -> Session {
    session_on(
        WmOptions {
            focus_mode: FocusMode::Hover,
            auto_raise: true,
            auto_raise_delay: AUTO_RAISE_DELAY,
            ..WmOptions::default()
        },
        Arc::new(ManualClock::new()),
    )
}

/// The title of the topmost window: the first window of the stacking order.
fn topmost(observation: &SessionSnapshot) -> Option<&str> {
    let top = observation.stacking().first()?;
    observation
        .windows()
        .find(|window| window.id == *top)
        .map(|window| window.title.as_str())
}

#[test]
fn raise_on_click_raises_the_window_pressed_in_its_exposed_part() {
    let session = session_on(WmOptions::default(), Arc::new(ManualClock::new()));
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let _pair = map_pair(session, &mut client, OVERLAP_LOWER, OVERLAP_UPPER);
        session.wait_for(|observation| topmost(observation) == Some("Right"));

        // Right covers the overlap, so a press there reaches Right until Left is raised.
        client.click(session, Point::new(250, 150));
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Right"));

        client.click(session, only_point_of(session, "Left"));
        session.wait_for(|observation| topmost(observation) == Some("Left"));
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Left"));

        // Left now lies above Right, so the same overlap point is Left's.
        client.click(session, Point::new(250, 150));
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Left"));
        assert_eq!(
            topmost(&session.windows()),
            Some("Left"),
            "the overlap press changed the stacking"
        );
    });
}

#[test]
fn raise_on_click_off_focuses_the_window_without_raising_it() {
    let session = session_on(
        WmOptions {
            raise_on_click: false,
            ..WmOptions::default()
        },
        Arc::new(ManualClock::new()),
    );
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let _pair = map_pair(session, &mut client, OVERLAP_LOWER, OVERLAP_UPPER);
        session.wait_for(|observation| topmost(observation) == Some("Right"));
        let before = session.windows().stacking().to_vec();

        client.click(session, only_point_of(session, "Left"));
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Left"));
        let after = session.windows().stacking().to_vec();
        assert_eq!(after, before, "a press without raise_on_click changed the stacking");
    });
}

#[test]
fn auto_raise_raises_the_window_the_pointer_rests_on_after_the_delay() {
    let session = auto_raise_session();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let _pair = map_pair(session, &mut client, OVERLAP_LOWER, OVERLAP_UPPER);
        session.wait_for(|observation| topmost(observation) == Some("Right"));
        // Enter Right first, so the pointer's move onto Left is a real enter that arms the raise.
        client.move_pointer(session, only_point_of(session, "Right"));
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Right"));
        client.move_pointer(session, only_point_of(session, "Left"));
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Left"));

        session.control.advance(AUTO_RAISE_DELAY - 1);
        settle(session);
        assert_eq!(
            topmost(&session.windows()),
            Some("Right"),
            "the window rose before its delay ran out"
        );
        session.control.advance(1);
        session.wait_for(|observation| topmost(observation) == Some("Left"));
    });
}

#[test]
fn auto_raise_is_cancelled_when_the_pointer_leaves_before_the_delay() {
    let session = auto_raise_session();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let _pair = map_pair(session, &mut client, OVERLAP_LOWER, OVERLAP_UPPER);
        session.wait_for(|observation| topmost(observation) == Some("Right"));
        client.move_pointer(session, only_point_of(session, "Right"));
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Right"));
        client.move_pointer(session, only_point_of(session, "Left"));
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Left"));

        session.control.advance(AUTO_RAISE_DELAY - 1);
        client.move_pointer(session, only_point_of(session, "Right"));
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Right"));
        session.control.advance(1);
        settle(session);
        assert_eq!(
            topmost(&session.windows()),
            Some("Right"),
            "a window the pointer left was raised after the delay"
        );
    });
}

#[test]
fn auto_raise_does_nothing_once_its_window_is_closed() {
    let session = auto_raise_session();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let (lower, _upper) = map_pair(session, &mut client, OVERLAP_LOWER, OVERLAP_UPPER);
        session.wait_for(|observation| topmost(observation) == Some("Right"));
        client.move_pointer(session, only_point_of(session, "Left"));
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Left"));

        client.destroy(lower);
        session.wait_for(|observation| observation.titled("Left").is_none());
        session.control.advance(AUTO_RAISE_DELAY);
        settle(session);
        assert_eq!(
            topmost(&session.windows()),
            Some("Right"),
            "the closed window's auto-raise changed the stacking"
        );
    });
}

#[test]
fn auto_raise_ignores_the_pointer_while_a_button_holds_its_grab() {
    let session = auto_raise_session();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let _lower = place_window(session, &mut client, "org.gnoblin.Left", "Left", OVERLAP_LOWER);
        let _upper = place_window(session, &mut client, "org.gnoblin.Right", "Right", OVERLAP_UPPER);
        session.wait_for(|observation| topmost(observation) == Some("Right"));
        client.move_pointer(session, only_point_of(session, "Right"));
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Right"));

        // The press holds a grab on Right, so the pointer crossing into Left must not arm a raise.
        client.left_button(session, true);
        client.inject_motion(session, only_point_of(session, "Left"));
        session.control.advance(AUTO_RAISE_DELAY);
        settle(session);
        assert_eq!(
            topmost(&session.windows()),
            Some("Right"),
            "a pointer grab let an auto-raise reorder the stacking"
        );
        client.left_button(session, false);
    });
}
