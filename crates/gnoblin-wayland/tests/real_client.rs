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

use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::InputEvent;
use gnoblin_backend::headless::Headless;
use gnoblin_types::Point;
use gnoblin_wayland::{Config, ControlHandle, LoopError, ObservedWindow, Server, SessionSnapshot};
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
        let directory = tempfile::tempdir().expect("a temporary directory");
        let socket = directory.path().join("wayland-0.sock");
        let config = Config {
            socket_path: Some(socket.clone()),
            ..Config::default()
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
    /// Whether the server asked the window to close.
    closed: bool,
    /// How many times the keyboard entered this client's surface.
    keyboard_enters: u32,
    /// How many times the pointer entered this client's surface.
    pointer_enters: u32,
    /// How many pings the server sent.
    pings: u32,
    /// How many popup configures arrived.
    popup_configures: u32,
    /// Whether the server dismissed a popup.
    popup_done: bool,
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
            .bind(&qh, 1..=1, ())
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
}

/// The centre of a window, in the output's logical space.
fn centre(window: &ObservedWindow) -> Point {
    Point::new(
        window.geometry.x + (window.geometry.width / 2) as i32,
        window.geometry.y + (window.geometry.height / 2) as i32,
    )
}

/// The logical size of the default headless output.
fn headless_screen() -> (i32, i32) {
    (
        gnoblin_backend::headless::DEFAULT_MODE.width as i32,
        gnoblin_backend::headless::DEFAULT_MODE.height as i32,
    )
}

#[test]
fn a_real_client_maps_a_window_and_gets_focus() {
    let session = Session::start(Headless::new());
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
fn a_second_client_takes_focus_and_a_click_switches_it_back() {
    let session = Session::start(Headless::new());
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
        let alpha = centre(observation.titled("Alpha").expect("the first window"));
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
    let session = Session::start(Headless::new());
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
    let session = Session::start(Headless::new());
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
    let session = Session::start(Headless::new());
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
        if let wl_keyboard::Event::Enter { .. } = event {
            state.keyboard_enters += 1;
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
        if let wl_pointer::Event::Enter { .. } = event {
            state.pointer_enters += 1;
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
            state.pings += 1;
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
            xdg_toplevel::Event::Configure { width, height, .. } => {
                state.toplevel_configures += 1;
                state.width = width;
                state.height = height;
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
