//! A `view`-role client reads a View's pixels from the compositor, end to end.
//!
//! The owner arranges a scene in-process through the tree queue, the way the shell does:
//! a shell-owned node holds a client's window, and a `view` node draws that node's
//! subtree. A client with the `view` role then binds `gnoblin_view_source_manager_v1`,
//! names the View, and reads it through `ext_image_copy_capture_v1` into its own `wl_shm`
//! buffer. The tests assert the pixels that arrive, that a recoloured window is delivered
//! again, and that the session lock stops the stream.
//!
//! The window's client and the lock's client are ordinary connections. The `view`-role client
//! is the only one that holds the role, which the test harness grants per connection: it
//! binds `wl_shm`, the view source manager and the copy manager and nothing else, because
//! those are the only globals the role is shown. A capture that passes here passes with the
//! restricted role, not with a role that merely sits beside the globals it uses.

mod owned;

use std::fs::File;
use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_render::Scene;
use gnoblin_render::transaction::{ClassInfo, Code, PropertyInfo, TreeOp, TreeTransaction, TreeValue, ValueType};
use gnoblin_types::{NodeId, Rect};
use gnoblin_wayland::{ClientRole, TreeHooks, TreeQueue};
use owned::protocols::gnoblin_view_source_v1::ext_image_capture_source_v1::ExtImageCaptureSourceV1;
use owned::protocols::gnoblin_view_source_v1::gnoblin_view_source_manager_v1::GnoblinViewSourceManagerV1;
use owned::{Bare, Connected, Handle, Session, TIMEOUT};
use wayland_client::globals::GlobalListContents;
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool, wl_surface};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1, ext_image_copy_capture_manager_v1, ext_image_copy_capture_session_v1,
};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_surface_v1, ext_session_lock_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// The window's first colour, opaque, as a native-endian `argb8888` word.
const SOURCE: u32 = 0xFF20_40C0;

/// The colour the window is repainted with.
const REPAINTED: u32 = 0xFFC0_4020;

/// The side of the window and of the View, in pixels.
const SIDE: u32 = 64;

/// The shell-owned node the window sits under, and the View's `source`.
const PARENT: NodeId = NodeId::new(0x100);

/// A shell-owned node between `PARENT` and the window that shifts the window to the origin
/// of its source, so the View's top left is the window's top left wherever the layout put it.
const PLACE: NodeId = NodeId::new(0x101);

/// The View. Its id carries an owner epoch in the high half, as every shell node's does, so a capture source that
/// could name only a 32-bit id could not name it.
const VIEW: NodeId = NodeId::new((1 << 32) | 0x102);

/// The schema the tree checks the owner's transactions against: the three classes the test
/// builds with, and the properties it writes. The names are the schema's (`tree/view.toml`).
struct Classes;

impl ClassInfo for Classes {
    fn has_class(&self, class: &str) -> bool {
        matches!(class, "node" | "window" | "view")
    }

    fn is_a(&self, class: &str, ancestor: &str) -> bool {
        class == ancestor || (ancestor == "node" && matches!(class, "window" | "view"))
    }

    fn property(&self, class: &str, key: &str) -> Option<PropertyInfo> {
        let kind = match (class, key) {
            (_, "x" | "y" | "width" | "height") => ValueType::Double,
            (_, "visible") => ValueType::Bool,
            ("view", "source" | "camera") => ValueType::Node("node".to_owned()),
            _ => return None,
        };
        Some(PropertyInfo {
            kind,
            readonly: false,
            animated: false,
        })
    }

    fn has_method(&self, _class: &str, _method: &str) -> bool {
        false
    }
}

/// The owner: it reads and writes the live scene through the tree queue, in-process.
struct Owner {
    queue: TreeQueue,
}

impl Owner {
    /// Runs `read` against the live scene at the start of the next frame, and answers what
    /// it returned with the serial of the last published batch.
    fn read<T: Send + 'static>(&self, read: impl FnOnce(&mut Scene) -> T + Send + 'static) -> (T, u64) {
        let (sender, receiver) = mpsc::channel();
        self.queue.read(move |scene, serial| {
            let _ = sender.send((read(scene), serial));
        });
        receiver.recv_timeout(TIMEOUT).expect("the frame loop ran the read")
    }

    /// The window the compositor made a node for, its place on the output, and the scene's
    /// root. The node is made on the frame after the window maps, so this waits for it.
    fn window(&self) -> (NodeId, Rect, NodeId) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let (found, _) = self.read(|scene| {
                let root = scene.root()?;
                let mut stack: Vec<NodeId> = scene.root_ids().to_vec();
                while let Some(node) = stack.pop() {
                    if scene.class(node) == Some("window") {
                        return Some((node, scene.absolute_bounds(node)?, root));
                    }
                    stack.extend(scene.children(node).unwrap_or(&[]));
                }
                None
            });
            if let Some(window) = found {
                return window;
            }
            assert!(
                Instant::now() < deadline,
                "the compositor never made a node for the window"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Applies `ops` as the shell and waits for the frame that answers them. A transaction
    /// built from a stale read is refused `stale-base` when a node it names changed since, so
    /// it is built again from a fresh serial; any other refusal fails the test.
    fn transact(&self, ops: impl Fn() -> Vec<TreeOp>) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let (_, serial) = self.read(|_| ());
            let (sender, receiver) = mpsc::channel();
            self.queue.submit(
                serial,
                TreeTransaction::new(ops()),
                || true,
                move |_, result| {
                    let _ = sender.send(result.map(|_| ()));
                },
            );
            match receiver
                .recv_timeout(TIMEOUT)
                .expect("the frame loop answered the transaction")
            {
                Ok(()) => return,
                Err(refusal) if refusal.code == Code::StaleBase && Instant::now() < deadline => {}
                Err(refusal) => panic!("the owner's transaction was refused: {refusal}"),
            }
        }
    }

    /// Lets the frame loop run `frames` whole iterations, so what an earlier frame applied
    /// has been published to capture.
    fn frames(&self, frames: usize) {
        for _ in 0..frames {
            self.read(|_| ());
        }
    }
}

/// What a client records of the events it received.
#[derive(Default)]
struct App {
    /// The lock's `locked` events.
    locked: u32,
    /// The size of the lock surface's last configure.
    lock_size: Option<(u32, u32)>,
    /// The size the capture session asked for.
    session_size: Option<(u32, u32)>,
    /// Every `wl_shm` format the capture session offered, as the protocol's number.
    session_formats: Vec<u32>,
    /// The capture session's `done` events, which close a set of constraints.
    session_done: u32,
    /// The capture session's `stopped` events.
    session_stopped: u32,
    /// Frames that were ready, and frames that failed.
    frame_ready: u32,
    frame_failed: u32,
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

impl Dispatch<ext_session_lock_v1::ExtSessionLockV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &ext_session_lock_v1::ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_v1::Event::Locked = event {
            state.locked += 1;
        }
    }
}

impl Dispatch<ext_session_lock_surface_v1::ExtSessionLockSurfaceV1, ()> for App {
    fn event(
        state: &mut Self,
        surface: &ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure { serial, width, height } = event {
            state.lock_size = Some((width, height));
            surface.ack_configure(serial);
        }
    }
}

impl Dispatch<ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                state.session_size = Some((width, height));
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
        event: ext_image_copy_capture_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_frame_v1::Event::Ready => state.frame_ready += 1,
            ext_image_copy_capture_frame_v1::Event::Failed { .. } => state.frame_failed += 1,
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
wayland_client::delegate_noop!(App: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1);
wayland_client::delegate_noop!(App: ignore GnoblinViewSourceManagerV1);
wayland_client::delegate_noop!(App: ignore ExtImageCaptureSourceV1);
wayland_client::delegate_noop!(App: ignore ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1);

/// A shared-memory buffer the test fills before the compositor reads it, or reads after the
/// compositor wrote into it. The pixels are native-endian `argb8888` or `xrgb8888` words.
struct Pixels {
    file: File,
    buffer: wl_buffer::WlBuffer,
    width: u32,
    height: u32,
    format: wl_shm::Format,
}

impl Pixels {
    /// A buffer of `width` by `height` pixels in `format`, every pixel `colour`.
    fn solid(
        shm: &wl_shm::WlShm,
        qh: &QueueHandle<App>,
        width: u32,
        height: u32,
        format: wl_shm::Format,
        colour: u32,
    ) -> Self {
        let stride = width * 4;
        let length = (stride * height) as usize;
        let mut file = tempfile::tempfile().expect("a shared memory file");
        file.set_len(length as u64).expect("a file that big");
        let bytes: Vec<u8> = (0..width * height).flat_map(|_| colour.to_ne_bytes()).collect();
        file.write_all(&bytes).expect("the pixels");
        let pool = shm.create_pool(file.as_fd(), length as i32, qh, ());
        let buffer = pool.create_buffer(0, width as i32, height as i32, stride as i32, format, qh, ());
        Self {
            file,
            buffer,
            width,
            height,
            format,
        }
    }

    /// One pixel of the buffer, as a native-endian word.
    fn word(&self, x: u32, y: u32) -> u32 {
        let mut bytes = [0u8; 4];
        let offset = u64::from(y) * u64::from(self.width) * 4 + u64::from(x) * 4;
        self.file.read_exact_at(&mut bytes, offset).expect("a pixel");
        u32::from_ne_bytes(bytes)
    }

    /// The first pixel that is not `colour`, with where it is, or `None` when every pixel is.
    /// An `xrgb8888` buffer has no alpha, so its fourth byte is not compared.
    fn first_other_than(&self, colour: u32) -> Option<(u32, u32, u32)> {
        let mask = if self.format == wl_shm::Format::Argb8888 {
            u32::MAX
        } else {
            0x00FF_FFFF
        };
        (0..self.height)
            .flat_map(|y| (0..self.width).map(move |x| (x, y)))
            .map(|(x, y)| (x, y, self.word(x, y)))
            .find(|(_, _, word)| word & mask != colour & mask)
    }
}

/// The client that owns a toplevel.
struct Window {
    surface: wl_surface::WlSurface,
    /// The toplevel's objects and its buffers, kept so the client's pools outlive the window.
    #[allow(dead_code)]
    objects: (xdg_surface::XdgSurface, xdg_toplevel::XdgToplevel),
    buffers: Vec<Pixels>,
}

impl Window {
    /// Maps a toplevel whose buffer is `SIDE` by `SIDE` pixels of `colour`.
    fn map(client: &mut Connected<App>, colour: u32) -> Self {
        let wm_base: xdg_wm_base::XdgWmBase = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the session advertises xdg_wm_base");
        let surface = client.compositor.create_surface(&client.qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, &client.qh, ());
        let toplevel = xdg_surface.get_toplevel(&client.qh, ());
        toplevel.set_title("view source".to_owned());
        surface.commit();
        client.roundtrip();
        let mut window = Self {
            surface,
            objects: (xdg_surface, toplevel),
            buffers: Vec::new(),
        };
        window.paint(client, colour);
        window
    }

    /// Commits a new buffer of `colour`.
    fn paint(&mut self, client: &mut Connected<App>, colour: u32) {
        let pixels = Pixels::solid(&client.shm, &client.qh, SIDE, SIDE, wl_shm::Format::Argb8888, colour);
        self.surface.attach(Some(&pixels.buffer), 0, 0);
        self.surface.damage(0, 0, SIDE as i32, SIDE as i32);
        self.surface.commit();
        self.buffers.push(pixels);
        client.roundtrip();
    }
}

/// The owner's scene: a shell parent node holding the window, and a View of that parent.
fn arrange(owner: &Owner) {
    let (window, placed, root) = owner.window();
    owner.transact(|| {
        let set = |node, key: &str, value| TreeOp::Set {
            node,
            key: key.to_owned(),
            value,
            timing: None,
        };
        let new = |class: &str, id| TreeOp::New {
            class: class.to_owned(),
            id,
        };
        let insert = |node, parent| TreeOp::Insert {
            node,
            parent,
            index: None,
        };
        vec![
            new("node", PARENT),
            insert(PARENT, root),
            new("node", PLACE),
            insert(PLACE, PARENT),
            // The window arrives unparented; the owner takes it under its own node and
            // shifts it so the window's top left is the source's origin.
            insert(window, PLACE),
            set(PLACE, "x", TreeValue::Double(-f64::from(placed.x))),
            set(PLACE, "y", TreeValue::Double(-f64::from(placed.y))),
            new("view", VIEW),
            insert(VIEW, root),
            set(VIEW, "width", TreeValue::Double(f64::from(SIDE))),
            set(VIEW, "height", TreeValue::Double(f64::from(SIDE))),
            set(VIEW, "source", TreeValue::Node(Some(PARENT))),
        ]
    });
    // The frame loop publishes the Views the scene holds once per iteration, and a View is
    // only offered as a source after that.
    owner.frames(3);
}

/// The `view`-role client and its capture session of the View.
struct Viewer {
    client: Bare<App>,
    session: ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1,
    /// The buffer the compositor writes frames into, in the format the session offered.
    target: Pixels,
    /// Kept so the source outlives the session that was made from it.
    #[allow(dead_code)]
    source: ExtImageCaptureSourceV1,
    /// Kept so the pool behind the target outlives it.
    _shm: wl_shm::WlShm,
}

impl Viewer {
    /// Connects with the `view` role, binds the capture globals, names the View, opens a
    /// session on it and allocates the buffer for the size and format the session advertises.
    ///
    /// The client binds `wl_shm`, the view source manager and the copy manager, which are all
    /// its registry holds: it has no compositor to make a surface with.
    fn open(handle: &Handle) -> Self {
        let mut client = handle.connect_bare(ClientRole::View, App::default());
        assert_eq!(
            client.interfaces(),
            [
                "ext_image_copy_capture_manager_v1",
                "gnoblin_view_source_manager_v1",
                "wl_shm"
            ],
            "the view role's registry is the capture globals and nothing else"
        );
        let shm: wl_shm::WlShm = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the view role binds wl_shm");
        let sources: GnoblinViewSourceManagerV1 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the view role binds the view source manager");
        let copies: ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the view role binds the image copy capture manager");
        let source = sources.create_source((VIEW.get() >> 32) as u32, VIEW.get() as u32, &client.qh, ());
        let session = copies.create_session(
            &source,
            ext_image_copy_capture_manager_v1::Options::empty(),
            &client.qh,
            (),
        );
        client.wait_until("the capture session's constraints", |state| state.session_done >= 1);
        assert_eq!(
            client.state.session_size,
            Some((SIDE, SIDE)),
            "the session is the View's size"
        );
        let format = [wl_shm::Format::Argb8888, wl_shm::Format::Xrgb8888]
            .into_iter()
            .find(|format| client.state.session_formats.contains(&(*format as u32)))
            .unwrap_or_else(|| {
                panic!(
                    "the session offers argb8888 or xrgb8888; it offered {:?}",
                    client.state.session_formats
                )
            });
        let target = Pixels::solid(&shm, &client.qh, SIDE, SIDE, format, 0);
        Self {
            client,
            session,
            target,
            source,
            _shm: shm,
        }
    }

    /// Asks for one frame into the target and waits for the compositor to answer it. `true`
    /// when the frame was delivered, `false` when it failed.
    fn capture(&mut self) -> bool {
        let (ready, failed) = (self.client.state.frame_ready, self.client.state.frame_failed);
        let frame = self.session.create_frame(&self.client.qh, ());
        frame.attach_buffer(&self.target.buffer);
        frame.damage_buffer(0, 0, SIDE as i32, SIDE as i32);
        frame.capture();
        self.client.wait_until("an answer to the frame request", |state| {
            state.frame_ready > ready || state.frame_failed > failed
        });
        let delivered = self.client.state.frame_ready > ready;
        frame.destroy();
        delivered
    }

    /// Captures frames until the target holds nothing but `colour`, and answers how many
    /// frames that took. A frame the compositor could not yet fill is asked for again; a
    /// frame request that is never answered fails the wait.
    fn capture_until(&mut self, colour: u32) -> u32 {
        let deadline = Instant::now() + TIMEOUT;
        let mut frames = 0;
        let mut last = String::from("no frame was delivered");
        while Instant::now() < deadline {
            frames += 1;
            if self.capture() {
                match self.target.first_other_than(colour) {
                    None => return frames,
                    Some((x, y, word)) => {
                        last = format!("pixel ({x}, {y}) is {word:#010x}, not {colour:#010x}");
                    }
                }
            } else {
                last = String::from("the last frame failed");
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("the capture never showed {colour:#010x}: {last}");
    }
}

/// A lock client and the objects that keep the lock up while it is held.
#[allow(dead_code)]
struct Locker {
    client: Connected<App>,
    lock: ext_session_lock_v1::ExtSessionLockV1,
    cover: Pixels,
}

/// Locks the session and covers its output, as a lock client does. The caller holds the
/// result for as long as the lock should stay up.
fn lock(socket: &std::path::Path) -> Locker {
    let mut client = Connected::connect(socket, App::default());
    let manager: ext_session_lock_manager_v1::ExtSessionLockManagerV1 = client
        .globals
        .bind(&client.qh, 1..=1, ())
        .expect("the session advertises ext_session_lock_manager_v1");
    let output: wl_output::WlOutput = client
        .globals
        .bind(&client.qh, 1..=4, ())
        .expect("the session advertises a wl_output");
    let lock = manager.lock(&client.qh, ());
    client.wait_until("`locked`", |state| state.locked >= 1);
    let surface = client.compositor.create_surface(&client.qh, ());
    let _lock_surface = lock.get_lock_surface(&surface, &output, &client.qh, ());
    client.wait_until("the lock surface's configure", |state| state.lock_size.is_some());
    let (width, height) = client.state.lock_size.expect("a configured lock surface");
    let cover = Pixels::solid(
        &client.shm,
        &client.qh,
        width,
        height,
        wl_shm::Format::Argb8888,
        0xFF00_0000,
    );
    surface.attach(Some(&cover.buffer), 0, 0);
    surface.damage(0, 0, width as i32, height as i32);
    surface.commit();
    client.roundtrip();
    Locker { client, lock, cover }
}

/// A session that carries the tree, and the owner's handle on its queue.
fn start() -> (Session, Owner) {
    let hooks = TreeHooks {
        classes: std::sync::Arc::new(Classes),
        ..TreeHooks::default()
    };
    let owner = Owner {
        queue: hooks.queue.clone(),
    };
    (Session::start_with_tree(None, hooks), owner)
}

#[test]
fn a_view_role_client_reads_the_pixels_of_a_view() {
    let (session, owner) = start();
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let _window = Window::map(&mut app, SOURCE);
        arrange(&owner);

        let mut viewer = Viewer::open(handle);
        viewer.capture_until(SOURCE);
        let last = SIDE - 1;
        for (x, y) in [(0, 0), (SIDE / 2, SIDE / 2), (last, 0), (0, last), (last, last)] {
            let word = viewer.target.word(x, y);
            assert_eq!(
                word & 0x00FF_FFFF,
                SOURCE & 0x00FF_FFFF,
                "pixel ({x}, {y}) of the capture is the window's colour"
            );
        }
    });
}

#[test]
fn a_repainted_window_is_delivered_again() {
    let (session, owner) = start();
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let mut window = Window::map(&mut app, SOURCE);
        arrange(&owner);

        let mut viewer = Viewer::open(handle);
        viewer.capture_until(SOURCE);

        // The client commits a buffer of another colour. The View redraws from it, the
        // session's next frame reports the change, and that frame holds the new colour.
        window.paint(&mut app, REPAINTED);
        viewer.capture_until(REPAINTED);
    });
}

#[test]
fn the_session_lock_stops_the_capture_of_a_view() {
    let (session, owner) = start();
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let _window = Window::map(&mut app, SOURCE);
        arrange(&owner);

        let mut viewer = Viewer::open(handle);
        viewer.capture_until(SOURCE);
        assert_eq!(
            viewer.client.state.session_stopped, 0,
            "the session runs before the lock"
        );

        let _locker = lock(handle.socket());
        viewer
            .client
            .wait_until("the capture session to stop", |state| state.session_stopped >= 1);

        // A stopped session answers no frame, and the buffer keeps what the client left in it.
        let ready = viewer.client.state.frame_ready;
        assert!(!viewer.capture(), "a stopped session delivers no frame");
        assert_eq!(viewer.client.state.frame_ready, ready, "no frame survived the lock");
    });
}
