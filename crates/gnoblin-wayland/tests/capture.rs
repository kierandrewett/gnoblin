//! The real-client capture session: both capture protocols against the server.
//!
//! Every test starts a session on a socket of its own and connects real
//! `wayland-client` connections: an application that maps a window, a capture client
//! that asks for frames, and — for the lock cases — a locker. Nothing here needs a
//! display, a device or a compositor from the environment.
//!
//! The tests are the failure-mode list at the top of
//! `crates/gnoblin-wayland/src/protocols/capture_shared.rs`, one to an item.
//!
//! # Why the client reads its own buffer back
//!
//! A capture writes into a `wl_shm` pool the client owns, so the test reads the pool
//! back through the file it created. That is the only assertion in this workspace that
//! proves a capture produced *those pixels at that place* rather than merely that the
//! protocol finished, which is why every pixel test crops the region it cares about
//! instead of trusting the protocol's `ready`.

use std::fs::File;
use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_config::Protocols;
use gnoblin_lock::State as LockState;
use gnoblin_wayland::{Config, ControlHandle, LoopError, Server, SessionSnapshot};
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1, ext_foreign_toplevel_list_v1,
};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_foreign_toplevel_image_capture_source_manager_v1, ext_output_image_capture_source_manager_v1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1, ext_image_copy_capture_manager_v1, ext_image_copy_capture_session_v1,
};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_surface_v1, ext_session_lock_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::screencopy::v1::client::{zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1};

/// How long a test waits for the session to reach a state before failing.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The window's pixels: opaque red, in the byte order `wl_shm` stores `argb8888` in.
const RED: u32 = 0xFFFF_0000;

/// The colour the compositor's background has: opaque black.
const BLACK: u32 = 0xFF00_0000;

/// What a worker thread can reach of a running session.
#[derive(Clone)]
struct Handle {
    socket: PathBuf,
    /// The socket the session's own processes connect on. A connection there is
    /// privileged and may bind a capture global; an application's may not.
    privileged_socket: PathBuf,
    control: ControlHandle,
    observation: Arc<Mutex<SessionSnapshot>>,
}

impl Handle {
    fn socket(&self) -> &Path {
        &self.socket
    }

    fn privileged_socket(&self) -> &Path {
        &self.privileged_socket
    }

    /// The session state right now.
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

    /// The geometry of the only window, once it is mapped.
    fn only_window(&self) -> gnoblin_types::Rect {
        let observation = self.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        observation.only().expect("one mapped window").geometry
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
    /// Creates a session on sockets of its own.
    fn start() -> Self {
        Self::start_with(Protocols::default())
    }

    /// Creates a session with an explicit protocol configuration.
    fn start_with(protocols: Protocols) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let socket = directory.path().join("wayland-0.sock");
        let privileged_socket = directory.path().join("wayland-0-privileged.sock");
        let config = Config {
            socket_path: Some(socket.clone()),
            privileged_socket_path: Some(privileged_socket.clone()),
            protocols,
            ..Config::default()
        };
        let server =
            Server::new(Box::new(gnoblin_backend::headless::Headless::new()), config).expect("the session starts");
        let handle = Handle {
            socket,
            privileged_socket,
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

/// One toplevel a foreign-toplevel list named.
#[derive(Debug, Clone)]
struct Toplevel {
    /// The handle a capture source is created from.
    handle: ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
    /// The identifier the compositor assigned it, which the protocol requires to be
    /// stable per toplevel.
    #[allow(dead_code)]
    identifier: String,
    /// Its title.
    title: String,
    /// Its application id.
    app_id: String,
}

/// One damaged rectangle a frame reported.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Damage {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

/// What a test's clients recorded from the events they received.
#[derive(Debug, Default)]
struct App {
    /// The lock objects that heard `locked` and `finished`.
    locked: usize,
    finished: usize,
    /// Lock-surface configures and the last one's size.
    lock_configures: usize,
    lock_width: i32,
    lock_height: i32,
    /// `xdg_surface` configures and acknowledgements, for mapping a window.
    configures: usize,
    acks: usize,
    toplevel_configures: usize,
    /// The screencopy frame's advertised buffer geometry.
    screencopy_buffer: Option<(u32, u32, u32, u32)>,
    screencopy_buffer_done: bool,
    /// The damage rectangles a screencopy frame reported.
    screencopy_damage: Vec<Damage>,
    screencopy_ready: usize,
    screencopy_failed: usize,
    /// The image-copy session's constraints.
    session_buffer_size: Option<(u32, u32)>,
    session_formats: Vec<u32>,
    session_done: usize,
    session_stopped: usize,
    /// The image-copy frame's outcome and damage.
    frame_ready: usize,
    frame_failed: usize,
    frame_damage: Vec<Damage>,
    /// The foreign toplevels this client was told about.
    toplevels: Vec<Toplevel>,
    toplevels_closed: usize,
    /// The frame callbacks that fired, per surface id.
    frame_callbacks: usize,
    released_buffers: usize,
}

/// A shared-memory buffer a test can fill before the compositor reads it and read after
/// a capture wrote into it.
struct Shm {
    file: File,
    buffer: wl_buffer::WlBuffer,
    width: u32,
    height: u32,
    stride: u32,
}

impl Shm {
    /// A buffer of `width` by `height` pixels, filled with `colour`.
    fn new(client: &Client, width: u32, height: u32, colour: u32) -> Self {
        let stride = width * 4;
        let length = (stride * height) as usize;
        let mut file = tempfile::tempfile().expect("a shared memory file");
        file.set_len(length as u64).expect("a file that big");
        let mut bytes = vec![0u8; length];
        for pixel in bytes.as_chunks_mut::<4>().0 {
            *pixel = colour.to_ne_bytes();
        }
        file.write_all(&bytes).expect("the pixels");
        let pool = client.shm.create_pool(file.as_fd(), length as i32, &client.qh, ());
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride as i32,
            wl_shm::Format::Argb8888,
            &client.qh,
            (),
        );
        Self {
            file,
            buffer,
            width,
            height,
            stride,
        }
    }

    /// One pixel of the buffer, as a native-endian `argb8888` word.
    fn pixel(&self, x: u32, y: u32) -> u32 {
        let mut bytes = [0u8; 4];
        let offset = u64::from(y) * u64::from(self.stride) + u64::from(x) * 4;
        self.file.read_exact_at(&mut bytes, offset).expect("a pixel");
        u32::from_ne_bytes(bytes)
    }

    /// Every pixel is the same colour, which is what "the buffer was never written"
    /// looks like.
    fn is_uniform(&self, colour: u32) -> bool {
        (0..self.height).all(|y| (0..self.width).all(|x| self.pixel(x, y) == colour))
    }
}

/// One managed toplevel, as the client created it.
struct Window {
    surface: wl_surface::WlSurface,
    xdg_surface: xdg_surface::XdgSurface,
    toplevel: xdg_toplevel::XdgToplevel,
    /// The buffer is kept so the client's pool outlives the window.
    #[allow(dead_code)]
    shm: Shm,
}

/// The objects of one lock surface.
struct LockSurface {
    #[allow(dead_code)]
    surface: wl_surface::WlSurface,
    #[allow(dead_code)]
    lock_surface: ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
    /// Kept so the client's pool outlives the surface's use of it.
    #[allow(dead_code)]
    shm: Shm,
}

/// A real Wayland client for one connection.
struct Client {
    /// The client owns its connection so the socket closes when the client is dropped,
    /// which is how a test makes a capture client disconnect.
    #[allow(dead_code)]
    connection: Connection,
    queue: EventQueue<App>,
    state: App,
    qh: QueueHandle<App>,
    globals: GlobalList,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    wm_base: xdg_wm_base::XdgWmBase,
    outputs: Vec<wl_output::WlOutput>,
    lock_manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    screens: Option<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1>,
    image_copy: Option<ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1>,
    output_source: Option<ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1>,
    toplevel_source:
        Option<ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1>,
    toplevel_list: Option<ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1>,
    /// Every lock object this client created.
    locks: Vec<ext_session_lock_v1::ExtSessionLockV1>,
}

impl Client {
    /// Connects and binds the globals an ordinary application uses.
    fn app(socket: &Path) -> Self {
        Self::connect(socket)
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

    /// Connects and binds every capture global.
    fn capturer(socket: &Path) -> Self {
        let mut client = Self::connect(socket);
        client.screens = Some(
            client
                .globals
                .bind(&client.qh, 1..=3, ())
                .expect("the session advertises zwlr_screencopy_manager_v1"),
        );
        client.image_copy = Some(
            client
                .globals
                .bind(&client.qh, 1..=1, ())
                .expect("the session advertises ext_image_copy_capture_manager_v1"),
        );
        client.output_source = Some(
            client
                .globals
                .bind(&client.qh, 1..=1, ())
                .expect("the session advertises ext_output_image_capture_source_manager_v1"),
        );
        let toplevel_source: ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1 =
            client
                .globals
                .bind(&client.qh, 1..=1, ())
                .expect("the session advertises ext_foreign_toplevel_image_capture_source_manager_v1");
        client.toplevel_source = Some(toplevel_source);
        client.toplevel_list = Some(
            client
                .globals
                .bind(&client.qh, 1..=1, ())
                .expect("the session advertises ext_foreign_toplevel_list_v1"),
        );
        client
    }

    /// Connects and binds the globals every client needs.
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
        let mut client = Self {
            connection,
            queue,
            state: App::default(),
            qh,
            globals,
            compositor,
            shm,
            wm_base,
            outputs: Vec::new(),
            lock_manager: None,
            screens: None,
            image_copy: None,
            output_source: None,
            toplevel_source: None,
            toplevel_list: None,
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

    /// The name of every global the session advertised to this connection.
    fn advertised(&self) -> Vec<String> {
        self.globals
            .contents()
            .clone_list()
            .iter()
            .map(|global| global.interface.clone())
            .collect()
    }

    /// Creates a toplevel and maps it with a `width` by `height` buffer of `colour`.
    fn map_window(&mut self, app_id: &str, title: &str, width: u32, height: u32, colour: u32) -> Window {
        let surface = self.compositor.create_surface(&self.qh, ());
        let xdg_surface = self.wm_base.get_xdg_surface(&surface, &self.qh, ());
        let toplevel = xdg_surface.get_toplevel(&self.qh, ());
        toplevel.set_app_id(app_id.to_owned());
        toplevel.set_title(title.to_owned());
        surface.commit();
        self.roundtrip();
        let shm = Shm::new(self, width, height, colour);
        surface.attach(Some(&shm.buffer), 0, 0);
        surface.damage(0, 0, width as i32, height as i32);
        surface.commit();
        self.roundtrip();
        Window {
            surface,
            xdg_surface,
            toplevel,
            shm,
        }
    }

    /// Redraws part of a window and waits until the compositor presented the commit.
    ///
    /// The frame callback is the only signal that says "the frame with this damage
    /// reached the output", which is what makes a damage assertion about the *next*
    /// frame rather than about a race.
    fn damage_and_wait(&mut self, window: &Window, x: u32, y: u32, width: u32, height: u32) {
        let callback = window.surface.frame(&self.qh, ());
        window.surface.damage(x as i32, y as i32, width as i32, height as i32);
        window.surface.commit();
        let deadline = Instant::now() + TIMEOUT;
        while callback.is_alive() {
            assert!(Instant::now() < deadline, "the compositor never presented the commit");
            self.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Sends `ext_session_lock_manager_v1.lock`.
    fn lock(&mut self) {
        let manager = self.lock_manager.as_ref().expect("a locker client");
        let lock = manager.lock(&self.qh, ());
        self.locks.push(lock);
        self.roundtrip();
    }

    /// Creates the lock surface for output `index` and commits a buffer to it.
    fn lock_surface(&mut self, index: usize) {
        let output = self.outputs.get(index).expect("an output").clone();
        let lock = self.locks.last().expect("a lock request");
        let surface = self.compositor.create_surface(&self.qh, ());
        let lock_surface = lock.get_lock_surface(&surface, &output, &self.qh, ());
        self.roundtrip();
        assert!(self.state.lock_configures >= 1, "the lock surface was configured");
        let (width, height) = (self.state.lock_width as u32, self.state.lock_height as u32);
        let shm = Shm::new(self, width, height, BLACK);
        surface.attach(Some(&shm.buffer), 0, 0);
        surface.damage(0, 0, width as i32, height as i32);
        surface.commit();
        self.roundtrip();
        let _surface = LockSurface {
            surface,
            lock_surface,
            shm,
        };
    }

    /// Waits until this client received `locked`.
    fn wait_for_locked(&mut self) {
        let deadline = Instant::now() + TIMEOUT;
        while self.state.locked == 0 {
            assert!(Instant::now() < deadline, "the locker never received `locked`");
            self.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Locks the session and covers its only output.
    fn lock_and_cover(&mut self) {
        self.lock();
        self.wait_for_locked();
        self.lock_surface(0);
    }

    /// A screencopy frame of the whole output.
    fn screencopy(&mut self, index: usize) -> zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1 {
        let manager = self.screens.as_ref().expect("a capture client");
        let output = self.outputs.get(index).expect("an output").clone();
        let frame = manager.capture_output(0, &output, &self.qh, ());
        self.roundtrip();
        frame
    }

    /// The buffer geometry the last screencopy frame advertised.
    fn screencopy_geometry(&self) -> (u32, u32, u32, u32) {
        let deadline = Instant::now() + TIMEOUT;
        while self.state.screencopy_buffer.is_none() {
            assert!(Instant::now() < deadline, "the frame never advertised a buffer");
            thread::sleep(Duration::from_millis(5));
        }
        self.state.screencopy_buffer.expect("a buffer event")
    }

    /// The image-copy session's constraints, once they arrived.
    fn session_geometry(&self) -> (u32, u32) {
        self.state.session_buffer_size.expect("a buffer size event")
    }

    /// Waits until the toplevel list named a window `title` and answers its handle.
    fn toplevel_handle(&mut self, title: &str) -> ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1 {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(toplevel) = self.state.toplevels.iter().find(|toplevel| toplevel.title == title) {
                return toplevel.handle.clone();
            }
            assert!(Instant::now() < deadline, "the toplevel list never named {title}");
            self.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// A toplevel capture source for the window `title` names.
    fn toplevel_source_for(
        &mut self,
        title: &str,
    ) -> wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1
    {
        let handle = self.toplevel_handle(title);
        let manager = self.toplevel_source.clone().expect("a capture client");
        manager.create_source(&handle, &self.qh, ())
    }

    /// Reads events until `predicate` holds for this client's state.
    fn wait_until(&mut self, predicate: impl Fn(&App) -> bool, what: &str) {
        let deadline = Instant::now() + TIMEOUT;
        while !predicate(&self.state) {
            assert!(
                Instant::now() < deadline,
                "the client never saw {what}; state: {:#?}",
                self.state
            );
            self.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }
    }
}

#[test]
fn a_screencopy_of_the_output_returns_the_window_pixels() {
    let session = Session::start();
    session.run(|session| {
        let mut app = Client::app(session.socket());
        let _window = app.map_window("org.gnoblin.Red", "Red", 200, 150, RED);
        let geometry = session.only_window();

        let mut capture = Client::capturer(session.privileged_socket());
        let frame = capture.screencopy(0);
        let (width, height, _stride, _format) = capture.screencopy_geometry();
        let shm = Shm::new(&capture, width, height, 0);
        frame.copy(&shm.buffer);
        capture.wait_until(|state| state.screencopy_ready >= 1, "the screencopy frame ready");

        // The window is drawn where the window manager placed it, so the pixel inside it
        // is the client's red and a pixel outside it is the compositor's background.
        let inside = shm.pixel(geometry.x as u32 + 5, geometry.y as u32 + 5);
        assert_eq!(inside, RED, "the capture returned the window's own pixels");
        let outside = shm.pixel(width - 5, height - 5);
        assert_eq!(outside, BLACK, "and the background where the window is not");
    });
}

#[test]
fn a_second_image_copy_frame_reports_only_the_changed_region() {
    let session = Session::start();
    session.run(|session| {
        let mut app = Client::app(session.socket());
        let window = app.map_window("org.gnoblin.Damaged", "Damaged", 200, 150, RED);
        let geometry = session.only_window();

        let mut capture = Client::capturer(session.privileged_socket());
        let output = capture.outputs.first().expect("an output").clone();
        let manager = capture.image_copy.clone().expect("a capture client");
        let sources = capture.output_source.clone().expect("a capture client");
        let source = sources.create_source(&output, &capture.qh, ());
        let copy_session = manager.create_session(
            &source,
            ext_image_copy_capture_manager_v1::Options::empty(),
            &capture.qh,
            (),
        );
        capture.wait_until(|state| state.session_done >= 1, "the session constraints");
        let (width, height) = capture.session_geometry();

        // The first frame of a session always carries full damage.
        let shm = Shm::new(&capture, width, height, 0);
        let frame = copy_session.create_frame(&capture.qh, ());
        frame.attach_buffer(&shm.buffer);
        frame.damage_buffer(0, 0, width as i32, height as i32);
        frame.capture();
        capture.wait_until(|state| state.frame_ready >= 1, "the first frame ready");
        assert_eq!(
            capture.state.frame_damage,
            vec![Damage {
                x: 0,
                y: 0,
                width: width as i32,
                height: height as i32
            }],
            "the first frame damages the whole buffer"
        );
        assert_eq!(shm.pixel(geometry.x as u32 + 5, geometry.y as u32 + 5), RED);
        frame.destroy();

        // The client redraws one corner of its window; only that rectangle changed.
        app.damage_and_wait(&window, 0, 0, 40, 30);
        capture.state.frame_damage.clear();
        let frame = copy_session.create_frame(&capture.qh, ());
        frame.attach_buffer(&shm.buffer);
        frame.damage_buffer(0, 0, width as i32, height as i32);
        let before = capture.state.frame_ready;
        frame.capture();
        capture.wait_until(|state| state.frame_ready > before, "the second frame ready");

        let damage = &capture.state.frame_damage;
        assert!(!damage.is_empty(), "the second frame reports the region that changed");
        let whole_output = Damage {
            x: 0,
            y: 0,
            width: width as i32,
            height: height as i32,
        };
        assert_ne!(
            damage.as_slice(),
            &[whole_output],
            "the second frame does not damage the whole output"
        );
        for rect in damage {
            assert!(
                rect.x >= geometry.x && rect.y >= geometry.y,
                "every damaged rectangle is inside the window that changed, got {rect:?} for {geometry:?}"
            );
        }
        // The client's buffer still holds the window's pixels where it did not change.
        assert_eq!(shm.pixel(geometry.x as u32 + 5, geometry.y as u32 + 5), RED);
    });
}

#[test]
fn a_capture_during_the_lock_is_refused() {
    let session = Session::start();
    session.run(|session| {
        let mut app = Client::app(session.socket());
        let _window = app.map_window("org.gnoblin.Hidden", "Hidden", 200, 150, RED);
        session.only_window();

        let mut locker = Client::locker(session.socket());
        locker.lock_and_cover();
        session.wait_for_lock(LockState::Locked);

        let mut capture = Client::capturer(session.privileged_socket());
        let frame = capture.screencopy(0);
        // The frame object exists and is refused: the client may not even allocate a
        // buffer for an output it cannot see.
        capture.wait_until(|state| state.screencopy_failed >= 1, "the refused screencopy frame");
        assert_eq!(capture.state.screencopy_ready, 0, "no frame was produced");
        assert_eq!(
            capture.state.screencopy_buffer, None,
            "a covered session advertises no buffer"
        );
        let _ = frame;

        // The modern protocol refuses the session itself, before any geometry reaches it.
        let output = capture.outputs.first().expect("an output").clone();
        let manager = capture.image_copy.clone().expect("a capture client");
        let sources = capture.output_source.clone().expect("a capture client");
        let source = sources.create_source(&output, &capture.qh, ());
        let _copy_session = manager.create_session(
            &source,
            ext_image_copy_capture_manager_v1::Options::empty(),
            &capture.qh,
            (),
        );
        capture.roundtrip();
        capture.roundtrip();
        assert_eq!(
            capture.state.session_buffer_size, None,
            "a locked session learns no geometry"
        );
        assert_eq!(capture.state.session_done, 0, "and receives no constraints");
    });
}

#[test]
fn a_refused_frame_never_repeats_the_previous_pixels() {
    let session = Session::start();
    session.run(|session| {
        let mut app = Client::app(session.socket());
        let _window = app.map_window("org.gnoblin.Stale", "Stale", 200, 150, RED);
        let geometry = session.only_window();

        let mut capture = Client::capturer(session.privileged_socket());
        let frame = capture.screencopy(0);
        let (width, height, _stride, _format) = capture.screencopy_geometry();
        let first = Shm::new(&capture, width, height, 0);
        frame.copy(&first.buffer);
        capture.wait_until(|state| state.screencopy_ready >= 1, "the screencopy frame ready");
        assert_eq!(first.pixel(geometry.x as u32 + 5, geometry.y as u32 + 5), RED);
        frame.destroy();

        let mut locker = Client::locker(session.socket());
        locker.lock_and_cover();
        session.wait_for_lock(LockState::Locked);

        // A second capture, into a fresh buffer: it fails and the buffer keeps whatever
        // the client left in it, which is zero. A compositor that answered from the
        // previous frame would leave red here.
        let frame = capture.screencopy(0);
        let refused = Shm::new(&capture, width, height, 0);
        frame.copy(&refused.buffer);
        capture.wait_until(|state| state.screencopy_failed >= 1, "the refused frame");
        assert_eq!(capture.state.screencopy_ready, 1, "and no new frame was produced");
        assert!(
            refused.is_uniform(0),
            "a refused capture leaves the client's buffer untouched"
        );
    });
}

#[test]
fn a_stream_started_before_the_lock_stops() {
    let session = Session::start();
    session.run(|session| {
        let mut app = Client::app(session.socket());
        let _window = app.map_window("org.gnoblin.Streamed", "Streamed", 200, 150, RED);
        session.only_window();

        let mut capture = Client::capturer(session.privileged_socket());
        let output = capture.outputs.first().expect("an output").clone();
        let manager = capture.image_copy.clone().expect("a capture client");
        let sources = capture.output_source.clone().expect("a capture client");
        let source = sources.create_source(&output, &capture.qh, ());
        let copy_session = manager.create_session(
            &source,
            ext_image_copy_capture_manager_v1::Options::empty(),
            &capture.qh,
            (),
        );
        capture.wait_until(|state| state.session_done >= 1, "the session constraints");
        let (width, height) = capture.session_geometry();
        let shm = Shm::new(&capture, width, height, 0);
        let frame = copy_session.create_frame(&capture.qh, ());
        frame.attach_buffer(&shm.buffer);
        frame.damage_buffer(0, 0, width as i32, height as i32);
        frame.capture();
        capture.wait_until(|state| state.frame_ready >= 1, "the first frame before the lock");
        frame.destroy();

        let mut locker = Client::locker(session.socket());
        locker.lock_and_cover();
        session.wait_for_lock(LockState::Locked);

        // The stream was opened before the lock and is cut by it: the client hears
        // `stopped` without asking for another frame.
        capture.wait_until(|state| state.session_stopped >= 1, "the session stopped");
        let next = copy_session.create_frame(&capture.qh, ());
        next.attach_buffer(&shm.buffer);
        next.capture();
        capture.wait_until(|state| state.frame_failed >= 1, "the frame of a stopped session");
        assert_eq!(capture.state.frame_ready, 1, "no frame survived the lock");
    });
}

#[test]
fn a_session_ends_when_its_client_disconnects() {
    let session = Session::start();
    session.run(|session| {
        let mut app = Client::app(session.socket());
        let _window = app.map_window("org.gnoblin.Leaving", "Leaving", 200, 150, RED);
        session.only_window();

        {
            let mut capture = Client::capturer(session.privileged_socket());
            let output = capture.outputs.first().expect("an output").clone();
            let manager = capture.image_copy.clone().expect("a capture client");
            let sources = capture.output_source.clone().expect("a capture client");
            let source = sources.create_source(&output, &capture.qh, ());
            let _copy_session = manager.create_session(
                &source,
                ext_image_copy_capture_manager_v1::Options::empty(),
                &capture.qh,
                (),
            );
            capture.wait_until(|state| state.session_done >= 1, "the session constraints");
            assert_eq!(
                session
                    .wait_for(|observation| observation.capture_sessions() == 1)
                    .capture_sessions(),
                1,
                "the session is in the table"
            );
            // Dropping the client closes the connection, which is a crashed adapter.
        }

        let observation = session.wait_for(|observation| observation.capture_sessions() == 0);
        assert_eq!(observation.capture_sessions(), 0, "the session went with its client");
    });
}

#[test]
fn a_window_source_follows_its_window_and_ends_with_it() {
    let session = Session::start();
    session.run(|session| {
        let mut app = Client::app(session.socket());
        let window = app.map_window("org.gnoblin.Followed", "Followed", 200, 150, RED);
        let geometry = session.only_window();

        let mut capture = Client::capturer(session.privileged_socket());
        let manager = capture.image_copy.clone().expect("a capture client");
        let source = capture.toplevel_source_for("Followed");
        let copy_session = manager.create_session(
            &source,
            ext_image_copy_capture_manager_v1::Options::empty(),
            &capture.qh,
            (),
        );
        capture.wait_until(|state| state.session_done >= 1, "the session constraints");
        let (width, height) = capture.session_geometry();
        // The constraints are the window's own rectangle, which is what a toplevel source
        // asks for: the frame it returns is the window, not the output it sits on.
        assert_eq!(
            (width, height),
            (geometry.width, geometry.height),
            "the constraints are the window's own size"
        );

        let shm = Shm::new(&capture, width, height, 0);
        let frame = copy_session.create_frame(&capture.qh, ());
        frame.attach_buffer(&shm.buffer);
        frame.damage_buffer(0, 0, width as i32, height as i32);
        frame.capture();
        capture.wait_until(|state| state.frame_ready >= 1, "the first window frame");
        assert_eq!(shm.pixel(5, 5), RED, "the frame is the window's own pixels");
        frame.destroy();

        // Move the window across the output. A copy that remembered the rectangle from
        // when the session opened would now return the background it used to cover.
        let moved = gnoblin_types::Rect::new(geometry.x + 100, geometry.y + 60, geometry.width, geometry.height);
        session.control.move_window(gnoblin_types::WindowId::new(1), moved);
        session.wait_for(|observation| {
            observation
                .only()
                .is_some_and(|window| window.geometry.x == moved.x && window.geometry.y == moved.y)
        });
        let frame = copy_session.create_frame(&capture.qh, ());
        frame.attach_buffer(&shm.buffer);
        frame.damage_buffer(0, 0, width as i32, height as i32);
        let before = capture.state.frame_ready;
        frame.capture();
        capture.wait_until(|state| state.frame_ready > before, "the frame after the move");
        assert_eq!(shm.pixel(5, 5), RED, "the capture followed the window");
        frame.destroy();

        // Closing the window ends the session: the frame fails and the client is told
        // the session stopped.
        window.toplevel.destroy();
        window.xdg_surface.destroy();
        window.surface.destroy();
        app.roundtrip();
        capture.wait_until(|state| state.session_stopped >= 1, "the session of the closed window");
        // The toplevel list says the window is gone too, which is what a taskbar sees.
        capture.wait_until(|state| state.toplevels_closed >= 1, "the toplevel closed");
    });
}

#[test]
fn a_sandboxed_client_cannot_bind_the_capture_globals() {
    // An application's connection is unprivileged: it may not see another client's
    // pixels, and the roster is the one answer both capture protocols read.
    let session = Session::start();
    session.run(|session| {
        let client = Client::app(session.socket());
        let advertised = client.advertised();
        for global in [
            "zwlr_screencopy_manager_v1",
            "ext_image_copy_capture_manager_v1",
            "ext_output_image_capture_source_manager_v1",
            "ext_foreign_toplevel_image_capture_source_manager_v1",
            "ext_foreign_toplevel_list_v1",
        ] {
            assert!(
                !advertised.iter().any(|name| name == global),
                "{global} must not reach an untrusted connection; it saw {advertised:?}"
            );
        }
        // And binding it anyway fails, because the global is not in the registry list.
        let screens: Result<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1, _> =
            client.globals.bind(&client.qh, 1..=3, ());
        assert!(screens.is_err(), "an unprivileged connection cannot bind screencopy");

        // The same globals are exactly what the session's own socket sees, so the denial
        // is the role and not a missing feature.
        let mut privileged = Client::capturer(session.privileged_socket());
        let seen = privileged.advertised();
        for global in [
            "zwlr_screencopy_manager_v1",
            "ext_image_copy_capture_manager_v1",
            "ext_output_image_capture_source_manager_v1",
            "ext_foreign_toplevel_image_capture_source_manager_v1",
        ] {
            assert!(
                seen.iter().any(|name| name == global),
                "a privileged connection sees {global}; it saw {seen:?}"
            );
        }
        privileged.roundtrip();
    });
}

#[test]
fn a_disabled_protocol_is_not_advertised() {
    // The configured half of the same gate: turning a protocol off in the document
    // hides it from a client that would otherwise see it.
    let mut protocols = gnoblin_config::Protocols::default();
    protocols.toggles.insert(gnoblin_config::Protocol::WlrScreencopy, false);
    let session = Session::start_with(protocols);
    session.run(|session| {
        // Only the registry list is read: binding the capture globals would bind the one
        // this test turned off.
        let mut client = Client::connect(session.privileged_socket());
        let advertised = client.advertised();
        assert!(
            !advertised.iter().any(|name| name == "zwlr_screencopy_manager_v1"),
            "a disabled protocol is advertised to nobody; saw {advertised:?}"
        );
        // The other capture globals stay: the configuration names them separately.
        assert!(
            advertised
                .iter()
                .any(|name| name == "ext_image_copy_capture_manager_v1"),
            "the modern protocol is still advertised"
        );
        client.roundtrip();
    });
}

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

impl Dispatch<wl_output::WlOutput, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_output::WlOutput,
        _: <wl_output::WlOutput as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for App {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: <wl_callback::WlCallback as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.frame_callbacks += 1;
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for App {
    fn event(
        state: &mut Self,
        _: &wl_buffer::WlBuffer,
        event: <wl_buffer::WlBuffer as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            state.released_buffers += 1;
        }
    }
}

impl Dispatch<wl_shm::WlShm, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_shm::WlShm,
        _: <wl_shm::WlShm as Proxy>::Event,
        _: &(),
        _: &Connection,
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
        _: &Connection,
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
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_compositor::WlCompositor,
        _: <wl_compositor::WlCompositor as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
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
            let _ = state;
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for App {
    fn event(
        state: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: <xdg_surface::XdgSurface as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            state.configures += 1;
            state.acks += 1;
            surface.ack_configure(serial);
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
            state.toplevel_configures += 1;
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
        match event {
            ext_session_lock_v1::Event::Locked => state.locked += 1,
            ext_session_lock_v1::Event::Finished => state.finished += 1,
            _ => {}
        }
    }
}

impl Dispatch<ext_session_lock_surface_v1::ExtSessionLockSurfaceV1, ()> for App {
    fn event(
        state: &mut Self,
        surface: &ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
        event: <ext_session_lock_surface_v1::ExtSessionLockSurfaceV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure { serial, width, height } = event {
            state.lock_configures += 1;
            state.lock_width = width as i32;
            state.lock_height = height as i32;
            surface.ack_configure(serial);
        }
    }
}

impl Dispatch<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
        _: <zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
        event: <zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format: WEnum::Value(format),
                width,
                height,
                stride,
            } => state.screencopy_buffer = Some((width, height, stride, format as u32)),
            zwlr_screencopy_frame_v1::Event::BufferDone => state.screencopy_buffer_done = true,
            zwlr_screencopy_frame_v1::Event::Damage { x, y, width, height } => state.screencopy_damage.push(Damage {
                x: x as i32,
                y: y as i32,
                width: width as i32,
                height: height as i32,
            }),
            zwlr_screencopy_frame_v1::Event::Ready { .. } => state.screencopy_ready += 1,
            zwlr_screencopy_frame_v1::Event::Failed => state.screencopy_failed += 1,
            _ => {}
        }
    }
}

impl Dispatch<ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1,
        _: <ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Each handle carries its own events; the list itself only names them.
    }

    wayland_client::event_created_child!(App, ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1, ()> for App {
    fn event(
        state: &mut Self,
        handle: &ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
        event: <ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let handle = ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1::clone(handle);
        match event {
            ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } => {
                state.toplevels.push(Toplevel {
                    handle,
                    identifier,
                    title: String::new(),
                    app_id: String::new(),
                });
            }
            ext_foreign_toplevel_handle_v1::Event::Title { title } => {
                if let Some(entry) = state.toplevels.iter_mut().find(|entry| entry.handle == handle) {
                    entry.title = title;
                }
            }
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                if let Some(entry) = state.toplevels.iter_mut().find(|entry| entry.handle == handle) {
                    entry.app_id = app_id;
                }
            }
            ext_foreign_toplevel_handle_v1::Event::Closed => state.toplevels_closed += 1,
            _ => {}
        }
    }
}

impl Dispatch<ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
        _: <ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1, ()>
    for App
{
    fn event(
        _: &mut Self,
        _: &ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1,
        _: <ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl
    Dispatch<
        wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1,
        (),
    > for App
{
    fn event(
        _: &mut Self,
        _: &wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1,
        _: <wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1,
        _: <ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1,
        event: <ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                state.session_buffer_size = Some((width, height));
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat {
                format: WEnum::Value(format),
            } => state.session_formats.push(format as u32),
            ext_image_copy_capture_session_v1::Event::Done => state.session_done += 1,
            ext_image_copy_capture_session_v1::Event::Stopped => state.session_stopped += 1,
            _ => {}
        }
    }
}

impl Dispatch<ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1,
        event: <ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_frame_v1::Event::Damage { x, y, width, height } => {
                state.frame_damage.push(Damage { x, y, width, height })
            }
            ext_image_copy_capture_frame_v1::Event::Ready => state.frame_ready += 1,
            ext_image_copy_capture_frame_v1::Event::Failed { .. } => state.frame_failed += 1,
            _ => {}
        }
    }
}
