//! The cursor shape path, end to end: a real client names a pointer shape through
//! `wp_cursor_shape_v1`, the session draws the themed cursor into the frame, and a
//! screencopy with `overlay_cursor` shows the result.
//!
//! The session reads its theme from `XCURSOR_PATH` in the process environment, and
//! this workspace forbids `unsafe`, so the environment cannot be set in-process. Each
//! test therefore re-runs this test binary as a child with its own `XCURSOR_PATH` and
//! runs the session there. The child is selected by [`CHILD_MARKER`].

use std::fs::File;
use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::InputEvent;
use gnoblin_config::Protocols;
use gnoblin_cursor::ProviderKind;
use gnoblin_types::{Point, Rect};
use gnoblin_wayland::{Config, ControlHandle, CursorSettings, LoopError, Server, SessionSnapshot};
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum};
use wayland_protocols::wp::cursor_shape::v1::client::{wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::screencopy::v1::client::{zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1};

const TIMEOUT: Duration = Duration::from_secs(10);

/// Set in the environment of the child that runs a test's session; holds the test name.
const CHILD_MARKER: &str = "GNOBLIN_CURSOR_SHAPE_CHILD";

/// The window's pixels: opaque red, in the byte order `wl_shm` stores `argb8888` in.
const RED: u32 = 0xFFFF_0000;
/// The compositor background: opaque black.
const BLACK: u32 = 0xFF00_0000;
/// The solid colour the `default` cursor file is painted with.
const GREEN: u32 = 0xFF00_FF00;
/// The solid colour the `text` cursor file is painted with.
const BLUE: u32 = 0xFF00_00FF;

/// The cursor size the theme files and the configuration both use.
const CURSOR_SIZE: u16 = 24;
/// The theme the tests install and configure.
const THEME: &str = "gnoblin-test";

/// What a test body reads from its session.
#[derive(Clone)]
struct Handle {
    socket: PathBuf,
    privileged_socket: PathBuf,
    control: ControlHandle,
    observation: Arc<Mutex<SessionSnapshot>>,
}

impl Handle {
    /// Waits for the observation to satisfy `predicate`, or fails with the last one.
    fn wait_for(&self, predicate: impl Fn(&SessionSnapshot) -> bool) -> SessionSnapshot {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let observation = self.observation.lock().expect("the observation lock").clone();
            if predicate(&observation) {
                return observation;
            }
            assert!(
                Instant::now() < deadline,
                "the session never reached the expected state: {observation:#?}"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// The geometry of the only window, once it is mapped.
    fn only_window(&self) -> Rect {
        let observation = self.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        observation.only().expect("one mapped window").geometry
    }

    /// Moves the pointer to `at` in the first output's logical space.
    fn move_pointer(&self, at: Point) {
        let observation = self.wait_for(|observation| !observation.outputs().is_empty());
        self.control.inject_input(InputEvent::PointerMotion {
            output: observation.outputs()[0],
            position: at,
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

/// Runs a test that needs a cursor theme.
///
/// In the parent process it writes `cursors` (name and solid colour) into a theme
/// directory and re-runs the current test as a child whose `XCURSOR_PATH` is that
/// directory. In the child it starts the session with `theme` and runs `body`.
fn with_theme(test: &str, theme: &str, cursors: &[(&str, u32)], body: impl FnOnce(&Handle) + Send + 'static) {
    let themes = tempfile::tempdir().expect("a theme directory");
    if std::env::var(CHILD_MARKER).as_deref() == Ok(test) {
        run_session(theme, body);
        return;
    }
    for (name, colour) in cursors {
        install_cursor(themes.path(), name, *colour);
    }
    let status = Command::new(std::env::current_exe().expect("this test binary"))
        .args([test, "--exact", "--nocapture", "--test-threads=1"])
        .env("XCURSOR_PATH", themes.path())
        .env(CHILD_MARKER, test)
        .status()
        .expect("the child test process starts");
    assert!(status.success(), "the child session for {test} failed: {status}");
}

/// Starts a session with cursor theme `theme`, runs `body` on a worker, and joins both.
fn run_session(theme: &str, body: impl FnOnce(&Handle) + Send + 'static) {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let socket = directory.path().join("wayland-0.sock");
    let privileged_socket = directory.path().join("wayland-0-privileged.sock");
    let config = Config {
        socket_path: Some(socket.clone()),
        privileged_socket_path: Some(privileged_socket.clone()),
        protocols: Protocols::default(),
        cursor: CursorSettings {
            theme: theme.to_owned(),
            size: CURSOR_SIZE,
            providers: vec![ProviderKind::Xcursor],
        },
        ..Config::default()
    };
    let server = Server::new(Box::new(gnoblin_backend::headless::Headless::new()), config).expect("the session starts");
    let handle = Handle {
        socket,
        privileged_socket,
        control: server.control(),
        observation: server.observation(),
    };
    let worker = thread::spawn({
        let handle = handle.clone();
        move || {
            let _stop = StopOnDrop(handle.control.clone());
            body(&handle);
        }
    });
    let outcome: Result<(), LoopError> = server.run();
    if let Err(payload) = worker.join() {
        std::panic::resume_unwind(payload);
    }
    outcome.expect("the loop stops cleanly");
}

/// Appends a native-endian `u32` to an Xcursor file being built.
fn push(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

/// Writes a one-image Xcursor file of a solid `colour` at `<themes>/THEME/cursors/<name>`.
fn install_cursor(themes: &Path, name: &str, colour: u32) {
    let directory = themes.join(THEME).join("cursors");
    std::fs::create_dir_all(&directory).expect("a theme directory");
    let size = u32::from(CURSOR_SIZE);
    let mut bytes = Vec::new();
    // File header: magic, header length, version, table-of-contents length.
    bytes.extend_from_slice(b"Xcur");
    push(&mut bytes, 16);
    push(&mut bytes, 0x0001_0000);
    push(&mut bytes, 1);
    // One table-of-contents entry: an image of the nominal size, at offset 28.
    push(&mut bytes, 0xfffd_0002);
    push(&mut bytes, size);
    push(&mut bytes, 28);
    // Image chunk: header length, type, subtype, version, width, height, hotspot, delay.
    push(&mut bytes, 36);
    push(&mut bytes, 0xfffd_0002);
    push(&mut bytes, size);
    push(&mut bytes, 1);
    push(&mut bytes, size);
    push(&mut bytes, size);
    push(&mut bytes, 0);
    push(&mut bytes, 0);
    push(&mut bytes, 0);
    for _ in 0..size * size {
        push(&mut bytes, colour);
    }
    File::create(directory.join(name))
        .and_then(|mut file| file.write_all(&bytes))
        .expect("a cursor file");
}

/// A `wl_shm` buffer filled with a solid colour, read back through the file it lives in.
struct Shm {
    file: File,
    buffer: wl_buffer::WlBuffer,
    width: u32,
    height: u32,
    stride: u32,
}

impl Shm {
    fn new(client: &Client, width: u32, height: u32, colour: u32) -> Self {
        let stride = width * 4;
        let length = (stride * height) as usize;
        let mut file = tempfile::tempfile().expect("a shared memory file");
        let mut bytes = Vec::with_capacity(length);
        for _ in 0..width * height {
            bytes.extend_from_slice(&colour.to_ne_bytes());
        }
        file.write_all(&bytes).expect("the pixels");
        let shm = client.shm.as_ref().expect("wl_shm");
        let pool: wl_shm_pool::WlShmPool = shm.create_pool(file.as_fd(), length as i32, &client.qh, ());
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
}

/// The events one client records.
#[derive(Default)]
struct App {
    pointer: Option<wl_pointer::WlPointer>,
    enter_serial: Option<u32>,
    screencopy_size: Option<(u32, u32, u32)>,
    screencopy_ready: u32,
    screencopy_failed: u32,
}

/// One client connection and the globals it bound.
struct Client {
    _connection: Connection,
    queue: EventQueue<App>,
    qh: QueueHandle<App>,
    state: App,
    shm: Option<wl_shm::WlShm>,
    compositor: Option<wl_compositor::WlCompositor>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    shapes: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    screens: Option<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1>,
    output: Option<wl_output::WlOutput>,
}

impl Client {
    fn connect(socket: &Path) -> (Connection, EventQueue<App>, GlobalList) {
        let stream = UnixStream::connect(socket).expect("the session's socket");
        let connection = Connection::from_socket(stream).expect("a connection");
        let (globals, queue) = registry_queue_init::<App>(&connection).expect("the globals");
        (connection, queue, globals)
    }

    /// An application connection: a window, a seat with a pointer, and the shape manager.
    fn app(socket: &Path) -> Self {
        let (connection, mut queue, globals) = Self::connect(socket);
        let qh = queue.handle();
        let compositor = globals.bind(&qh, 1..=6, ()).expect("wl_compositor");
        let wm_base = globals.bind(&qh, 1..=6, ()).expect("xdg_wm_base");
        let shapes = globals.bind(&qh, 1..=1, ()).expect("wp_cursor_shape_manager_v1");
        let shm = globals.bind(&qh, 1..=1, ()).expect("wl_shm");
        let _seat: wl_seat::WlSeat = globals.bind(&qh, 1..=9, ()).expect("wl_seat");
        let mut state = App::default();
        queue.roundtrip(&mut state).expect("a roundtrip");
        Self {
            _connection: connection,
            queue,
            qh,
            state,
            shm: Some(shm),
            compositor: Some(compositor),
            wm_base: Some(wm_base),
            shapes: Some(shapes),
            screens: None,
            output: None,
        }
    }

    /// A capture connection: screencopy and the output, on the privileged socket.
    fn capturer(socket: &Path) -> Self {
        let (connection, mut queue, globals) = Self::connect(socket);
        let qh = queue.handle();
        let shm = globals.bind(&qh, 1..=1, ()).expect("wl_shm");
        let screens = globals.bind(&qh, 1..=3, ()).expect("zwlr_screencopy_manager_v1");
        let output = globals.bind(&qh, 1..=4, ()).expect("wl_output");
        let mut state = App::default();
        queue.roundtrip(&mut state).expect("a roundtrip");
        Self {
            _connection: connection,
            queue,
            qh,
            state,
            shm: Some(shm),
            compositor: None,
            wm_base: None,
            shapes: None,
            screens: Some(screens),
            output: Some(output),
        }
    }

    fn roundtrip(&mut self) {
        self.queue.roundtrip(&mut self.state).expect("a roundtrip");
    }

    /// Reads events until `predicate` holds for this client's state.
    fn wait_until(&mut self, predicate: impl Fn(&App) -> bool, what: &str) {
        let deadline = Instant::now() + TIMEOUT;
        while !predicate(&self.state) {
            assert!(Instant::now() < deadline, "the client never saw {what}");
            self.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Maps a red window of the given size and leaves it mapped for the test.
    fn map_red_window(&mut self, width: u32, height: u32) {
        let compositor = self.compositor.clone().expect("wl_compositor");
        let wm_base = self.wm_base.clone().expect("xdg_wm_base");
        let surface = compositor.create_surface(&self.qh, ());
        let xdg = wm_base.get_xdg_surface(&surface, &self.qh, ());
        let toplevel = xdg.get_toplevel(&self.qh, ());
        toplevel.set_app_id("org.gnoblin.Red".to_owned());
        toplevel.set_title("Red".to_owned());
        surface.commit();
        self.roundtrip();
        let shm = Shm::new(self, width, height, RED);
        surface.attach(Some(&shm.buffer), 0, 0);
        surface.damage(0, 0, width as i32, height as i32);
        surface.commit();
        self.roundtrip();
        // The window and its buffer stay alive for the rest of the test.
        std::mem::forget((surface, xdg, toplevel, shm));
    }

    /// Names `shape` for the pointer over the window, once the pointer has entered it.
    fn set_pointer_shape(&mut self, shape: wp_cursor_shape_device_v1::Shape) {
        self.wait_until(|state| state.enter_serial.is_some(), "the pointer enter");
        let serial = self.state.enter_serial.expect("an enter serial");
        let pointer = self.state.pointer.clone().expect("a pointer");
        let shapes = self.shapes.clone().expect("the shape manager");
        let device = shapes.get_pointer(&pointer, &self.qh, ());
        device.set_shape(serial, shape);
        self.roundtrip();
        device.destroy();
    }

    /// A screencopy of the whole output, with the cursor when `overlay` is 1.
    fn capture(&mut self, overlay: i32) -> Shm {
        let screens = self.screens.clone().expect("the screencopy manager");
        let output = self.output.clone().expect("an output");
        self.state.screencopy_size = None;
        let frame: zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1 =
            screens.capture_output(overlay, &output, &self.qh, ());
        self.wait_until(|state| state.screencopy_size.is_some(), "the screencopy buffer size");
        let (width, height, _stride) = self.state.screencopy_size.expect("a buffer size");
        let shm = Shm::new(self, width, height, BLACK);
        let ready_before = self.state.screencopy_ready;
        frame.copy(&shm.buffer);
        self.wait_until(
            |state| state.screencopy_ready > ready_before || state.screencopy_failed > 0,
            "the screencopy frame ready",
        );
        assert_eq!(self.state.screencopy_failed, 0, "the screencopy failed");
        frame.destroy();
        shm
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for App {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for App {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
            && capabilities.contains(wl_seat::Capability::Pointer)
            && state.pointer.is_none()
        {
            state.pointer = Some(seat.get_pointer(qh, ()));
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for App {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Enter { serial, .. } = event {
            state.enter_serial = Some(serial);
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for App {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for App {
    fn event(
        _: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
        }
    }
}

impl Dispatch<zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format: WEnum::Value(_),
                width,
                height,
                stride,
            } => state.screencopy_size = Some((width, height, stride)),
            zwlr_screencopy_frame_v1::Event::Ready { .. } => state.screencopy_ready += 1,
            zwlr_screencopy_frame_v1::Event::Failed => state.screencopy_failed += 1,
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(App: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(App: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(App: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(App: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(App: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(App: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(App: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(App: ignore wp_cursor_shape_manager_v1::WpCursorShapeManagerV1);
wayland_client::delegate_noop!(App: ignore wp_cursor_shape_device_v1::WpCursorShapeDeviceV1);
wayland_client::delegate_noop!(App: ignore zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1);

/// A point a little inside the cursor image, so the same sample lands on the cursor
/// whatever its hotspot.
fn tip_sample(at: Point) -> (u32, u32) {
    ((at.x + 2) as u32, (at.y + 2) as u32)
}

/// Screencopies with the cursor until `done` accepts one. The pointer's motion is
/// queued on the compositor, so the first frame may still show the cursor at its old
/// place; this waits, bounded by `TIMEOUT`, for the frame to show the new one.
fn capture_until(capture: &mut Client, done: impl Fn(&Shm) -> bool) -> Shm {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let frame = capture.capture(1);
        if done(&frame) {
            return frame;
        }
        assert!(Instant::now() < deadline, "the frame never showed the expected cursor");
        thread::sleep(Duration::from_millis(10));
    }
}

/// Whether `frame` differs from `bare` in the cursor's box at `at`.
fn differs_near(frame: &Shm, bare: &Shm, at: Point) -> bool {
    let (x0, y0) = (at.x as u32, at.y as u32);
    (y0..y0 + 26).any(|y| (x0..x0 + 26).any(|x| frame.pixel(x, y) != bare.pixel(x, y)))
}

/// A pointer position inside the window, away from its edges.
fn pointer_inside(window: Rect) -> Point {
    Point::new(window.x + 50, window.y + 50)
}

/// Maps the red window, places the pointer over it and names `shape` for it.
fn pointer_over_window(app: &mut Client, session: &Handle, shape: wp_cursor_shape_device_v1::Shape) -> Point {
    app.map_red_window(200, 150);
    let at = pointer_inside(session.only_window());
    session.move_pointer(at);
    app.set_pointer_shape(shape);
    at
}

#[test]
fn a_pointer_shape_is_drawn_by_the_screencopy_overlay_and_not_without_it() {
    with_theme(
        "a_pointer_shape_is_drawn_by_the_screencopy_overlay_and_not_without_it",
        THEME,
        &[("default", GREEN)],
        |session| {
            let mut app = Client::app(&session.socket);
            let at = pointer_over_window(&mut app, session, wp_cursor_shape_device_v1::Shape::Default);
            let mut capture = Client::capturer(&session.privileged_socket);
            let (x, y) = tip_sample(at);
            let with_cursor = capture_until(&mut capture, |frame| frame.pixel(x, y) == GREEN);
            assert_eq!(
                with_cursor.pixel(x, y),
                GREEN,
                "the overlay draws the themed cursor at the pointer"
            );
            let without_cursor = capture.capture(0);
            assert_eq!(
                without_cursor.pixel(x, y),
                RED,
                "without the overlay the window shows through"
            );
        },
    );
}

#[test]
fn a_missing_theme_falls_back_to_the_built_in_arrow_with_its_tip_at_the_pointer() {
    with_theme(
        "a_missing_theme_falls_back_to_the_built_in_arrow_with_its_tip_at_the_pointer",
        "gnoblin-missing",
        &[],
        |session| {
            let mut app = Client::app(&session.socket);
            let at = pointer_over_window(&mut app, session, wp_cursor_shape_device_v1::Shape::Default);
            let mut capture = Client::capturer(&session.privileged_socket);
            let without_cursor = capture.capture(0);
            let with_cursor = capture_until(&mut capture, |frame| differs_near(frame, &without_cursor, at));

            // The built-in arrow is drawn: some pixels differ from the bare frame. The
            // first differing pixel in reading order is the arrow's top row, which sits at
            // the pointer's tip.
            let first = (0..without_cursor.height)
                .flat_map(|y| (0..without_cursor.width).map(move |x| (x, y)))
                .find(|&(x, y)| with_cursor.pixel(x, y) != without_cursor.pixel(x, y))
                .expect("the built-in arrow draws over the window");
            let (fx, fy) = (i64::from(first.0), i64::from(first.1));
            assert!(
                (fx - i64::from(at.x)).abs() <= 2 && (fy - i64::from(at.y)).abs() <= 2,
                "the arrow's tip sits at the pointer: first changed pixel {first:?}, pointer {at:?}"
            );
        },
    );
}

#[test]
fn the_default_and_text_shapes_draw_different_cursors_at_the_same_size() {
    with_theme(
        "the_default_and_text_shapes_draw_different_cursors_at_the_same_size",
        THEME,
        &[("default", GREEN), ("text", BLUE)],
        |session| {
            let mut app = Client::app(&session.socket);
            let at = pointer_over_window(&mut app, session, wp_cursor_shape_device_v1::Shape::Default);
            let (x, y) = tip_sample(at);
            let mut capture = Client::capturer(&session.privileged_socket);
            let default = capture_until(&mut capture, |frame| frame.pixel(x, y) == GREEN);

            app.set_pointer_shape(wp_cursor_shape_device_v1::Shape::Text);
            let text = capture_until(&mut capture, |frame| frame.pixel(x, y) == BLUE);
            assert_ne!(
                default.pixel(x, y),
                text.pixel(x, y),
                "the text shape draws a different cursor"
            );
        },
    );
}
