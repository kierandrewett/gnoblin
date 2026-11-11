//! The surface tree on the screen: a real client's popups and subsurfaces, drawn by the real compositor.
//!
//! A client's menus, tooltips and combo boxes are `xdg_popup`s, and its video and GL widgets are `wl_subsurface`s.
//! The compositor used to draw one buffer per window, so none of them reached an output. Every test here maps a
//! window, adds such a surface, and reads the pixels the output shows through [`ControlHandle::screenshot`]: the
//! same readback `tests/render.rs` uses, so a probe is the colour an output has at a point and not a proxy for it.
//!
//! The cases are the failure-mode list at the top of `src/surface_tree.rs`, one to an item, and the composer's own
//! pixel tests (`src/render/surface_tree_tests.rs`) cover the same ordering without a client.
//!
//! ## Both renderers, on purpose
//!
//! The ordering and placement tests run on the software path and on the GLES path, as `tests/render.rs` does: a
//! popup hanging past a window's edge, or a layer behind its parent, is the kind of thing one renderer can get wrong
//! that the frame the composer describes does not show.

use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::{Headless, HeadlessConfig};
use gnoblin_types::{OutputId, Rect};
use gnoblin_wayland::{Config, ControlHandle, LoopError, Server, SessionSnapshot};
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_subcompositor, wl_subsurface, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::xdg::shell::client::{xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base};

/// How long a test waits for the session to reach a state before failing.
const TIMEOUT: Duration = Duration::from_secs(10);

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const BACKGROUND: [u8; 4] = [0, 0, 0, 255];

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

    fn windows(&self) -> SessionSnapshot {
        self.observation.lock().expect("the observation lock").clone()
    }

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

    fn output(&self) -> OutputId {
        *self
            .windows()
            .outputs()
            .first()
            .expect("a headless session starts with one output")
    }

    /// The colour at one point of the output now.
    fn probe(&self, x: i32, y: i32) -> [u8; 4] {
        self.control
            .screenshot(self.output())
            .expect("the loop answers a screenshot request")
            .colour_at(x, y)
            .unwrap_or_else(|| panic!("the probe at ({x}, {y}) is inside the output"))
    }

    /// Waits until the output shows `expected` at a point, or fails with the colour it last showed.
    ///
    /// The client's commit has been answered when its round trip returns, but the frame that draws it may come a
    /// loop iteration later, so a probe that expects a change waits for it.
    fn expect(&self, x: i32, y: i32, expected: [u8; 4], what: &str) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let shown = self.probe(x, y);
            if shown == expected {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{what}: the output shows {shown:?} at ({x}, {y}), expected {expected:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Asserts the output keeps showing `expected` at a point for a moment, so a surface that should never be drawn
    /// is not drawn a frame late.
    fn expect_steadily(&self, x: i32, y: i32, expected: [u8; 4], what: &str) {
        let until = Instant::now() + Duration::from_millis(250);
        while Instant::now() < until {
            assert_eq!(self.probe(x, y), expected, "{what} (at ({x}, {y}))");
            thread::sleep(Duration::from_millis(20));
        }
    }
}

struct StopOnDrop(ControlHandle);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.stop();
    }
}

struct Session {
    #[allow(dead_code)]
    directory: tempfile::TempDir,
    server: Server,
    handle: Handle,
}

impl Session {
    /// A session with motion off: the tests read what a window looks like once it has landed, and a window that maps
    /// on real time would still be fading in when a probe reads it.
    fn start(backend: Headless) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let socket = directory.path().join("wayland-0.sock");
        let config = Config {
            socket_path: Some(socket.clone()),
            ..Config::default()
        };
        let server = Server::new(Box::new(backend), config).expect("the session starts");
        server.control().set_reduced_motion(true);
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

    fn run(self, body: impl FnOnce(&Handle) + Send + 'static) {
        let handle = self.handle.clone();
        let worker = thread::spawn(move || {
            let _stop = StopOnDrop(handle.control.clone());
            body(&handle);
        });
        let outcome: Result<(), LoopError> = self.server.run();
        if let Err(payload) = worker.join() {
            std::panic::resume_unwind(payload);
        }
        outcome.expect("the loop stops cleanly");
    }
}

/// Runs `body` on every renderer this machine offers: the software path always, and the GLES path when a surfaceless
/// EGL context can be made.
fn for_each_renderer(body: impl Fn(Headless)) {
    body(Headless::new());
    match Headless::with_config(HeadlessConfig::gles()) {
        Ok(backend) => body(backend),
        Err(error) => println!("skipping the GLES half: {error} (no surfaceless EGL context on this machine)"),
    }
}

#[derive(Default)]
struct App {
    /// The position and size of the last popup configure, relative to the parent's window geometry.
    popup_placed: Option<(i32, i32, i32, i32)>,
}

/// The objects of one toplevel.
struct Window {
    surface: wl_surface::WlSurface,
    xdg_surface: xdg_surface::XdgSurface,
    #[allow(dead_code)]
    toplevel: xdg_toplevel::XdgToplevel,
}

/// The objects of one popup.
struct Popup {
    surface: wl_surface::WlSurface,
    xdg_surface: xdg_surface::XdgSurface,
    popup: xdg_popup::XdgPopup,
}

/// The objects of one subsurface.
struct Sub {
    surface: wl_surface::WlSurface,
    subsurface: wl_subsurface::WlSubsurface,
}

/// A real Wayland client for one connection.
struct Client {
    #[allow(dead_code)]
    connection: Connection,
    queue: EventQueue<App>,
    state: App,
    qh: QueueHandle<App>,
    compositor: wl_compositor::WlCompositor,
    subcompositor: wl_subcompositor::WlSubcompositor,
    shm: wl_shm::WlShm,
    wm_base: xdg_wm_base::XdgWmBase,
    /// The buffers the client has attached, kept so their files stay mapped.
    buffers: Vec<(wl_buffer::WlBuffer, wl_shm_pool::WlShmPool)>,
    #[allow(dead_code)]
    globals: GlobalList,
}

impl Client {
    fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("the session is listening");
        let connection = Connection::from_socket(stream).expect("a client connection");
        let (globals, queue) = registry_queue_init::<App>(&connection).expect("the global list");
        let qh = queue.handle();
        let compositor = globals.bind(&qh, 1..=1, ()).expect("wl_compositor");
        let subcompositor = globals.bind(&qh, 1..=1, ()).expect("wl_subcompositor");
        let shm = globals.bind(&qh, 1..=1, ()).expect("wl_shm");
        let wm_base = globals.bind(&qh, 1..=1, ()).expect("xdg_wm_base");
        Self {
            connection,
            queue,
            state: App::default(),
            qh,
            compositor,
            subcompositor,
            shm,
            wm_base,
            buffers: Vec::new(),
            globals,
        }
    }

    fn roundtrip(&mut self) {
        self.queue.roundtrip(&mut self.state).expect("a round trip");
    }

    /// A buffer of `width` by `height` filled with `colour`, in the order `wl_shm` stores `argb8888`.
    fn buffer(&mut self, width: i32, height: i32, colour: [u8; 4]) -> wl_buffer::WlBuffer {
        let stride = width * 4;
        let length = stride * height;
        let mut file = tempfile::tempfile().expect("a shared memory file");
        file.set_len(length as u64).expect("a buffer that big");
        let [red, green, blue, alpha] = colour;
        let pixels: Vec<u8> = [blue, green, red, alpha].repeat((width * height) as usize);
        file.write_all(&pixels).expect("the buffer's pixels");
        let pool = self.shm.create_pool(file.as_fd(), length, &self.qh, ());
        let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Argb8888, &self.qh, ());
        self.buffers.push((buffer.clone(), pool));
        buffer
    }

    /// Attaches a `width` by `height` buffer of `colour` to `surface` and commits.
    fn paint(&mut self, surface: &wl_surface::WlSurface, width: i32, height: i32, colour: [u8; 4]) {
        let buffer = self.buffer(width, height, colour);
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, width, height);
        surface.commit();
        self.roundtrip();
    }

    /// Creates a toplevel, takes its first configure and maps it with a `width` by `height` buffer of `colour`.
    fn map_window(&mut self, title: &str, width: i32, height: i32, colour: [u8; 4]) -> Window {
        let surface = self.compositor.create_surface(&self.qh, ());
        let xdg_surface = self.wm_base.get_xdg_surface(&surface, &self.qh, ());
        let toplevel = xdg_surface.get_toplevel(&self.qh, ());
        toplevel.set_app_id("org.gnoblin.SurfaceTree".to_owned());
        toplevel.set_title(title.to_owned());
        surface.commit();
        self.roundtrip();
        self.paint(&surface, width, height, colour);
        Window {
            surface,
            xdg_surface,
            toplevel,
        }
    }

    /// Maps a popup of `parent` whose top left corner is at (`x`, `y`) in the parent's window geometry, `size` big
    /// and filled with `colour`.
    fn map_popup(
        &mut self,
        parent: &xdg_surface::XdgSurface,
        at: (i32, i32),
        size: (i32, i32),
        colour: [u8; 4],
    ) -> Popup {
        let surface = self.compositor.create_surface(&self.qh, ());
        let xdg_surface = self.wm_base.get_xdg_surface(&surface, &self.qh, ());
        let positioner = self.wm_base.create_positioner(&self.qh, ());
        positioner.set_size(size.0, size.1);
        positioner.set_anchor_rect(at.0, at.1, 1, 1);
        positioner.set_anchor(xdg_positioner::Anchor::TopLeft);
        positioner.set_gravity(xdg_positioner::Gravity::BottomRight);
        let popup = xdg_surface.get_popup(Some(parent), &positioner, &self.qh, ());
        positioner.destroy();
        surface.commit();
        self.roundtrip();
        assert_eq!(
            self.state.popup_placed,
            Some((at.0, at.1, size.0, size.1)),
            "the compositor placed the popup where it asked"
        );
        self.paint(&surface, size.0, size.1, colour);
        Popup {
            surface,
            xdg_surface,
            popup,
        }
    }

    /// Maps a subsurface of `parent` at (`x`, `y`) in it, `size` big and filled with `colour`, then commits the parent
    /// so the subsurface's state is applied whatever its mode.
    fn map_subsurface(
        &mut self,
        parent: &wl_surface::WlSurface,
        at: (i32, i32),
        size: (i32, i32),
        colour: [u8; 4],
    ) -> Sub {
        let surface = self.compositor.create_surface(&self.qh, ());
        let subsurface = self.subcompositor.get_subsurface(&surface, parent, &self.qh, ());
        subsurface.set_position(at.0, at.1);
        self.paint(&surface, size.0, size.1, colour);
        parent.commit();
        self.roundtrip();
        Sub { surface, subsurface }
    }
}

/// The window's frame rectangle once it is mapped. The window's buffer is drawn from its top left corner.
fn mapped_origin(session: &Handle) -> (i32, i32) {
    let observation = session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
    let geometry: Rect = observation.only().expect("one window").geometry;
    (geometry.x, geometry.y)
}

/// A window of `size` and the origin it was placed at.
fn place_window(session: &Handle, client: &mut Client, size: i32) -> (Window, i32, i32) {
    let window = client.map_window("Tree", size, size, RED);
    let (x, y) = mapped_origin(session);
    session.expect(x + size / 2, y + size / 2, RED, "the window is drawn");
    (window, x, y)
}

#[test]
fn a_popup_hanging_past_its_window_is_drawn_above_it() {
    for_each_renderer(|backend| {
        let session = Session::start(backend);
        session.run(|session| {
            let mut client = Client::connect(session.socket());
            let (window, x, y) = place_window(session, &mut client, 100);
            // The popup's corner is 10 pixels inside the window's bottom right corner: it overlaps the window and
            // hangs past its edge.
            let _popup = client.map_popup(&window.xdg_surface, (90, 90), (40, 40), BLUE);

            session.expect(x + 95, y + 95, BLUE, "the popup is above the window where they overlap");
            session.expect(x + 115, y + 115, BLUE, "the popup is drawn past the window's edge");
            assert_eq!(session.probe(x + 50, y + 50), RED, "the window is still there");
            assert_eq!(
                session.probe(x + 85, y + 85),
                RED,
                "the window shows up to the popup's corner"
            );
            assert_eq!(
                session.probe(x + 131, y + 131),
                BACKGROUND,
                "and nothing past the popup's"
            );
            assert_eq!(
                session.windows().len(),
                1,
                "a popup is drawn with its window and is not a window of its own"
            );
        });
    });
}

#[test]
fn a_destroyed_popup_leaves_the_screen() {
    for_each_renderer(|backend| {
        let session = Session::start(backend);
        session.run(|session| {
            let mut client = Client::connect(session.socket());
            let (window, x, y) = place_window(session, &mut client, 100);
            let popup = client.map_popup(&window.xdg_surface, (90, 90), (40, 40), BLUE);
            session.expect(x + 115, y + 115, BLUE, "the popup is drawn");

            popup.popup.destroy();
            popup.xdg_surface.destroy();
            popup.surface.destroy();
            client.roundtrip();
            session.expect(x + 115, y + 115, BACKGROUND, "the popup is gone from the output");
            session.expect(x + 95, y + 95, RED, "and the window it covered shows again");
        });
    });
}

#[test]
fn a_popup_that_commits_again_is_drawn_with_its_new_pixels() {
    let session = Session::start(Headless::new());
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let (window, x, y) = place_window(session, &mut client, 100);
        let popup = client.map_popup(&window.xdg_surface, (90, 90), (40, 40), BLUE);
        session.expect(x + 115, y + 115, BLUE, "the popup is drawn");

        client.paint(&popup.surface, 40, 40, GREEN);
        session.expect(x + 115, y + 115, GREEN, "a popup's commit repaints where it is");
    });
}

#[test]
fn a_submenu_is_drawn_beside_its_menu() {
    let session = Session::start(Headless::new());
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let (window, x, y) = place_window(session, &mut client, 100);
        let menu = client.map_popup(&window.xdg_surface, (90, 0), (60, 60), GREEN);
        // The submenu hangs from the menu: its position is in the menu's window geometry.
        let _submenu = client.map_popup(&menu.xdg_surface, (50, 10), (40, 40), BLUE);

        session.expect(x + 105, y + 5, GREEN, "the menu is drawn past the window's edge");
        session.expect(x + 145, y + 15, BLUE, "the submenu is above the menu it hangs from");
        session.expect(x + 175, y + 25, BLUE, "and past it");
    });
}

#[test]
fn a_subsurface_is_drawn_above_its_parent_at_its_offset() {
    for_each_renderer(|backend| {
        let session = Session::start(backend);
        session.run(|session| {
            let mut client = Client::connect(session.socket());
            let (window, x, y) = place_window(session, &mut client, 100);
            let _video = client.map_subsurface(&window.surface, (80, 80), (40, 40), BLUE);

            session.expect(
                x + 90,
                y + 90,
                BLUE,
                "a subsurface is above its parent where they overlap",
            );
            session.expect(x + 110, y + 110, BLUE, "and is drawn past its edge");
            assert_eq!(session.probe(x + 50, y + 50), RED);
        });
    });
}

#[test]
fn a_subsurface_below_the_parent_is_drawn_behind_it() {
    for_each_renderer(|backend| {
        let session = Session::start(backend);
        session.run(|session| {
            let mut client = Client::connect(session.socket());
            let (window, x, y) = place_window(session, &mut client, 100);
            let video = client.map_subsurface(&window.surface, (80, 80), (40, 40), BLUE);
            session.expect(x + 90, y + 90, BLUE, "above the parent to begin with");

            video.subsurface.place_below(&window.surface);
            window.surface.commit();
            client.roundtrip();
            session.expect(x + 90, y + 90, RED, "the parent covers a subsurface placed below it");
            session.expect(x + 110, y + 110, BLUE, "which shows where the parent does not reach");
        });
    });
}

#[test]
fn place_above_and_place_below_reorder_sibling_subsurfaces() {
    let session = Session::start(Headless::new());
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let (window, x, y) = place_window(session, &mut client, 100);
        // Two subsurfaces beside the window that overlap each other.
        let first = client.map_subsurface(&window.surface, (120, 0), (40, 40), BLUE);
        let second = client.map_subsurface(&window.surface, (140, 20), (40, 40), GREEN);
        session.expect(x + 150, y + 30, GREEN, "the one created last is on top");

        second.subsurface.place_below(&first.surface);
        window.surface.commit();
        client.roundtrip();
        session.expect(x + 150, y + 30, BLUE, "place_below puts it under its sibling");

        second.subsurface.place_above(&first.surface);
        window.surface.commit();
        client.roundtrip();
        session.expect(x + 150, y + 30, GREEN, "and place_above puts it back");
    });
}

#[test]
fn a_synchronized_subsurface_shows_nothing_until_its_parent_commits() {
    let session = Session::start(Headless::new());
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let (window, x, y) = place_window(session, &mut client, 100);
        let video = client.map_subsurface(&window.surface, (120, 0), (40, 40), BLUE);
        session.expect(x + 140, y + 20, BLUE, "mapped");

        // A synchronized subsurface's commit is held until its parent commits.
        client.paint(&video.surface, 40, 40, GREEN);
        session.expect_steadily(x + 140, y + 20, BLUE, "a synchronized subsurface's commit waits");
        window.surface.commit();
        client.roundtrip();
        session.expect(x + 140, y + 20, GREEN, "the parent's commit applies it");

        // A desynchronized one shows its own commits.
        video.subsurface.set_desync();
        window.surface.commit();
        client.roundtrip();
        client.paint(&video.surface, 40, 40, BLUE);
        session.expect(x + 140, y + 20, BLUE, "a desynchronized subsurface draws as it commits");
    });
}

#[test]
fn a_subsurface_without_a_buffer_hides_what_hangs_from_it() {
    let session = Session::start(Headless::new());
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let (window, x, y) = place_window(session, &mut client, 100);
        let child = client.map_subsurface(&window.surface, (120, 0), (40, 40), BLUE);
        let _grandchild = client.map_subsurface(&child.surface, (30, 0), (40, 40), GREEN);
        child.surface.commit();
        window.surface.commit();
        client.roundtrip();
        session.expect(x + 125, y + 20, BLUE, "the child is drawn");
        session.expect(x + 165, y + 20, GREEN, "and the surface that hangs from it");

        child.surface.attach(None, 0, 0);
        child.surface.commit();
        window.surface.commit();
        client.roundtrip();
        session.expect(x + 125, y + 20, BACKGROUND, "a subsurface with no buffer is not drawn");
        session.expect(x + 165, y + 20, BACKGROUND, "and nor is what hangs from it");
    });
}

#[test]
fn a_subsurface_that_is_destroyed_leaves_the_screen() {
    let session = Session::start(Headless::new());
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let (window, x, y) = place_window(session, &mut client, 100);
        let video = client.map_subsurface(&window.surface, (120, 0), (40, 40), BLUE);
        session.expect(x + 140, y + 20, BLUE, "drawn");

        video.subsurface.destroy();
        video.surface.destroy();
        client.roundtrip();
        session.expect(x + 140, y + 20, BACKGROUND, "destroyed, so gone");
    });
}

// The interfaces this client binds but only ever listens to.

wayland_client::delegate_noop!(App: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(App: ignore wl_subcompositor::WlSubcompositor);
wayland_client::delegate_noop!(App: ignore wl_subsurface::WlSubsurface);
wayland_client::delegate_noop!(App: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(App: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(App: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(App: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(App: ignore xdg_positioner::XdgPositioner);
wayland_client::delegate_noop!(App: ignore xdg_toplevel::XdgToplevel);

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
        _: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: <xdg_surface::XdgSurface as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
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
        if let xdg_popup::Event::Configure { x, y, width, height } = event {
            state.popup_placed = Some((x, y, width, height));
        }
    }
}
