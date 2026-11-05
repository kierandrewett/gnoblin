//! The pixel round trip: a real client's buffer, the real compositor, real pixels.
//!
//! Everything here is end to end. A `wayland-client` connects over the session's own
//! socket, maps a toplevel, writes a solid colour into a `wl_shm` buffer and commits
//! it; the compositor's software renderer composes that buffer into its output's
//! offscreen buffer; and the test reads the result back through
//! [`ControlHandle::screenshot`]. No display, no GPU and no session of the machine's
//! own are involved.
//!
//! ## Why probes and not images
//!
//! The plan's rule for the renderer is "semantic pixel probes only across renderers;
//! full images only across candidate builds". So every assertion here samples a few
//! points and names what should be there: a red window's centre, the background where
//! the window is not, the blended value of a half-transparent window. A full-image
//! compare would pin the software path's rounding and make the GLES path's parity a
//! matter of tolerance arithmetic rather than of meaning.
//!
//! ## Two threads, on purpose
//!
//! A calloop loop and the backend it drives are not `Send`, so the session runs on the
//! test's thread and the client choreography runs on a worker. That is also the real
//! arrangement: the compositor is one process and the client is another. The worker
//! talks to the session through the control handle, which is `Send`, exactly as a
//! test kit would.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::Headless;
use gnoblin_types::{OutputId, Rect, WindowId};
use gnoblin_wayland::{Config, ControlHandle, FrameStats, LoopError, ObservedWindow, Server, SessionSnapshot};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// How long a test waits for the session to reach a state before failing.
const TIMEOUT: Duration = Duration::from_secs(10);

/// `BTN_LEFT` from `linux/input-event-codes.h`.
const BTN_LEFT: u32 = 0x110;

/// Red, in the RGBA order a probe reports.
const RED: [u8; 4] = [255, 0, 0, 255];
/// Blue, the second window's colour.
const BLUE: [u8; 4] = [0, 0, 255, 255];
/// Opaque black, the desktop background.
const BACKGROUND: [u8; 4] = [0, 0, 0, 255];

/// What a worker thread can reach of a running session.
#[derive(Clone)]
struct Handle {
    socket: PathBuf,
    control: ControlHandle,
    observation: Arc<Mutex<SessionSnapshot>>,
    frames: Arc<Mutex<FrameStats>>,
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

    /// The frame accounting right now.
    fn stats(&self) -> FrameStats {
        self.frames.lock().expect("the frame stats lock").clone()
    }

    /// The only output's identity.
    fn output(&self) -> OutputId {
        *self
            .windows()
            .outputs()
            .first()
            .expect("a headless session starts with one output")
    }

    /// The pixels the output currently shows.
    fn screenshot(&self) -> gnoblin_backend::render::Image {
        self.control
            .screenshot(self.output())
            .expect("the loop answers a screenshot request")
    }

    /// The colour at one point of the output.
    fn probe(&self, x: i32, y: i32) -> [u8; 4] {
        self.screenshot()
            .colour_at(x, y)
            .unwrap_or_else(|| panic!("the probe at ({x}, {y}) is inside the output"))
    }
}

/// Stops the session when the worker finishes, including when it panics: without this
/// a failing assertion would leave the loop running and hang the test.
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
            frames: server.frames(),
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
            // A failed assertion in the worker is the test's failure.
            std::panic::resume_unwind(payload);
        }
        outcome.expect("the loop stops cleanly");
    }
}

/// What the client records from the events it receives.
#[derive(Default)]
struct App {
    /// Whether to acknowledge configures. A window that stops acknowledging keeps the
    /// state it has, which is how a test gets one frame out of a move.
    ack: bool,
    acks: u32,
}

/// A real Wayland client for one connection.
struct Client {
    /// The client owns its connection so the socket closes when the client is
    /// dropped.
    #[allow(dead_code)]
    connection: Connection,
    queue: EventQueue<App>,
    state: App,
    qh: QueueHandle<App>,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    wm_base: xdg_wm_base::XdgWmBase,
    /// The buffers the client has attached, kept so the file stays mapped.
    #[allow(dead_code)]
    buffers: Vec<(wl_buffer::WlBuffer, wl_shm_pool::WlShmPool)>,
}

/// The objects of one toplevel.
struct Window {
    surface: wl_surface::WlSurface,
    xdg_surface: xdg_surface::XdgSurface,
    #[allow(dead_code)]
    toplevel: xdg_toplevel::XdgToplevel,
}

impl Client {
    /// Connects and binds the globals this test writes into.
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
        Self {
            connection,
            queue,
            state: App {
                ack: true,
                ..App::default()
            },
            qh,
            compositor,
            shm,
            wm_base,
            buffers: Vec::new(),
        }
    }

    /// Processes every event the server has sent.
    fn roundtrip(&mut self) {
        self.queue.roundtrip(&mut self.state).expect("a round trip");
    }

    /// Creates a toplevel and commits it once, which asks for a configure.
    fn create_window(&mut self, title: &str) -> Window {
        let surface = self.compositor.create_surface(&self.qh, ());
        let xdg_surface = self.wm_base.get_xdg_surface(&surface, &self.qh, ());
        let toplevel = xdg_surface.get_toplevel(&self.qh, ());
        toplevel.set_app_id("org.gnoblin.Render".to_owned());
        toplevel.set_title(title.to_owned());
        surface.commit();
        self.roundtrip();
        Window {
            surface,
            xdg_surface,
            toplevel,
        }
    }

    /// Attaches a `width` by `height` buffer filled with `colour` and commits.
    ///
    /// The pixels are written in the buffer's own ARGB8888 order, which is the format
    /// every client uses on a little-endian machine: blue, green, red, alpha.
    fn attach_solid(&mut self, window: &Window, width: i32, height: i32, colour: [u8; 4]) {
        self.attach_with_stride(window, width, height, width * 4, colour);
    }

    /// Attaches a buffer whose rows are `stride` bytes apart.
    ///
    /// A stride wider than the packed width is what a real toolkit produces when it
    /// aligns rows, and it is the case a renderer that assumes `width * 4` reads as
    /// diagonal garbage.
    fn attach_with_stride(&mut self, window: &Window, width: i32, height: i32, stride: i32, colour: [u8; 4]) {
        let length = stride * height;
        let mut file = tempfile::tempfile().expect("a shared memory file");
        file.set_len(length as u64).expect("a buffer that big");
        let [red, green, blue, alpha] = colour;
        let mut pixels = vec![0u8; length as usize];
        for row in 0..height as usize {
            for column in 0..width as usize {
                let start = row * stride as usize + column * 4;
                pixels[start..start + 4].copy_from_slice(&[blue, green, red, alpha]);
            }
        }
        file.write_all(&pixels).expect("the buffer's pixels");
        let pool = self.shm.create_pool(file.as_fd(), length, &self.qh, ());
        let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, &self.qh, ());
        window.surface.attach(Some(&buffer), 0, 0);
        window.surface.damage(0, 0, width, height);
        window.surface.commit();
        self.roundtrip();
        self.buffers.push((buffer, pool));
    }

    /// Destroys the first buffer the client attached, which a client may do at any
    /// time and which must not stop the compositor's frame.
    fn destroy_first_buffer(&mut self) {
        if self.buffers.is_empty() {
            return;
        }
        let (buffer, pool) = self.buffers.remove(0);
        buffer.destroy();
        pool.destroy();
        self.roundtrip();
    }

    /// Sends a pointer press at a global position, as the backend would.
    fn click(&mut self, session: &Handle, at: gnoblin_types::Point) {
        let output = session.output();
        session
            .control
            .inject_input(gnoblin_backend::InputEvent::PointerMotion { output, position: at });
        session
            .control
            .inject_input(gnoblin_backend::InputEvent::PointerButton {
                button: BTN_LEFT,
                pressed: true,
            });
        session
            .control
            .inject_input(gnoblin_backend::InputEvent::PointerButton {
                button: BTN_LEFT,
                pressed: false,
            });
        self.roundtrip();
    }
}

/// The centre of a rectangle, in global logical coordinates.
fn centre(rect: Rect) -> gnoblin_types::Point {
    gnoblin_types::Point::new(rect.x + (rect.width / 2) as i32, rect.y + (rect.height / 2) as i32)
}

/// The fallback size of the headless output, which no test changes.
fn screen() -> (i32, i32) {
    (
        gnoblin_backend::headless::DEFAULT_MODE.width as i32,
        gnoblin_backend::headless::DEFAULT_MODE.height as i32,
    )
}

/// Waits for the window's frame rectangle to be the one the test asked for.
fn wait_for_rect(session: &Handle, title: &str, rect: Rect) -> ObservedWindow {
    let observation =
        session.wait_for(|observation| observation.titled(title).is_some_and(|window| window.geometry == rect));
    observation.titled(title).expect("the window").clone()
}

#[test]
fn a_committed_buffer_is_drawn_where_the_window_is() {
    let session = Session::start(Headless::new());
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let window = client.create_window("Red");
        client.attach_solid(&window, 100, 100, RED);
        let observation = session.wait_for(|observation| {
            observation
                .only()
                .is_some_and(|window| window.mapped && window.geometry.width >= 100)
        });
        let geometry = observation.only().expect("one window").geometry;

        // The probe is the centre of the window's frame rectangle, which the 100x100
        // buffer fills from the window's origin.
        let inside = centre(Rect::new(
            geometry.x,
            geometry.y,
            geometry.width.min(100),
            geometry.height.min(100),
        ));
        assert_eq!(
            session.probe(inside.x, inside.y),
            RED,
            "the window's own pixels are drawn at the window's position"
        );

        // The far corner of the output is not covered by the window.
        let (width, height) = screen();
        assert_eq!(
            session.probe(width - 1, height - 1),
            BACKGROUND,
            "the desktop background is drawn where no window is"
        );
    });
}

#[test]
fn a_stride_padded_buffer_is_read_from_the_right_offsets() {
    let session = Session::start(Headless::new());
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let window = client.create_window("Padded");
        // 100 pixels of content in a 416-byte row: a renderer that read the buffer as
        // 400 bytes per row would show the damage as a diagonal smear.
        client.attach_with_stride(&window, 100, 100, 416, RED);
        let observation = session.wait_for(|observation| {
            observation
                .only()
                .is_some_and(|window| window.mapped && window.geometry.width >= 100)
        });
        let geometry = observation.only().expect("one window").geometry;
        let left = geometry.x + 1;
        let top = geometry.y + 1;
        let right = geometry.x + 99;
        let bottom = geometry.y + 99;
        assert_eq!(session.probe(left, top), RED, "the first pixel of the buffer");
        assert_eq!(session.probe(right, bottom), RED, "the last pixel of the buffer");
    });
}

#[test]
fn moving_a_window_moves_its_pixels_and_clears_the_old_rectangle() {
    let session = Session::start(Headless::new());
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let window = client.create_window("Moving");
        client.attach_solid(&window, 100, 100, RED);
        let observation = session.wait_for(|observation| {
            observation
                .only()
                .is_some_and(|window| window.mapped && window.geometry.width >= 100)
        });
        let id = observation.only().expect("one window").id;
        // A client that stops acknowledging keeps the state it has, so the move
        // produces exactly one frame: the one the test measures.
        client.state.ack = false;

        let before = Rect::new(40, 40, 120, 90);
        session.control.move_window(id, before);
        wait_for_rect(session, "Moving", before);
        assert_eq!(
            session.probe(centre(before).x, centre(before).y),
            RED,
            "the window is drawn at its new rectangle"
        );

        let after = Rect::new(300, 260, 120, 90);
        session.control.move_window(id, after);
        wait_for_rect(session, "Moving", after);
        assert_eq!(
            session.probe(centre(after).x, centre(after).y),
            RED,
            "the pixels moved with the window"
        );
        assert_eq!(
            session.probe(centre(before).x, centre(before).y),
            BACKGROUND,
            "the rectangle the window left is repainted with the background"
        );

        // The damage of the frame the move drew covers both rectangles and nothing
        // else, which is what "only the changed pixels are repainted" means.
        let stats = session.stats();
        let damage = stats.damage;
        assert_eq!(damage.len(), 2, "the old and the new rectangle: {damage:?}");
        for rect in [before, after] {
            assert!(
                damage
                    .iter()
                    .any(|damaged| damaged.contains(gnoblin_types::Point::new(rect.x, rect.y))),
                "the damage covers {rect:?}: {damage:?}"
            );
        }
    });
}

#[test]
fn a_half_transparent_window_blends_with_the_background() {
    let session = Session::start(Headless::new());
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let window = client.create_window("Faded");
        client.attach_solid(&window, 100, 100, RED);
        let observation = session.wait_for(|observation| {
            observation
                .only()
                .is_some_and(|window| window.mapped && window.geometry.width >= 100)
        });
        let id = observation.only().expect("one window").id;
        let geometry = observation.only().expect("one window").geometry;
        let inside = centre(Rect::new(
            geometry.x,
            geometry.y,
            geometry.width.min(100),
            geometry.height.min(100),
        ));
        assert_eq!(session.probe(inside.x, inside.y), RED, "the window starts opaque");

        session.control.set_window_opacity(id, 0.5);
        // Red at half alpha over an opaque black background is half red: the blue and
        // green channels stay dark and red lands at half intensity.
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let [red, green, blue, alpha] = session.probe(inside.x, inside.y);
            if red.abs_diff(128) <= 2 && green == 0 && blue == 0 && alpha == 255 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "half opacity never blended: the probe is {red} {green} {blue} {alpha}"
            );
            thread::sleep(Duration::from_millis(5));
        }
    });
}

#[test]
fn overlapping_windows_are_drawn_in_stacking_order() {
    let session = Session::start(Headless::new());
    session.run(|session| {
        let mut first = Client::connect(session.socket());
        let red_window = first.create_window("Red");
        first.attach_solid(&red_window, 100, 100, RED);
        let observation = session.wait_for(|observation| {
            observation
                .only()
                .is_some_and(|window| window.mapped && window.geometry.width >= 100)
        });
        let red_id = observation.only().expect("the first window").id;

        let mut second = Client::connect(session.socket());
        let blue_window = second.create_window("Blue");
        second.attach_solid(&blue_window, 100, 100, BLUE);
        let observation = session.wait_for(|observation| observation.len() == 2);
        let blue_id = observation.titled("Blue").expect("the second window").id;

        // Two overlapping rectangles, the second window on top and raised by its own
        // map. The overlap is where stacking is visible.
        let red = Rect::new(40, 40, 100, 100);
        let blue = Rect::new(80, 80, 100, 100);
        session.control.move_window(red_id, red);
        session.control.move_window(blue_id, blue);
        wait_for_rect(session, "Red", red);
        wait_for_rect(session, "Blue", blue);
        assert_eq!(session.probe(120, 120), BLUE, "the topmost window owns the overlap");
        assert_eq!(session.probe(60, 60), RED, "the lower window shows where it is alone");

        // A press on the lower window's exposed part is an activation request, and the
        // window manager raises it.
        first.click(session, gnoblin_types::Point::new(60, 60));
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if session.probe(120, 120) == RED {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the clicked window was never raised above the other"
            );
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            session.probe(140, 140),
            BLUE,
            "the other window still shows where the raised one does not cover it"
        );
    });
}

#[test]
fn a_window_without_a_buffer_is_not_drawn() {
    let session = Session::start(Headless::new());
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let window = client.create_window("Empty");
        let observation = session.wait_for(|observation| observation.only().is_some());
        let managed = observation.only().expect("one window");
        assert!(!managed.mapped, "a window with no buffer is not mapped");
        let geometry = managed.geometry;
        let probe = centre(Rect::new(
            geometry.x,
            geometry.y,
            geometry.width.min(100),
            geometry.height.min(100),
        ));
        assert_eq!(
            session.probe(probe.x, probe.y),
            BACKGROUND,
            "nothing is drawn for a window that has attached nothing"
        );
        // The session still serves the client: it can attach a buffer afterwards.
        client.attach_solid(&window, 100, 100, RED);
        session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        assert_eq!(
            session.probe(probe.x, probe.y),
            RED,
            "and the window is drawn once it has one"
        );
    });
}

#[test]
fn a_client_that_destroys_its_buffer_does_not_stop_the_frame() {
    let session = Session::start(Headless::new());
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let window = client.create_window("Gone");
        client.attach_solid(&window, 100, 100, RED);
        let observation = session.wait_for(|observation| {
            observation
                .only()
                .is_some_and(|window| window.mapped && window.geometry.width >= 100)
        });
        let geometry = observation.only().expect("one window").geometry;
        let probe = centre(Rect::new(
            geometry.x,
            geometry.y,
            geometry.width.min(100),
            geometry.height.min(100),
        ));
        assert_eq!(session.probe(probe.x, probe.y), RED);

        // A client may destroy the buffer the compositor still holds. The reference
        // the compositor keeps is what makes the memory stay mapped, so the frame is
        // still drawn from it: nothing reads freed memory and nothing goes stale.
        let id = session.windows().only().expect("the window").id;
        client.state.ack = false;
        let moved = Rect::new(200, 160, 120, 90);
        session.control.move_window(id, moved);
        wait_for_rect(session, "Gone", moved);
        assert_eq!(
            session.probe(centre(moved).x, centre(moved).y),
            RED,
            "the frame is still drawn from the buffer the client released"
        );
        assert_eq!(
            session.probe(probe.x, probe.y),
            BACKGROUND,
            "the rectangle the window left is repainted"
        );

        // A new buffer replaces it, and the destroyed one is not read again: buffers
        // are imported per frame rather than cached across commits.
        client.attach_solid(&window, 100, 100, BLUE);
        client.destroy_first_buffer();
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if session.probe(centre(moved).x, centre(moved).y) == BLUE {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the second buffer was never drawn: the first one is still being read"
            );
            thread::sleep(Duration::from_millis(5));
        }
        let (width, height) = screen();
        assert_eq!(
            session.probe(width - 1, height - 1),
            BACKGROUND,
            "the rest of the output is unchanged"
        );
    });
}

#[test]
fn an_idle_session_draws_no_frames() {
    let session = Session::start(Headless::new());
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let window = client.create_window("Still");
        client.attach_solid(&window, 100, 100, RED);
        session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        // Let the map settle: the assertion is about the iterations after it.
        thread::sleep(Duration::from_millis(200));
        let settled = session.stats();
        thread::sleep(Duration::from_millis(400));
        let idle = session.stats();
        assert!(
            idle.iterations > settled.iterations,
            "the loop kept iterating: {} then {}",
            settled.iterations,
            idle.iterations
        );
        assert_eq!(
            idle.frames, settled.frames,
            "no iteration drew a frame while nothing changed"
        );
        assert_eq!(
            idle.damage, settled.damage,
            "an idle iteration keeps the last frame's damage and adds none"
        );
    });
}

#[test]
fn a_second_output_draws_its_own_extent() {
    let mut backend = Headless::new();
    backend.push_output(
        "HEADLESS-2",
        gnoblin_backend::headless::DEFAULT_MODE,
        gnoblin_types::Scale::integral(1),
    );
    let session = Session::start(backend);
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let window = client.create_window("Two");
        client.attach_solid(&window, 100, 100, RED);
        session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        let outputs = session.windows().outputs().to_vec();
        assert_eq!(outputs.len(), 2, "the session publishes two outputs");
        // The window is placed on the first output, so the second's buffer holds the
        // background only: each output composes its own extent.
        let second = session
            .control
            .screenshot(outputs[1])
            .expect("the loop answers a screenshot request");
        assert_eq!(
            second.colour_at(0, 0),
            Some(BACKGROUND),
            "the second output paints its own background"
        );
        let first = session
            .control
            .screenshot(outputs[0])
            .expect("the loop answers a screenshot request");
        let (pixels, _) = first.data().as_chunks::<4>();
        assert!(pixels.contains(&RED), "the window is drawn on the first output");
    });
}

// The interfaces this client binds but only ever listens to.

wayland_client::delegate_noop!(App: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(App: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(App: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(App: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(App: ignore wl_buffer::WlBuffer);

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

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for App {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: <xdg_wm_base::XdgWmBase as Proxy>::Event,
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
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: <xdg_surface::XdgSurface as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event
            && state.ack
        {
            xdg_surface.ack_configure(serial);
            state.acks += 1;
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for App {
    fn event(
        _: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        _: <xdg_toplevel::XdgToplevel as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The toplevel's configure size and state are not what these tests assert on:
        // the compositor's own observation is.
    }
}

#[test]
fn a_window_id_is_never_reused_for_a_different_window() {
    // A window id names a scene node in the composer, so a reused id would move a new
    // window's node onto the old one's pixels. The session hands out a fresh id for
    // every toplevel, which this asserts while two clients come and go.
    let session = Session::start(Headless::new());
    session.run(|session| {
        let mut first = Client::connect(session.socket());
        let window = first.create_window("First");
        first.attach_solid(&window, 100, 100, RED);
        let observation = session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        let first_id: WindowId = observation.only().expect("the first window").id;
        window.toplevel.destroy();
        window.xdg_surface.destroy();
        window.surface.destroy();
        first.roundtrip();
        session.wait_for(|observation| observation.is_empty());

        let mut second = Client::connect(session.socket());
        let window = second.create_window("Second");
        second.attach_solid(&window, 100, 100, BLUE);
        let observation = session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        let second_id = observation.only().expect("the second window").id;
        assert_ne!(
            first_id, second_id,
            "a closed window's identity is not handed out again"
        );
        let geometry = observation.only().expect("the second window").geometry;
        let inside = centre(Rect::new(
            geometry.x,
            geometry.y,
            geometry.width.min(100),
            geometry.height.min(100),
        ));
        assert_eq!(
            session.probe(inside.x, inside.y),
            BLUE,
            "the new window is drawn, not the old one's pixels"
        );
    });
}
