//! The real-client lock session: an `ext-session-lock-v1` client against the server.
//!
//! Every test starts a session on a socket of its own, connects real `wayland-client`
//! connections, and asserts on what each connection and the published observation saw.
//! The backend's [`gnoblin_backend::headless::HeadlessHandle`] is what makes the two
//! timing cases reachable: a test can hotplug an output and, more importantly, hold
//! the presents, which is the only way to prove that a session does not report itself
//! locked before a covering frame reached every output.
//!
//! The tests are the crate's failure-mode list for this protocol, in the order
//! `crates/gnoblin-wayland/src/protocols/ext_session_lock_v1.rs` states them.

use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::InputEvent;
use gnoblin_backend::headless::{Headless, HeadlessHandle};
use gnoblin_lock::State as LockState;
use gnoblin_types::Point;
use gnoblin_wayland::{Config, ControlHandle, LoopError, Server, SessionSnapshot};
use wayland_client::backend::ObjectId;
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_surface_v1, ext_session_lock_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// How long a test waits for the session to reach a state before failing.
const TIMEOUT: Duration = Duration::from_secs(10);

/// `BTN_LEFT` from `linux/input-event-codes.h`.
const BTN_LEFT: u32 = 0x110;

/// `KEY_A` from `linux/input-event-codes.h`.
const KEY_A: u32 = 30;

/// What a worker thread can reach of a running session.
#[derive(Clone)]
struct Handle {
    socket: PathBuf,
    control: ControlHandle,
    observation: Arc<Mutex<SessionSnapshot>>,
    backend: HeadlessHandle,
}

impl Handle {
    fn socket(&self) -> &Path {
        &self.socket
    }

    /// The window and lock state right now.
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

    /// Waits until the lock is in `state`.
    fn wait_for_lock(&self, state: LockState) -> SessionSnapshot {
        self.wait_for(|observation| observation.lock_state() == state)
    }

    /// Queues a key press and release, as the backend would deliver them.
    fn press_key(&self) {
        self.control.inject_input(InputEvent::Key {
            keycode: KEY_A,
            pressed: true,
        });
        self.control.inject_input(InputEvent::Key {
            keycode: KEY_A,
            pressed: false,
        });
    }

    /// Queues a pointer motion and a click at a global position.
    fn click(&self, at: Point) {
        let output = self.snapshot().outputs().first().copied().expect("an output");
        self.control
            .inject_input(InputEvent::PointerMotion { output, position: at });
        self.control.inject_input(InputEvent::PointerButton {
            button: BTN_LEFT,
            pressed: true,
        });
        self.control.inject_input(InputEvent::PointerButton {
            button: BTN_LEFT,
            pressed: false,
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
    /// Creates a session on a socket of its own, with a handle to its backend.
    fn start() -> Self {
        let backend = Headless::new();
        let backend_handle = backend.handle();
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

/// What a test's client records from the events it receives.
#[derive(Default)]
struct App {
    /// How many `xdg_surface.configure` events arrived, and how many were acked.
    configures: u32,
    acks: u32,
    /// How many `xdg_toplevel.configure` events arrived.
    toplevel_configures: u32,
    /// How many times the keyboard entered, left and delivered a key.
    keyboard_enters: u32,
    keyboard_leaves: u32,
    keys: u32,
    /// How many times the pointer entered and moved, and where it last was.
    pointer_enters: u32,
    pointer_motions: u32,
    pointer_x: f64,
    pointer_y: f64,
    pointer_buttons: u32,
    /// The lock objects that received `locked`, in arrival order.
    locked: Vec<ObjectId>,
    /// The lock objects that received `finished`, in arrival order.
    finished: Vec<ObjectId>,
    /// How many lock-surface configures arrived, and the last one's values.
    lock_configures: u32,
    lock_acks: u32,
    lock_width: i32,
    lock_height: i32,
}

/// A real Wayland client for one connection.
struct Client {
    /// The client owns its connection so the socket closes when it is dropped, which
    /// is what a locker crash is.
    #[allow(dead_code)]
    connection: Connection,
    queue: EventQueue<App>,
    state: App,
    qh: QueueHandle<App>,
    globals: GlobalList,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    #[allow(dead_code)]
    seat: wl_seat::WlSeat,
    outputs: Vec<wl_output::WlOutput>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    lock_manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    /// Every lock object this client created, in request order. A client may ask more
    /// than once, and only the newest request is the one a refusal answers.
    locks: Vec<ext_session_lock_v1::ExtSessionLockV1>,
}

/// The objects of one toplevel.
///
/// The proxies are held so their events keep arriving; dropping one would only stop
/// the events, so a test that never destroys its window keeps them.
struct Window {
    surface: wl_surface::WlSurface,
    #[allow(dead_code)]
    xdg_surface: xdg_surface::XdgSurface,
    #[allow(dead_code)]
    toplevel: xdg_toplevel::XdgToplevel,
}

/// The objects of one lock surface.
struct LockSurface {
    #[allow(dead_code)]
    surface: wl_surface::WlSurface,
    #[allow(dead_code)]
    lock_surface: ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
}

impl Client {
    /// Connects and binds the globals an ordinary application uses.
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

    /// Connects and binds the session lock manager.
    fn locker(socket: &Path) -> Self {
        let mut client = Self::connect(socket);
        client.lock_manager = Some(
            client
                .globals
                .bind(&client.qh, 1..=1, ())
                .expect("the session advertises ext_session_lock_manager_v1"),
        );
        client
    }

    /// Connects and binds the globals both clients need.
    fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("the session is listening");
        let connection = Connection::from_socket(stream).expect("a client connection");
        let (globals, queue) = registry_queue_init::<App>(&connection).expect("the global list");
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor = globals
            .bind(&qh, 1..=1, ())
            .expect("the session advertises wl_compositor");
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).expect("the session advertises wl_shm");
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).expect("the session advertises wl_seat");
        let mut client = Self {
            connection,
            queue,
            state: App::default(),
            qh,
            globals,
            compositor,
            shm,
            seat,
            outputs: Vec::new(),
            wm_base: None,
            lock_manager: None,
            locks: Vec::new(),
        };
        client.outputs = client.bind_outputs();
        client
    }

    /// Processes every event the server has sent.
    fn roundtrip(&mut self) {
        self.queue.roundtrip(&mut self.state).expect("a round trip");
    }

    /// Every `wl_output` the session advertises right now.
    fn bind_outputs(&self) -> Vec<wl_output::WlOutput> {
        self.globals
            .contents()
            .clone_list()
            .iter()
            .filter(|global| global.interface == "wl_output")
            .map(|global| {
                self.globals.registry().bind::<wl_output::WlOutput, _, _>(
                    global.name,
                    global.version.min(4),
                    &self.qh,
                    (),
                )
            })
            .collect()
    }

    /// Creates a toplevel and commits it once, which is what asks for a configure.
    fn create_window(&mut self, app_id: &str, title: &str) -> Window {
        let wm_base = self.wm_base.as_ref().expect("an application client");
        let surface = self.compositor.create_surface(&self.qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, &self.qh, ());
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

    /// Attaches a shared-memory buffer and commits, which maps the surface.
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
    fn map_window(&mut self, app_id: &str, title: &str) -> Window {
        let window = self.create_window(app_id, title);
        self.attach_buffer(&window.surface, 200, 150);
        window
    }

    /// Sends `ext_session_lock_manager_v1.lock` and flushes it to the server.
    fn lock(&mut self) -> ObjectId {
        let manager = self.lock_manager.as_ref().expect("a locker client");
        let lock = manager.lock(&self.qh, ());
        let id = lock.id();
        self.locks.push(lock);
        self.roundtrip();
        id
    }

    /// Creates one lock surface for an output and commits a buffer to it.
    ///
    /// The order is the protocol's: create the surface, receive a configure, ack it,
    /// then commit a buffer of exactly the configured size.
    fn lock_surface(&mut self, index: usize) -> LockSurface {
        let output = self.outputs.get(index).expect("an output").clone();
        let lock = self.locks.last().expect("a lock request");
        let surface = self.compositor.create_surface(&self.qh, ());
        let lock_surface = lock.get_lock_surface(&surface, &output, &self.qh, ());
        self.roundtrip();
        assert!(self.state.lock_configures >= 1, "the lock surface was configured");
        let (width, height) = (self.state.lock_width, self.state.lock_height);
        assert!(width > 0 && height > 0, "the configure carries the output size");
        self.attach_buffer(&surface, width, height);
        LockSurface { surface, lock_surface }
    }

    /// Destroys the newest lock object without unlocking, which is a locker that gives
    /// up before `locked`.
    fn abandon(&mut self) {
        let lock = self.locks.pop().expect("a lock request");
        lock.destroy();
        self.roundtrip();
    }

    /// Sends `unlock_and_destroy`, which only the owner may do after `locked`.
    fn unlock(&mut self) {
        let lock = self.locks.pop().expect("a lock");
        lock.unlock_and_destroy();
        self.roundtrip();
    }

    /// Waits until this client received `locked`, with a fresh round trip per try.
    fn wait_for_locked(&mut self) {
        let deadline = Instant::now() + TIMEOUT;
        while self.state.locked.is_empty() {
            assert!(Instant::now() < deadline, "the locker never received `locked`");
            self.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Waits until this client received `finished`.
    fn wait_for_finished(&mut self) {
        let deadline = Instant::now() + TIMEOUT;
        while self.state.finished.is_empty() {
            assert!(Instant::now() < deadline, "the locker never received `finished`");
            self.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Reads every event that has arrived, then reports the counters.
    fn settle(&mut self) -> &App {
        self.roundtrip();
        self.roundtrip();
        &self.state
    }
}

/// The centre of a window, in the output's logical space.
fn centre(geometry: gnoblin_types::Rect) -> Point {
    Point::new(
        geometry.x + (geometry.width / 2) as i32,
        geometry.y + (geometry.height / 2) as i32,
    )
}

#[test]
fn locked_waits_for_a_presented_cover() {
    let session = Session::start();
    session.run(|session| {
        // The backend presents nothing until the test says so, so no frame can prove
        // the cover.
        session.backend.hold_presents(true);
        let mut locker = Client::locker(session.socket());
        locker.lock();
        let observation = session.wait_for_lock(LockState::Locking);
        assert!(observation.lock_active(), "the session is covered while it waits");
        assert!(
            !observation.capture_allowed(),
            "capture is embargoed during the transition"
        );
        assert!(locker.settle().locked.is_empty(), "`locked` is not sent before a frame");

        session.backend.hold_presents(false);
        let observation = session.wait_for_lock(LockState::Locked);
        assert!(observation.lock_active());
        locker.wait_for_locked();
        assert_eq!(locker.state.locked.len(), 1, "one `locked` per owner");
        assert!(locker.state.finished.is_empty(), "the request was granted");
    });
}

#[test]
fn a_second_locker_is_refused_with_finished() {
    let session = Session::start();
    session.run(|session| {
        let mut owner = Client::locker(session.socket());
        owner.lock();
        owner.wait_for_locked();

        let mut rival = Client::locker(session.socket());
        rival.lock();
        rival.wait_for_finished();
        assert!(
            rival.state.locked.is_empty(),
            "a refused locker is never told it locked"
        );
        assert_eq!(owner.state.locked.len(), 1, "the owner heard `locked` once");

        let observation = session.wait_for_lock(LockState::Locked);
        assert!(observation.lock_active());
    });
}

#[test]
fn a_second_request_from_the_same_client_is_refused_alone() {
    let session = Session::start();
    session.run(|session| {
        // The first request is still waiting for its cover when the same client asks
        // again, which is what a locker that retries does.
        session.backend.hold_presents(true);
        let mut locker = Client::locker(session.socket());
        let first = locker.lock();
        let second = locker.lock();
        assert_eq!(locker.settle().finished, vec![second], "the second request was refused");

        session.backend.hold_presents(false);
        locker.wait_for_locked();
        assert_eq!(
            locker.state.locked,
            vec![first],
            "the request that already owned the lock was not refused with it"
        );
        assert_eq!(session.wait_for_lock(LockState::Locked).lock_state(), LockState::Locked);
    });
}

#[test]
fn a_covered_client_receives_no_input() {
    let session = Session::start();
    session.run(|session| {
        let mut app = Client::app(session.socket());
        let _window = app.map_window("org.gnoblin.Covered", "Covered");
        let observation =
            session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped && window.focused));
        let geometry = observation.only().expect("the window").geometry;
        assert_eq!(app.state.keyboard_enters, 1, "the mapped window took focus");

        let mut locker = Client::locker(session.socket());
        locker.lock();
        locker.wait_for_locked();
        let _surface = locker.lock_surface(0);
        // The lock surface is now the only destination input may reach.
        let deadline = Instant::now() + TIMEOUT;
        while locker.state.keyboard_enters == 0 {
            assert!(Instant::now() < deadline, "the lock surface never took focus");
            locker.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }
        app.roundtrip();
        assert_eq!(app.state.keyboard_leaves, 1, "the covered window lost focus");
        assert_eq!(app.state.keyboard_enters, 1, "and did not get it back");

        session.press_key();
        session.click(centre(geometry));

        let app_state = app.settle();
        assert_eq!(app_state.keys, 0, "no key reached the covered client");
        assert_eq!(app_state.pointer_buttons, 0, "no button reached the covered client");
        assert_eq!(app_state.pointer_motions, 0, "no motion reached the covered client");

        let locker_state = locker.settle();
        assert!(locker_state.keys >= 1, "the lock surface received the key");
        assert!(
            locker_state.pointer_enters >= 1,
            "the lock surface received the pointer"
        );
        // The pointer's second element is the surface's origin, so the client sees the
        // position inside its own surface. The only output is at the origin, so a
        // corner-valued origin would show up as (0, 0) here.
        assert_eq!(
            (locker_state.pointer_x, locker_state.pointer_y),
            (f64::from(centre(geometry).x), f64::from(centre(geometry).y)),
            "the lock surface saw the position inside itself, not the corner"
        );

        let observation = session.wait_for(|observation| observation.lock_state() == LockState::Locked);
        assert!(
            observation.focused().is_none(),
            "no window holds keyboard focus while the session is covered"
        );

        // The cover hid the window, it did not close it.
        assert_eq!(observation.len(), 1, "the covered window is still managed");
    });
}

#[test]
fn an_unmapped_lock_surface_receives_no_input() {
    let session = Session::start();
    session.run(|session| {
        let mut app = Client::app(session.socket());
        let _window = app.map_window("org.gnoblin.Covered", "Covered");
        session.wait_for(|observation| observation.only().is_some_and(|window| window.focused));

        let mut locker = Client::locker(session.socket());
        locker.lock();
        locker.wait_for_locked();
        // No lock surface: the owner exists, but it has nothing mapped to receive a
        // key, so the gate consumes the event instead of falling through.
        session.press_key();
        assert_eq!(app.settle().keys, 0, "the covered client never sees the key");
        assert_eq!(locker.settle().keys, 0, "nor does a locker with no lock surface");
    });
}

#[test]
fn a_locker_crash_keeps_the_cover() {
    let session = Session::start();
    session.run(|session| {
        let mut app = Client::app(session.socket());
        let _window = app.map_window("org.gnoblin.Covered", "Covered");
        session.wait_for(|observation| observation.only().is_some_and(|window| window.focused));

        let mut locker = Client::locker(session.socket());
        locker.lock();
        locker.wait_for_locked();
        let _surface = locker.lock_surface(0);
        // The connection goes away without an unlock, which is a crashed locker.
        drop(locker);

        let observation = session.wait_for_lock(LockState::CoverFailed);
        assert!(observation.lock_active(), "the cover stays after the locker dies");
        assert!(!observation.capture_allowed(), "the embargo stays");

        session.press_key();
        assert_eq!(app.settle().keys, 0, "the covered client still receives nothing");
        assert_eq!(
            session
                .wait_for(|observation| observation.only().is_some_and(|window| !window.focused))
                .focused()
                .map(|window| window.id),
            None,
            "nothing took focus back"
        );
    });
}

#[test]
fn a_locker_crash_before_locked_keeps_the_cover() {
    let session = Session::start();
    session.run(|session| {
        session.backend.hold_presents(true);
        let mut app = Client::app(session.socket());
        let _window = app.map_window("org.gnoblin.Covered", "Covered");
        session.wait_for(|observation| observation.only().is_some_and(|window| window.focused));

        let mut locker = Client::locker(session.socket());
        locker.lock();
        session.wait_for_lock(LockState::Locking);
        drop(locker);

        let observation = session.wait_for_lock(LockState::CoverFailed);
        assert!(observation.lock_active(), "the cover stays when the locker goes");
        session.backend.hold_presents(false);
        // A frame cannot prove a cover that is not armed, so the state stays put.
        thread::sleep(Duration::from_millis(50));
        assert_eq!(session.snapshot().lock_state(), LockState::CoverFailed);
        session.press_key();
        assert_eq!(app.settle().keys, 0, "the covered client receives nothing");
    });
}

#[test]
fn abandoning_before_locked_keeps_the_cover() {
    let session = Session::start();
    session.run(|session| {
        session.backend.hold_presents(true);
        let mut app = Client::app(session.socket());
        let _window = app.map_window("org.gnoblin.Covered", "Covered");
        session.wait_for(|observation| observation.only().is_some_and(|window| window.focused));

        let mut locker = Client::locker(session.socket());
        locker.lock();
        session.wait_for_lock(LockState::Locking);
        // A destroy request, not a disconnect: the connection stays open and the lock
        // object is gone all the same.
        locker.abandon();

        let observation = session.wait_for_lock(LockState::CoverFailed);
        assert!(observation.lock_active(), "abandoning is not an unlock");
        session.press_key();
        assert_eq!(app.settle().keys, 0, "the covered client receives nothing");

        // A replacement may take over, but only once its own cover is presented.
        let mut replacement = Client::locker(session.socket());
        replacement.lock();
        assert!(
            replacement.settle().locked.is_empty(),
            "a takeover is not granted on the abandoned request's proof"
        );
        session.backend.hold_presents(false);
        replacement.wait_for_locked();
        assert_eq!(replacement.state.locked.len(), 1);
    });
}

#[test]
fn a_takeover_waits_for_a_fresh_cover() {
    let session = Session::start();
    session.run(|session| {
        let mut first = Client::locker(session.socket());
        first.lock();
        first.wait_for_locked();
        drop(first);
        session.wait_for_lock(LockState::CoverFailed);

        // A replacement may only take over the failsafe cover by presenting its own,
        // so with the presents held it hears nothing.
        session.backend.hold_presents(true);
        let mut second = Client::locker(session.socket());
        second.lock();
        session.wait_for_lock(LockState::Locking);
        assert!(
            second.settle().locked.is_empty(),
            "a takeover is not granted on the old proof"
        );

        session.backend.hold_presents(false);
        session.wait_for_lock(LockState::Locked);
        let deadline = Instant::now() + TIMEOUT;
        while second.state.locked.is_empty() {
            assert!(Instant::now() < deadline, "the replacement never received `locked`");
            second.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(second.state.locked.len(), 1);
    });
}

#[test]
fn removing_every_output_keeps_the_cover() {
    let session = Session::start();
    session.run(|session| {
        let mut app = Client::app(session.socket());
        let _window = app.map_window("org.gnoblin.Covered", "Covered");
        session.wait_for(|observation| observation.only().is_some_and(|window| window.focused));

        let mut locker = Client::locker(session.socket());
        locker.lock();
        locker.wait_for_locked();

        let output = session.snapshot().outputs().first().copied().expect("an output");
        session.backend.disconnect_output(output);
        let observation = session.wait_for(|observation| observation.outputs().is_empty());
        assert_eq!(
            observation.lock_state(),
            LockState::CoverFailed,
            "with no output to present on the lock fails closed"
        );
        assert!(observation.lock_active(), "and keeps the embargo");

        session.press_key();
        assert_eq!(app.settle().keys, 0, "a covered client receives nothing");
    });
}

#[test]
fn a_hotplugged_output_is_covered_before_it_shows_anything() {
    let session = Session::start();
    session.run(|session| {
        let mut app = Client::app(session.socket());
        let _window = app.map_window("org.gnoblin.Covered", "Covered");
        session.wait_for(|observation| observation.only().is_some_and(|window| window.focused));

        let mut locker = Client::locker(session.socket());
        locker.lock();
        locker.wait_for_locked();
        let _surface = locker.lock_surface(0);

        // The new output has to present the cover before the lock is proven again.
        session.backend.hold_presents(true);
        session
            .backend
            .connect_output(
                "HEADLESS-2",
                gnoblin_backend::headless::DEFAULT_MODE,
                gnoblin_types::Scale::integral(1),
            )
            .expect("a mode with pixels");
        let observation = session.wait_for(|observation| observation.outputs().len() == 2);
        assert_eq!(
            observation.lock_state(),
            LockState::Locking,
            "the barrier restarted for the new output"
        );
        assert!(observation.lock_active(), "the session stays covered");

        session.press_key();
        assert_eq!(app.settle().keys, 0, "the covered client receives nothing");

        session.backend.hold_presents(false);
        let observation = session.wait_for_lock(LockState::Locked);
        assert!(observation.lock_active());
        let deadline = Instant::now() + TIMEOUT;
        while locker.state.locked.is_empty() {
            assert!(Instant::now() < deadline, "the lock was never proven again");
            locker.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            locker.state.locked.len(),
            1,
            "hotplug re-proves the cover without a second `locked`"
        );

        // The locker can now cover the new output as well: the registry event for it
        // arrives on a round trip, like any other global.
        locker.roundtrip();
        locker.outputs = locker.bind_outputs();
        assert_eq!(locker.outputs.len(), 2, "the session advertises the new output");
        let _second = locker.lock_surface(1);
        assert_eq!(locker.state.lock_configures, 2, "each lock surface was configured");
    });
}

#[test]
fn unlock_restores_the_previous_focus() {
    let session = Session::start();
    session.run(|session| {
        let mut app = Client::app(session.socket());
        let _window = app.map_window("org.gnoblin.Covered", "Covered");
        let observation =
            session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped && window.focused));
        let focused = observation.focus().expect("a focused window");
        assert_eq!(app.state.keyboard_enters, 1);

        let mut locker = Client::locker(session.socket());
        locker.lock();
        locker.wait_for_locked();
        let _surface = locker.lock_surface(0);
        app.roundtrip();
        assert_eq!(app.state.keyboard_leaves, 1, "the covered window lost focus");

        locker.unlock();
        let observation = session.wait_for_lock(LockState::Unlocked);
        assert!(!observation.lock_active(), "the session is not covered any more");
        assert!(observation.capture_allowed(), "capture is released with the cover");
        assert_eq!(observation.focus(), Some(focused), "the window manager's focus is back");

        let deadline = Instant::now() + TIMEOUT;
        while app.state.keyboard_enters < 2 {
            assert!(Instant::now() < deadline, "the previous window never got focus back");
            app.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }

        // The lock really is free: a new locker is granted after the reveal.
        let mut next = Client::locker(session.socket());
        next.lock();
        session.wait_for_lock(LockState::Locked);
        next.wait_for_locked();
        assert_eq!(next.state.locked.len(), 1);
    });
}

// The interfaces this client binds but only ever listens to.

wayland_client::delegate_noop!(App: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(App: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(App: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(App: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(App: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(App: ignore wl_output::WlOutput);

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
            wl_keyboard::Event::Enter { .. } => state.keyboard_enters += 1,
            wl_keyboard::Event::Leave { .. } => state.keyboard_leaves += 1,
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
                // The enter carries the first position, and it is the one a focus change
                // produces: a `motion` only follows a later move.
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
            state.configures = state.configures.saturating_add(1);
            xdg_surface.ack_configure(serial);
            state.acks = state.acks.saturating_add(1);
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
        if let xdg_toplevel::Event::Configure { .. } = event {
            state.toplevel_configures = state.toplevel_configures.saturating_add(1);
        }
    }
}

wayland_client::delegate_noop!(App: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1);

impl Dispatch<ext_session_lock_v1::ExtSessionLockV1, ()> for App {
    fn event(
        state: &mut Self,
        lock: &ext_session_lock_v1::ExtSessionLockV1,
        event: <ext_session_lock_v1::ExtSessionLockV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_session_lock_v1::Event::Locked => state.locked.push(lock.id()),
            ext_session_lock_v1::Event::Finished => state.finished.push(lock.id()),
            _ => {}
        }
    }
}

impl Dispatch<ext_session_lock_surface_v1::ExtSessionLockSurfaceV1, ()> for App {
    fn event(
        state: &mut Self,
        lock_surface: &ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
        event: <ext_session_lock_surface_v1::ExtSessionLockSurfaceV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure { serial, width, height } = event {
            state.lock_configures = state.lock_configures.saturating_add(1);
            state.lock_width = width as i32;
            state.lock_height = height as i32;
            // Acking before the first buffer commit is what the protocol requires, and
            // the serial has to be the one this configure carried.
            lock_surface.ack_configure(serial);
            state.lock_acks = state.lock_acks.saturating_add(1);
        }
    }
}
