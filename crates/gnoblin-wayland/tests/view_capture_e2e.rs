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

mod mock_gpu;
mod owned;

use std::fs::File;
use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_pipewire_export::testing::{Daemon, FrameConsumer};
use gnoblin_render::Scene;
use gnoblin_render::transaction::{ClassInfo, Code, PropertyInfo, TreeOp, TreeTransaction, TreeValue, ValueType};
use gnoblin_secret::SecretText;
use gnoblin_types::{NodeId, Rect};
use gnoblin_wayland::{ClientRole, Clock, ManualClock, Observer, SessionSnapshot, TreeAnswer, TreeHooks, TreeQueue};
use owned::protocols::gnoblin_view_source_v1::ext_image_capture_source_v1::ExtImageCaptureSourceV1;
use owned::protocols::gnoblin_view_source_v1::gnoblin_view_source_manager_v1::GnoblinViewSourceManagerV1;
use owned::{Bare, Connected, Extras, Handle, Session, TIMEOUT};
use wayland_client::globals::GlobalListContents;
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool, wl_surface};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1, ext_image_copy_capture_manager_v1, ext_image_copy_capture_session_v1,
};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_surface_v1, ext_session_lock_v1,
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1 as params, zwp_linux_dmabuf_v1 as dmabuf_global,
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
        matches!(class, "node" | "window" | "view" | "canvas" | "lock")
    }

    fn is_a(&self, class: &str, ancestor: &str) -> bool {
        class == ancestor || (ancestor == "node" && matches!(class, "window" | "view" | "canvas" | "lock"))
    }

    fn property(&self, class: &str, key: &str) -> Option<PropertyInfo> {
        let kind = match (class, key) {
            (_, "x" | "y" | "width" | "height") => ValueType::Double,
            (_, "secure") => ValueType::Bool,
            (_, "visible") => ValueType::Bool,
            ("canvas", "content") => ValueType::Bytes,
            ("view", "source" | "camera") => ValueType::Node("node".to_owned()),
            ("view", "fps") => ValueType::Uint,
            _ => return None,
        };
        Some(PropertyInfo {
            kind,
            readonly: false,
            animated: false,
        })
    }

    fn has_method(&self, class: &str, method: &str) -> bool {
        (class == "view" && method == "capture") || (class == "lock" && method == "authenticate")
    }
}

/// The owner: it reads and writes the live scene through the tree queue, in-process.
struct Owner {
    queue: TreeQueue,
    /// The answers the frames published, in order.
    answers: Arc<Mutex<Vec<TreeAnswer>>>,
    /// The next token a call is made under.
    token: AtomicU64,
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

    /// Calls `capture(path)` on `view` as the shell does and waits for the answer the compositor publishes.
    fn capture(&self, view: NodeId, path: &str) -> TreeAnswer {
        self.call(view, "capture", path)
    }

    /// Calls `method` on `node` with `text` as the call's text, and waits for the answer the compositor publishes.
    fn call(&self, node: NodeId, method: &str, text: &str) -> TreeAnswer {
        let token = self.token.fetch_add(1, Ordering::SeqCst) + 1;
        self.transact(|| {
            vec![TreeOp::TreeCall {
                node,
                method: method.to_owned(),
                args: Vec::new(),
                text: SecretText::new(text),
                token,
            }]
        });
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(answer) = self
                .answers
                .lock()
                .expect("the answers lock")
                .iter()
                .find(|answer| answer.token == token)
            {
                return answer.clone();
            }
            assert!(Instant::now() < deadline, "the {method} call was never answered");
            self.frames(1);
            thread::sleep(Duration::from_millis(5));
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
        Self::patterned(shm, qh, width, height, format, |_, _| colour)
    }

    /// A buffer of `width` by `height` pixels in `format`, pixel `(x, y)` the word `pattern(x, y)`.
    fn patterned(
        shm: &wl_shm::WlShm,
        qh: &QueueHandle<App>,
        width: u32,
        height: u32,
        format: wl_shm::Format,
        pattern: impl Fn(u32, u32) -> u32,
    ) -> Self {
        let stride = width * 4;
        let length = (stride * height) as usize;
        let mut file = tempfile::tempfile().expect("a shared memory file");
        file.set_len(length as u64).expect("a file that big");
        let mut bytes = Vec::with_capacity(length);
        for y in 0..height {
            for x in 0..width {
                bytes.extend_from_slice(&pattern(x, y).to_ne_bytes());
            }
        }
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

    /// Overwrites every pixel with `colour`, as a client clears a buffer before it asks for a frame.
    fn fill(&self, colour: u32) {
        let bytes: Vec<u8> = (0..self.width * self.height)
            .flat_map(|_| colour.to_ne_bytes())
            .collect();
        self.file.write_all_at(&bytes, 0).expect("the pixels");
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

    /// How many pixels are `colour`. An `xrgb8888` buffer has no alpha, so its fourth byte is not compared.
    fn count_of(&self, colour: u32) -> usize {
        let mask = if self.format == wl_shm::Format::Argb8888 {
            u32::MAX
        } else {
            0x00FF_FFFF
        };
        (0..self.height)
            .flat_map(|y| (0..self.width).map(move |x| (x, y)))
            .filter(|(x, y)| self.word(*x, *y) & mask == colour & mask)
            .count()
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

    /// Commits a buffer whose every pixel is its own [`gradient`] word, so a copy of any part of the window says
    /// where in the window it came from.
    fn paint_gradient(&mut self, client: &mut Connected<App>) {
        let pixels = Pixels::patterned(&client.shm, &client.qh, SIDE, SIDE, wl_shm::Format::Argb8888, gradient);
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
        Self::open_view(handle, VIEW)
    }

    /// [`Viewer::open`] for the View `view`.
    fn open_view(handle: &Handle, view: NodeId) -> Self {
        Self::open_as(handle, view, None)
    }

    /// [`Viewer::open_view`] with a buffer of `wanted` when one is named, the first format the session offers
    /// otherwise.
    fn open_as(handle: &Handle, view: NodeId, wanted: Option<wl_shm::Format>) -> Self {
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
        let source = sources.create_source((view.get() >> 32) as u32, view.get() as u32, &client.qh, ());
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
        let format = match wanted {
            Some(format) => {
                assert!(
                    client.state.session_formats.contains(&(format as u32)),
                    "the session offers {format:?}; it offered {:?}",
                    client.state.session_formats
                );
                format
            }
            None => [wl_shm::Format::Argb8888, wl_shm::Format::Xrgb8888]
                .into_iter()
                .find(|format| client.state.session_formats.contains(&(*format as u32)))
                .unwrap_or_else(|| {
                    panic!(
                        "the session offers argb8888 or xrgb8888; it offered {:?}",
                        client.state.session_formats
                    )
                }),
        };
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

/// What a test reads of the answers the frames publish, and the hooks that record them.
fn hooks() -> (TreeHooks, Arc<Mutex<Vec<TreeAnswer>>>) {
    let answers: Arc<Mutex<Vec<TreeAnswer>>> = Arc::default();
    let hooks = TreeHooks {
        classes: Arc::new(Classes),
        publish: Arc::new({
            let answers = Arc::clone(&answers);
            move |batch| answers.lock().expect("the answers lock").extend(batch.answers)
        }),
        ..TreeHooks::default()
    };
    (hooks, answers)
}

/// The owner's handle on a session's tree queue.
fn owner_of(hooks: &TreeHooks, answers: Arc<Mutex<Vec<TreeAnswer>>>) -> Owner {
    Owner {
        queue: hooks.queue.clone(),
        answers,
        token: AtomicU64::new(0),
    }
}

/// A session that carries the tree, and the owner's handle on its queue.
fn start() -> (Session, Owner) {
    let (hooks, answers) = hooks();
    let owner = owner_of(&hooks, answers);
    (Session::start_with_tree(None, hooks), owner)
}

/// A session that carries the tree, with an observer and a clock of the test's own.
fn start_with(observer: Option<Arc<dyn Observer>>, clock: Option<Arc<dyn Clock>>) -> (Session, Owner) {
    let (hooks, answers) = hooks();
    let owner = owner_of(&hooks, answers);
    let extras = Extras {
        observer,
        clock,
        ..Extras::default()
    };
    (Session::start_with_extras(None, hooks, extras), owner)
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

/// The paint list a titlebar-like canvas sends: a filled bar, a line of text and a close icon.
fn bar_paint_list() -> Vec<u8> {
    use gnoblin_paint::{PaintList, PaintOp, Rgba8};
    gnoblin_paint::encode(&PaintList {
        ops: vec![
            PaintOp::Rect {
                x: 0.0,
                y: 0.0,
                width: SIDE as f32,
                height: SIDE as f32,
                radius: 0.0,
                fill: Rgba8::new(0x20, 0x40, 0xC0, 0xFF),
                stroke: Rgba8::TRANSPARENT,
                stroke_width: 0.0,
            },
            PaintOp::Text {
                x: 4.0,
                y: 2.0,
                size: 16.0,
                colour: Rgba8::new(0xFF, 0xFF, 0xFF, 0xFF),
                font: "sans".to_owned(),
                text: "Hi".to_owned(),
            },
            PaintOp::Icon {
                x: 40.0,
                y: 4.0,
                size: 16.0,
                colour: Rgba8::new(0xFF, 0xFF, 0xFF, 0xFF),
                name: "window-close-symbolic".to_owned(),
            },
        ],
    })
}

/// The owner's scene: a shell parent node holding one canvas that paints [`bar_paint_list`], and a View of the parent.
fn arrange_canvas(owner: &Owner) {
    // The scene's root, read through the queue; there is no window in this scene.
    let (root, _) = owner.read(|scene| scene.root().expect("the scene has a root"));
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
            new("canvas", PLACE),
            insert(PLACE, PARENT),
            set(PLACE, "width", TreeValue::Double(f64::from(SIDE))),
            set(PLACE, "height", TreeValue::Double(f64::from(SIDE))),
            set(PLACE, "content", TreeValue::Bytes(bar_paint_list())),
            new("view", VIEW),
            insert(VIEW, root),
            set(VIEW, "width", TreeValue::Double(f64::from(SIDE))),
            set(VIEW, "height", TreeValue::Double(f64::from(SIDE))),
            set(VIEW, "source", TreeValue::Node(Some(PARENT))),
        ]
    });
    owner.frames(3);
}

impl Viewer {
    /// Captures frames until pixel `(x, y)` of the target is `word`, ignoring alpha.
    fn capture_until_pixel(&mut self, x: u32, y: u32, word: u32) {
        let deadline = Instant::now() + TIMEOUT;
        let mut last = String::from("no frame was delivered");
        while Instant::now() < deadline {
            if self.capture() {
                let seen = self.target.word(x, y);
                if seen & 0x00FF_FFFF == word & 0x00FF_FFFF {
                    return;
                }
                last = format!("pixel ({x}, {y}) is {seen:#010x}, not {word:#010x}");
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("the capture never showed {word:#010x} at ({x}, {y}): {last}");
    }

    /// How many pixels of the box `[x, x + width) x [y, y + height)` of the target differ from `colour`, ignoring alpha.
    fn differing(&self, x: u32, y: u32, width: u32, height: u32, colour: u32) -> usize {
        (y..y + height)
            .flat_map(|row| (x..x + width).map(move |column| (column, row)))
            .filter(|(column, row)| self.target.word(*column, *row) & 0x00FF_FFFF != colour & 0x00FF_FFFF)
            .count()
    }
}

/// A canvas the shell paints reaches the pixels of a View: the bar's fill where nothing is drawn over it, the text
/// where the text is, and the icon where the icon is.
#[test]
fn a_canvas_inside_a_view_is_captured_with_its_painted_pixels() {
    let (session, owner) = start();
    session.run(move |handle| {
        arrange_canvas(&owner);
        let mut viewer = Viewer::open(handle);
        // The bottom right of the bar has nothing drawn over the fill.
        viewer.capture_until_pixel(SIDE - 2, SIDE - 2, SOURCE);
        assert_eq!(
            viewer.differing(0, 24, SIDE, SIDE - 24, SOURCE),
            0,
            "below the text and the icon is fill"
        );
        let text = viewer.differing(2, 0, 30, 24, SOURCE);
        assert!(text >= 20, "the text paints pixels over the fill: {text} differ");
        let icon = viewer.differing(40, 4, 16, 16, SOURCE);
        assert!(icon >= 10, "the close icon paints pixels over the fill: {icon} differ");
        assert_eq!(
            viewer.differing(34, 0, 4, 24, SOURCE),
            0,
            "the gap between the text and the icon is fill"
        );
    });
}

/// Repainting a canvas is delivered again: new content, new pixels.
#[test]
fn a_canvas_whose_content_is_written_again_is_captured_again() {
    let (session, owner) = start();
    session.run(move |handle| {
        arrange_canvas(&owner);
        let mut viewer = Viewer::open(handle);
        viewer.capture_until_pixel(SIDE - 2, SIDE - 2, SOURCE);
        let repainted = {
            use gnoblin_paint::{PaintList, PaintOp, Rgba8};
            gnoblin_paint::encode(&PaintList {
                ops: vec![PaintOp::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: SIDE as f32,
                    height: SIDE as f32,
                    radius: 0.0,
                    fill: Rgba8::new(0xC0, 0x40, 0x20, 0xFF),
                    stroke: Rgba8::TRANSPARENT,
                    stroke_width: 0.0,
                }],
            })
        };
        owner.transact(|| {
            vec![TreeOp::Set {
                node: PLACE,
                key: "content".to_owned(),
                value: TreeValue::Bytes(repainted.clone()),
                timing: None,
            }]
        });
        viewer.capture_until(REPAINTED);
    });
}

/// What the session published while a capture ran: how many capture sessions were open at each publish, and whether the
/// file the capture writes existed then.
#[derive(Debug, Default)]
struct Recorder {
    /// The file the capture writes, once the test knows where the session's runtime directory is.
    target: OnceLock<std::path::PathBuf>,
    /// One entry per publish.
    seen: Mutex<Vec<(usize, bool)>>,
}

impl Observer for Recorder {
    fn published(&self, snapshot: &SessionSnapshot) {
        let written = self.target.get().is_some_and(|target| target.exists());
        self.seen
            .lock()
            .expect("the recorder lock")
            .push((snapshot.capture_sessions(), written));
    }
}

/// The pixel of a PNG at `(x, y)` as `0xRRGGBB`, and the PNG's size.
fn png_pixel(path: &std::path::Path, x: u32, y: u32) -> ((u32, u32), u32) {
    let decoder = png::Decoder::new(std::io::BufReader::new(File::open(path).expect("the PNG was written")));
    let mut reader = decoder.read_info().expect("a PNG header");
    let mut data = vec![0; reader.output_buffer_size().expect("a PNG buffer size")];
    let info = reader.next_frame(&mut data).expect("a PNG frame");
    assert_eq!(info.color_type, png::ColorType::Rgba, "the PNG is RGBA");
    let at = (y * info.width + x) as usize * 4;
    (
        (info.width, info.height),
        u32::from(data[at]) << 16 | u32::from(data[at + 1]) << 8 | u32::from(data[at + 2]),
    )
}

/// A capture call writes the View's picture where the shell names, answers the path, and keeps the privacy
/// indicator's count of capture sessions up from before the pixels are read to after the file is written.
#[test]
fn a_capture_call_writes_a_png_of_the_view_while_the_indicator_is_on() {
    let recorder = Arc::new(Recorder::default());
    let (session, owner) = start_with(Some(recorder.clone()), None);
    session.run(move |handle| {
        arrange_canvas(&owner);
        let runtime = std::fs::canonicalize(handle.socket().parent().expect("the socket's directory")).expect("a path");
        let target = runtime.join("view.png");
        recorder.target.set(target.clone()).expect("the target is set once");

        let answer = owner.capture(VIEW, target.to_str().expect("a utf-8 path"));
        assert_eq!(
            (answer.error.as_str(), answer.result.as_str()),
            ("", target.to_str().expect("a utf-8 path")),
            "the shell is answered with the path written"
        );
        let (size, colour) = png_pixel(&target, SIDE - 2, SIDE - 2);
        assert_eq!(size, (SIDE, SIDE), "the PNG is the View's size");
        assert_eq!(colour, SOURCE & 0x00FF_FFFF, "the PNG holds the View's pixels");

        let seen = recorder.seen.lock().expect("the recorder lock").clone();
        assert!(
            seen.contains(&(1, false)),
            "a publish showed the capture running before the file existed: {seen:?}"
        );
        assert!(
            seen.iter().all(|(sessions, written)| *sessions == 0 || !written),
            "the capture is shown open until the file is written, never after: {seen:?}"
        );
        assert_eq!(
            seen.last(),
            Some(&(0, true)),
            "the capture closes once the file exists: {seen:?}"
        );
        let staged: Vec<_> = std::fs::read_dir(&runtime)
            .expect("the runtime directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".view.png"))
            .collect();
        assert!(staged.is_empty(), "no staging file is left behind");
    });
}

/// A locked session writes nothing: the capture is refused at the copy and the indicator never shows it.
#[test]
fn a_capture_call_while_locked_is_refused_and_writes_nothing() {
    let recorder = Arc::new(Recorder::default());
    let (session, owner) = start_with(Some(recorder.clone()), None);
    session.run(move |handle| {
        arrange_canvas(&owner);
        let target = std::fs::canonicalize(handle.socket().parent().expect("the socket's directory"))
            .expect("a path")
            .join("locked.png");
        recorder.target.set(target.clone()).expect("the target is set once");
        let _locker = lock(handle.socket());
        owner.frames(3);

        let answer = owner.capture(VIEW, target.to_str().expect("a utf-8 path"));
        assert_eq!(answer.error, "locked");
        assert!(answer.result.is_empty(), "a refusal carries no result");
        assert!(!target.exists(), "nothing was written under the lock");
        let seen = recorder.seen.lock().expect("the recorder lock").clone();
        assert!(
            seen.iter().all(|(sessions, _)| *sessions == 0),
            "a refused capture never showed as running: {seen:?}"
        );
    });
}

/// The path is checked: only an absolute `.png` inside the runtime directory is written, and a View with nothing
/// drawn has nothing to write.
#[test]
fn a_capture_call_refuses_a_path_outside_the_runtime_directory_and_a_view_with_no_picture() {
    let (session, owner) = start();
    session.run(move |handle| {
        arrange_canvas(&owner);
        let runtime = std::fs::canonicalize(handle.socket().parent().expect("the socket's directory")).expect("a path");
        let elsewhere = tempfile::tempdir().expect("another directory");
        let outside = elsewhere.path().join("view.png");
        let climbing = format!("{}/../view.png", runtime.display());
        for path in [
            outside.to_str().expect("a utf-8 path"),
            "view.png",
            climbing.as_str(),
            runtime.join("view.txt").to_str().expect("a utf-8 path"),
            "",
        ] {
            let answer = owner.capture(VIEW, path);
            assert_eq!(answer.error, "bad-value", "{path:?} is refused");
            assert!(answer.result.is_empty());
        }
        assert!(!outside.exists(), "nothing was written outside the runtime directory");
        assert!(
            !runtime.join("view.png").exists(),
            "nothing was written in the runtime directory"
        );

        // A View with no source draws no picture.
        let bare = NodeId::new((1 << 32) | 0x103);
        let (root, _) = owner.read(|scene| scene.root().expect("the scene has a root"));
        owner.transact(|| {
            vec![
                TreeOp::New {
                    class: "view".to_owned(),
                    id: bare,
                },
                TreeOp::Insert {
                    node: bare,
                    parent: root,
                    index: None,
                },
            ]
        });
        owner.frames(3);
        let answer = owner.capture(bare, runtime.join("bare.png").to_str().expect("a utf-8 path"));
        assert_eq!(answer.error, "unavailable");
        assert!(!runtime.join("bare.png").exists());
    });
}

/// `fps` reaches the composer through the tree: a View with `fps = 2` holds its picture for half a second of the
/// session's clock however often its source changes, and draws the last state when the slot ends.
#[test]
fn a_views_fps_paces_its_picture_on_the_session_clock() {
    let clock = Arc::new(ManualClock::new());
    let (session, owner) = start_with(None, Some(clock.clone()));
    session.run(move |handle| {
        arrange_canvas(&owner);
        let runtime = std::fs::canonicalize(handle.socket().parent().expect("the socket's directory")).expect("a path");
        let still = |name: &str| {
            let target = runtime.join(name);
            let answer = owner.capture(VIEW, target.to_str().expect("a utf-8 path"));
            assert_eq!(answer.error, "", "the View was captured");
            png_pixel(&target, SIDE - 2, SIDE - 2).1
        };
        assert_eq!(still("first.png"), SOURCE & 0x00FF_FFFF);

        owner.transact(|| {
            vec![TreeOp::Set {
                node: VIEW,
                key: "fps".to_owned(),
                value: TreeValue::Uint(2),
                timing: None,
            }]
        });
        let repaint = |red: u8| {
            use gnoblin_paint::{PaintList, PaintOp, Rgba8};
            let content = gnoblin_paint::encode(&PaintList {
                ops: vec![PaintOp::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: SIDE as f32,
                    height: SIDE as f32,
                    radius: 0.0,
                    fill: Rgba8::new(red, 0x40, 0x20, 0xFF),
                    stroke: Rgba8::TRANSPARENT,
                    stroke_width: 0.0,
                }],
            });
            owner.transact(|| {
                vec![TreeOp::Set {
                    node: PLACE,
                    key: "content".to_owned(),
                    value: TreeValue::Bytes(content.clone()),
                    timing: None,
                }]
            });
            owner.frames(3);
        };
        repaint(0x70);
        repaint(0xC0);
        assert_eq!(
            still("held.png"),
            SOURCE & 0x00FF_FFFF,
            "no time has passed on the session's clock, so the View keeps its picture"
        );

        clock.advance(Duration::from_millis(499));
        owner.frames(3);
        assert_eq!(still("early.png"), SOURCE & 0x00FF_FFFF, "499 ms is inside the slot");

        clock.advance(Duration::from_millis(1));
        owner.frames(3);
        assert_eq!(
            still("late.png"),
            0xC0_40_20,
            "the slot's end draws the last state, not the one before it"
        );
    });
}

/// The word a secure node's place holds in a View's capture: the neutral grey an output capture draws over a secure
/// layer, opaque.
const MASKED: u32 = 0xFF80_8080;

/// The second View of the nested arrangement, a plain node holding it, and the View of that node.
const INNER: NodeId = NodeId::new((1 << 32) | 0x103);
const HOLDER: NodeId = NodeId::new(0x104);
const OUTER: NodeId = NodeId::new((1 << 32) | 0x105);

/// Every pixel of a PNG, as `0xRRGGBB`, and its size.
fn png_words(path: &std::path::Path) -> ((u32, u32), Vec<u32>) {
    let decoder = png::Decoder::new(std::io::BufReader::new(File::open(path).expect("the PNG was written")));
    let mut reader = decoder.read_info().expect("a PNG header");
    let mut data = vec![0; reader.output_buffer_size().expect("a PNG buffer size")];
    let info = reader.next_frame(&mut data).expect("a PNG frame");
    assert_eq!(info.color_type, png::ColorType::Rgba, "the PNG is RGBA");
    let words = data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| u32::from(pixel[0]) << 16 | u32::from(pixel[1]) << 8 | u32::from(pixel[2]))
        .collect();
    ((info.width, info.height), words)
}

/// Writes `secure` on `node`, as the shell does for a window it marks.
fn mark_secure(owner: &Owner, node: NodeId, secure: bool) {
    owner.transact(|| {
        vec![TreeOp::Set {
            node,
            key: "secure".to_owned(),
            value: TreeValue::Bool(secure),
            timing: None,
        }]
    });
    // The mark is applied by the frame after the write, and the View is drawn again in the frame that follows.
    owner.frames(3);
}

/// Captures `view` to a PNG in the session's runtime directory and answers its pixels.
fn still_of(owner: &Owner, handle: &Handle, view: NodeId, name: &str) -> Vec<u32> {
    let runtime = std::fs::canonicalize(handle.socket().parent().expect("the socket's directory")).expect("a path");
    let target = runtime.join(name);
    let answer = owner.capture(view, target.to_str().expect("a utf-8 path"));
    assert_eq!(answer.error, "", "the View {view:?} was captured");
    let (size, words) = png_words(&target);
    assert_eq!(size, (SIDE, SIDE), "the PNG is the View's size");
    words
}

/// A secure window's pixels are masked in the PNG of a View, and come back when the mark is cleared.
#[test]
fn a_png_of_a_view_holds_none_of_a_secure_windows_pixels() {
    let (session, owner) = start();
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let _window = Window::map(&mut app, SOURCE);
        arrange(&owner);
        let (window, ..) = owner.window();
        let source = SOURCE & 0x00FF_FFFF;

        assert!(
            still_of(&owner, handle, VIEW, "open.png")
                .iter()
                .all(|word| *word == source),
            "an unmarked window is captured as it is drawn"
        );
        mark_secure(&owner, window, true);
        let masked = still_of(&owner, handle, VIEW, "secure.png");
        assert!(
            masked.iter().all(|word| *word == MASKED & 0x00FF_FFFF),
            "every pixel of the secure window is the mask"
        );
        mark_secure(&owner, window, false);
        assert!(
            still_of(&owner, handle, VIEW, "cleared.png")
                .iter()
                .all(|word| *word == source),
            "a cleared mark shows the window again"
        );
    });
}

/// A `view`-role session's frames never carry a secure window's pixels, however long it runs, and show the window
/// again when the mark is cleared.
#[test]
fn a_view_role_session_never_receives_a_secure_windows_pixels() {
    let (session, owner) = start();
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let _window = Window::map(&mut app, SOURCE);
        arrange(&owner);
        let mut viewer = Viewer::open(handle);
        viewer.capture_until(SOURCE);
        let (window, ..) = owner.window();

        mark_secure(&owner, window, true);
        viewer.capture_until(MASKED);
        for _ in 0..5 {
            assert!(viewer.capture(), "the session keeps delivering frames");
            assert_eq!(
                viewer.target.count_of(SOURCE),
                0,
                "no frame holds a pixel of the secure window"
            );
        }

        mark_secure(&owner, window, false);
        viewer.capture_until(SOURCE);
    });
}

/// A secure node marks everything below it: a window under a secure plain node is masked though the window itself is
/// not marked.
#[test]
fn a_window_under_a_secure_node_is_masked_in_a_views_capture() {
    let (session, owner) = start();
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let _window = Window::map(&mut app, SOURCE);
        arrange(&owner);

        mark_secure(&owner, PARENT, true);
        let masked = still_of(&owner, handle, VIEW, "parent.png");
        assert!(
            masked.iter().all(|word| *word == MASKED & 0x00FF_FFFF),
            "the window below the secure node is masked"
        );
        let mut viewer = Viewer::open(handle);
        viewer.capture_until(MASKED);
    });
}

/// A View of a View: the outer View's source holds an inner View of the secure window. Neither capture carries the
/// window's pixels: the inner View's is masked, and the outer one draws no picture of the inner at all.
#[test]
fn a_view_of_a_view_never_shows_a_secure_windows_pixels() {
    let (session, owner) = start();
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let _window = Window::map(&mut app, SOURCE);
        arrange(&owner);
        let (window, _, root) = owner.window();
        owner.transact(|| {
            let set = |node, key: &str, value| TreeOp::Set {
                node,
                key: key.to_owned(),
                value,
                timing: None,
            };
            let side = TreeValue::Double(f64::from(SIDE));
            vec![
                TreeOp::New {
                    class: "node".to_owned(),
                    id: HOLDER,
                },
                TreeOp::Insert {
                    node: HOLDER,
                    parent: root,
                    index: None,
                },
                TreeOp::New {
                    class: "view".to_owned(),
                    id: INNER,
                },
                TreeOp::Insert {
                    node: INNER,
                    parent: HOLDER,
                    index: None,
                },
                set(INNER, "width", side.clone()),
                set(INNER, "height", side.clone()),
                set(INNER, "source", TreeValue::Node(Some(PARENT))),
                TreeOp::New {
                    class: "view".to_owned(),
                    id: OUTER,
                },
                TreeOp::Insert {
                    node: OUTER,
                    parent: root,
                    index: None,
                },
                set(OUTER, "width", side.clone()),
                set(OUTER, "height", side),
                set(OUTER, "source", TreeValue::Node(Some(HOLDER))),
            ]
        });
        owner.frames(3);
        let source = SOURCE & 0x00FF_FFFF;
        assert!(
            still_of(&owner, handle, INNER, "inner-open.png")
                .iter()
                .all(|word| *word == source),
            "the inner View shows the window while it is not secure"
        );

        mark_secure(&owner, window, true);
        let inner = still_of(&owner, handle, INNER, "inner.png");
        assert!(
            inner.iter().all(|word| *word == MASKED & 0x00FF_FFFF),
            "the inner View's capture is the mask"
        );
        let outer = still_of(&owner, handle, OUTER, "outer.png");
        assert!(
            outer.iter().all(|word| *word != source),
            "the outer View's capture holds no pixel of the secure window"
        );
        let mut viewer = Viewer::open_view(handle, OUTER);
        for _ in 0..3 {
            assert!(viewer.capture(), "the outer session delivers frames");
            assert_eq!(
                viewer.target.count_of(SOURCE),
                0,
                "no frame of the outer View holds the window"
            );
        }
    });
}

/// The word of window pixel `(x, y)` when the window is painted by `paint_gradient`: opaque, green the column and blue
/// the row, so no two pixels of the window share a word.
fn gradient(x: u32, y: u32) -> u32 {
    0xFF00_0000 | (x << 8) | y
}

/// What a client leaves in a buffer it asked a frame into, so a pixel the compositor did not write is plain.
const UNTOUCHED: u32 = 0xFF11_2233;

/// A frame of a View that has shrunk below its session's size holds the View's whole picture where the picture is, and
/// nothing where it is not; one of a View that grew holds the session's size of it.
#[test]
fn a_resized_view_is_copied_as_the_matching_sub_image() {
    let (session, owner) = start();
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let mut window = Window::map(&mut app, SOURCE);
        window.paint_gradient(&mut app);
        arrange(&owner);
        let mut viewer = Viewer::open_as(handle, VIEW, Some(wl_shm::Format::Argb8888));

        viewer.target.fill(UNTOUCHED);
        viewer.capture_until_pixel(SIDE - 1, SIDE - 1, gradient(SIDE - 1, SIDE - 1));
        for (x, y) in [(0, 0), (SIDE - 1, 0), (0, SIDE - 1), (SIDE - 1, SIDE - 1), (17, 41)] {
            assert_eq!(
                viewer.target.word(x, y),
                gradient(x, y),
                "pixel ({x}, {y}) of the full frame"
            );
        }

        // The View is now half the size its session negotiated: its picture is the window's top left quarter.
        let half = SIDE / 2;
        owner.transact(|| {
            ["width", "height"]
                .map(|key| TreeOp::Set {
                    node: VIEW,
                    key: key.to_owned(),
                    value: TreeValue::Double(f64::from(half)),
                    timing: None,
                })
                .into()
        });
        owner.frames(3);
        viewer.target.fill(UNTOUCHED);
        assert!(viewer.capture(), "the frame of the smaller View is delivered");
        for (x, y) in [(0, 0), (half - 1, 0), (0, half - 1), (half - 1, half - 1), (9, 21)] {
            assert_eq!(
                viewer.target.word(x, y),
                gradient(x, y),
                "pixel ({x}, {y}) inside the View is the window's pixel at that place"
            );
        }
        for (x, y) in [(half, 0), (0, half), (half, half), (SIDE - 1, SIDE - 1), (50, 7)] {
            assert_eq!(
                viewer.target.word(x, y),
                UNTOUCHED,
                "pixel ({x}, {y}) outside the View is left as the client had it"
            );
        }

        // Grown to twice the session's size, the View still fills exactly the session's buffer.
        owner.transact(|| {
            ["width", "height"]
                .map(|key| TreeOp::Set {
                    node: VIEW,
                    key: key.to_owned(),
                    value: TreeValue::Double(f64::from(SIDE * 2)),
                    timing: None,
                })
                .into()
        });
        owner.frames(3);
        viewer.target.fill(UNTOUCHED);
        assert!(viewer.capture(), "the frame of the larger View is delivered");
        for (x, y) in [(0, 0), (SIDE - 1, 0), (0, SIDE - 1), (SIDE - 1, SIDE - 1), (33, 5)] {
            assert_eq!(
                viewer.target.word(x, y),
                gradient(x, y),
                "pixel ({x}, {y}) of the buffer is the window's pixel at that place"
            );
        }
    });
}

/// An `xrgb8888` buffer is converted by its own format: the same pixels arrive, with an opaque fourth byte, and an
/// `argb8888` buffer receives the picture's alpha.
#[test]
fn an_xrgb_buffer_receives_the_views_pixels_converted() {
    let (session, owner) = start();
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let mut window = Window::map(&mut app, SOURCE);
        window.paint_gradient(&mut app);
        arrange(&owner);
        for format in [wl_shm::Format::Xrgb8888, wl_shm::Format::Argb8888] {
            let mut viewer = Viewer::open_as(handle, VIEW, Some(format));
            viewer.target.fill(UNTOUCHED);
            viewer.capture_until_pixel(SIDE - 1, SIDE - 1, gradient(SIDE - 1, SIDE - 1));
            for (x, y) in [(0, 0), (SIDE - 1, SIDE - 1), (30, 12)] {
                assert_eq!(
                    viewer.target.word(x, y),
                    gradient(x, y),
                    "pixel ({x}, {y}) of a {format:?} buffer, fourth byte included"
                );
            }
        }
    });
}

/// A buffer smaller than the size the session advertised is refused with the frame's `failed` event, and nothing is
/// written into it.
#[test]
fn a_buffer_smaller_than_the_view_is_refused_and_left_untouched() {
    let (session, owner) = start();
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let _window = Window::map(&mut app, SOURCE);
        arrange(&owner);
        let mut viewer = Viewer::open(handle);
        let small = Pixels::solid(
            &viewer._shm,
            &viewer.client.qh,
            SIDE / 2,
            SIDE / 2,
            viewer.target.format,
            UNTOUCHED,
        );
        let (ready, failed) = (viewer.client.state.frame_ready, viewer.client.state.frame_failed);
        let frame = viewer.session.create_frame(&viewer.client.qh, ());
        frame.attach_buffer(&small.buffer);
        frame.capture();
        viewer.client.wait_until("an answer to the frame request", |state| {
            state.frame_ready > ready || state.frame_failed > failed
        });
        assert_eq!(
            viewer.client.state.frame_ready, ready,
            "the undersized buffer is not served"
        );
        assert_eq!(viewer.client.state.frame_failed, failed + 1, "the frame fails");
        assert_eq!(
            small.first_other_than(UNTOUCHED),
            None,
            "nothing was written into the buffer"
        );
        frame.destroy();
        assert!(viewer.capture(), "the session still serves a buffer of the right size");
    });
}

// The password check through the compositor's lock path: a real lock client takes the lock, the shell calls
// `authenticate` on the compositor's `lock` node, and a scripted authenticator answers where PAM would. The same
// session is driven from the owner's tree, so these are the calls the stock shell's `Lock:unlock` makes.

const USER: &str = "kieran";
const PASSWORD: &str = "correct horse";

/// A session whose lock checks passwords with `authenticator` for `USER`.
fn start_with_lock_check(authenticator: gnoblin_lock_auth::FakeAuthenticator) -> (Session, Owner) {
    let (hooks, answers) = hooks();
    let owner = owner_of(&hooks, answers);
    let extras = Extras {
        lock_check: Some(gnoblin_wayland::LockCheck {
            authenticator: Arc::new(authenticator),
            user: Some(USER.to_owned()),
        }),
        ..Extras::default()
    };
    (Session::start_with_extras(None, hooks, extras), owner)
}

/// The compositor's `lock` node, once the composer has made it.
fn lock_node(owner: &Owner) -> NodeId {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let (found, _) = owner.read(|scene| scene.node_ids().into_iter().find(|id| scene.class(*id) == Some("lock")));
        if let Some(node) = found {
            return node;
        }
        assert!(Instant::now() < deadline, "the compositor never made its lock node");
        thread::sleep(Duration::from_millis(5));
    }
}

/// Waits until the `lock` node's `state` reads `want`, or fails with what it read last.
fn wait_lock_state(owner: &Owner, node: NodeId, want: &str) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let (state, _) = owner.read(move |scene| scene.text(node, "state").map(str::to_owned));
        if state.as_deref() == Some(want) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the lock never read `{want}`; it read {state:?}"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

/// What the lock reads right now.
fn lock_state(owner: &Owner, node: NodeId) -> Option<String> {
    owner.frames(2);
    owner.read(move |scene| scene.text(node, "state").map(str::to_owned)).0
}

#[test]
fn the_right_password_unlocks_the_session() {
    let (session, owner) = start_with_lock_check(gnoblin_lock_auth::FakeAuthenticator::new().accept(USER, PASSWORD));
    session.run(move |handle| {
        let _locker = lock(handle.socket());
        let node = lock_node(&owner);
        wait_lock_state(&owner, node, "locked");
        let answer = owner.call(node, "authenticate", PASSWORD);
        assert_eq!((answer.error.as_str(), answer.result.as_str()), ("", "ok"));
        wait_lock_state(&owner, node, "not-locked");
    });
}

#[test]
fn a_wrong_password_keeps_the_session_locked_and_repeated_ones_are_rate_limited() {
    let (session, owner) = start_with_lock_check(gnoblin_lock_auth::FakeAuthenticator::new().accept(USER, PASSWORD));
    session.run(move |handle| {
        let _locker = lock(handle.socket());
        let node = lock_node(&owner);
        wait_lock_state(&owner, node, "locked");
        for attempt in 0..gnoblin_lock_auth::DENIALS_BEFORE_DELAY {
            let answer = owner.call(node, "authenticate", "wrong");
            assert_eq!(answer.error, "denied", "attempt {attempt}");
            assert_eq!(lock_state(&owner, node).as_deref(), Some("locked"));
        }
        // The next attempt waits out the delay, even with the right password.
        let limited = owner.call(node, "authenticate", PASSWORD);
        assert_eq!(limited.error, "rate-limited", "{limited:?}");
        assert!(
            limited.result.parse::<u64>().is_ok_and(|ms| ms > 0),
            "the answer names the wait: {limited:?}"
        );
        assert_eq!(
            lock_state(&owner, node).as_deref(),
            Some("locked"),
            "the right password did not unlock"
        );
    });
}

/// A check PAM cannot finish, an expired or denied account among them, never unlocks, and its answer names the failure
/// without the password.
#[test]
fn a_check_that_cannot_run_keeps_the_session_locked_and_answers_without_the_password() {
    for failure in ["PAM returned error 13", "cannot load libpam.so.0"] {
        let (session, owner) = start_with_lock_check(gnoblin_lock_auth::FakeAuthenticator::new().fail(failure));
        session.run(move |handle| {
            let _locker = lock(handle.socket());
            let node = lock_node(&owner);
            wait_lock_state(&owner, node, "locked");
            let answer = owner.call(node, "authenticate", PASSWORD);
            assert_eq!(answer.error, format!("failed: {failure}"));
            assert!(answer.result.is_empty(), "{answer:?}");
            assert!(
                !format!("{answer:?}").contains(PASSWORD),
                "the answer never carries the password"
            );
            assert_eq!(lock_state(&owner, node).as_deref(), Some("locked"));
        });
    }
}

// A View as a PipeWire cast source. The portal's half is the IPC (`cast.grant`, `cast.start`); what the compositor
// does at `cast.start` is `ControlHandle::start_cast`, called here as the IPC server calls it. Each test runs a
// private PipeWire daemon and reads the stream with a consumer linked to its node, so what is asserted is the pixels
// an app would receive.

/// The IPC connection that holds the user's consent, as the compositor names it.
const PORTAL: gnoblin_types::ClientId = gnoblin_types::ClientId::new(1);

/// A colour word as `[red, green, blue]`.
fn rgb(word: u32) -> [u8; 3] {
    [(word >> 16) as u8, (word >> 8) as u8, word as u8]
}

/// A session whose casts go to `daemon`, with the extras a test hands the loop.
fn start_casting(
    daemon: &Daemon,
    observer: Option<Arc<dyn Observer>>,
    clock: Option<Arc<dyn Clock>>,
) -> (Session, Owner) {
    let (hooks, answers) = hooks();
    let owner = owner_of(&hooks, answers);
    let extras = Extras {
        observer,
        clock,
        pipewire_socket: Some(daemon.socket()),
        ..Extras::default()
    };
    (Session::start_with_extras(None, hooks, extras), owner)
}

/// Starts the cast of `view` as the portal does after the user said yes, and answers the stream.
#[track_caller]
fn cast(handle: &Handle, view: NodeId) -> gnoblin_wayland::CastStream {
    handle
        .control
        .start_cast(PORTAL, gnoblin_capture::Target::View(view))
        .expect("the loop answers a stream start")
        .expect("a View in the scene is cast")
}

/// The name the compositor gives the PipeWire node of `stream`.
fn node_name(stream: &gnoblin_wayland::CastStream) -> String {
    format!("gnoblin-capture-{}", stream.session.get())
}

/// A stream of a View is the View's picture at the View's size: a consumer linked to its node receives a frame whose
/// every pixel is the colour of the window the View draws, and the privacy indicator is on for as long as the stream
/// runs.
#[test]
fn a_cast_of_a_view_delivers_the_views_pixels_and_keeps_the_indicator_on_while_it_runs() {
    let daemon = Daemon::start();
    let recorder = Arc::new(Recorder::default());
    let (session, owner) = start_casting(&daemon, Some(recorder.clone()), None);
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let _window = Window::map(&mut app, SOURCE);
        arrange(&owner);

        let stream = cast(handle, VIEW);
        assert_eq!(
            (stream.size.width, stream.size.height),
            (SIDE, SIDE),
            "the stream is the View's size"
        );
        assert_eq!(
            recorder
                .seen
                .lock()
                .expect("the recorder lock")
                .last()
                .map(|seen| seen.0),
            Some(1),
            "the indicator's count of capture sessions is up when the start is answered"
        );

        let consumer = FrameConsumer::connect(&daemon.socket(), stream.node);
        let frame = consumer.wait_for_frame("of the View's colour", |frame| frame.is_solid(rgb(SOURCE)));
        assert_eq!(
            (frame.width, frame.height),
            (SIDE, SIDE),
            "the consumer negotiated the View's size"
        );

        handle.control.stop_cast(stream.session);
        assert!(
            consumer.observer.wait_gone(&node_name(&stream)),
            "the stream's node leaves the daemon when the cast ends"
        );
        wait_for_count(&recorder, 0);
    });
}

/// Waits until the last publish counted `count` capture sessions.
#[track_caller]
fn wait_for_count(recorder: &Recorder, count: usize) {
    let deadline = Instant::now() + TIMEOUT;
    while Instant::now() < deadline {
        if recorder
            .seen
            .lock()
            .expect("the recorder lock")
            .last()
            .map(|seen| seen.0)
            == Some(count)
        {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("the indicator never showed {count} capture sessions");
}

/// A View that changes is sent again: the window is repainted, the View draws it, and the next frame the consumer
/// receives is the new colour.
#[test]
fn a_cast_of_a_view_sends_a_new_frame_when_the_view_changes() {
    let daemon = Daemon::start();
    let (session, owner) = start_casting(&daemon, None, None);
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let mut window = Window::map(&mut app, SOURCE);
        arrange(&owner);
        let stream = cast(handle, VIEW);
        let consumer = FrameConsumer::connect(&daemon.socket(), stream.node);
        consumer.wait_for_frame("of the first colour", |frame| frame.is_solid(rgb(SOURCE)));

        window.paint(&mut app, REPAINTED);
        consumer.wait_for_frame("of the repainted colour", |frame| frame.is_solid(rgb(REPAINTED)));
    });
}

/// The stream shows a secure window as the mask and never as its pixels, from the frame after the mark and for as
/// long as it stands, and shows the window again once the mark is cleared.
#[test]
fn a_cast_of_a_view_masks_a_secure_window() {
    let daemon = Daemon::start();
    let (session, owner) = start_casting(&daemon, None, None);
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let _window = Window::map(&mut app, SOURCE);
        arrange(&owner);
        let stream = cast(handle, VIEW);
        let consumer = FrameConsumer::connect(&daemon.socket(), stream.node);
        consumer.wait_for_frame("of the window's colour", |frame| frame.is_solid(rgb(SOURCE)));
        let (window, ..) = owner.window();

        mark_secure(&owner, window, true);
        consumer.wait_for_frame("of the mask", |frame| frame.is_solid(rgb(MASKED)));
        // From the first masked frame on, no frame the stream sends holds a pixel of the window.
        consumer.forget();
        consumer.listen(Duration::from_millis(600));
        let frames = consumer.frames();
        assert!(!frames.is_empty(), "the stream keeps delivering frames while masked");
        assert!(
            frames.iter().all(|frame| frame.count_of(rgb(SOURCE)) == 0),
            "no frame holds a pixel of the secure window"
        );

        mark_secure(&owner, window, false);
        consumer.wait_for_frame("of the window's colour again", |frame| frame.is_solid(rgb(SOURCE)));
    });
}

/// The lock ends the stream: its node leaves the daemon, and a cast started while the desktop is locked is refused,
/// because a user who consented to a cast did not consent to one of a locked desktop.
#[test]
fn the_session_lock_ends_a_cast_of_a_view_and_refuses_a_new_one() {
    let daemon = Daemon::start();
    let (session, owner) = start_casting(&daemon, None, None);
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let _window = Window::map(&mut app, SOURCE);
        arrange(&owner);
        let stream = cast(handle, VIEW);
        let consumer = FrameConsumer::connect(&daemon.socket(), stream.node);
        consumer.wait_for_frame("of the window's colour", |frame| frame.is_solid(rgb(SOURCE)));

        let _locker = lock(handle.socket());
        assert!(
            consumer.observer.wait_gone(&node_name(&stream)),
            "the lock ends the stream: its node is gone"
        );
        consumer.forget();
        consumer.listen(Duration::from_millis(300));
        assert!(
            consumer.frames().iter().all(|frame| frame.count_of(rgb(SOURCE)) == 0),
            "no frame of the desktop arrives after the lock"
        );
        let refused = handle
            .control
            .start_cast(PORTAL, gnoblin_capture::Target::View(VIEW))
            .expect("the loop answers");
        assert_eq!(refused, Err(gnoblin_wayland::CastRefusal::Locked));
    });
}

/// A View that is destroyed ends its stream: the node leaves the daemon, the indicator goes off, and the View is no
/// longer a target.
#[test]
fn a_cast_of_a_view_ends_cleanly_when_the_view_is_destroyed() {
    let daemon = Daemon::start();
    let recorder = Arc::new(Recorder::default());
    let (session, owner) = start_casting(&daemon, Some(recorder.clone()), None);
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let _window = Window::map(&mut app, SOURCE);
        arrange(&owner);
        let stream = cast(handle, VIEW);
        let consumer = FrameConsumer::connect(&daemon.socket(), stream.node);
        consumer.wait_for_frame("of the window's colour", |frame| frame.is_solid(rgb(SOURCE)));
        wait_for_count(&recorder, 1);

        owner.transact(|| vec![TreeOp::Destroy { node: VIEW }]);
        owner.frames(3);
        assert!(
            consumer.observer.wait_gone(&node_name(&stream)),
            "the node leaves the daemon with the View"
        );
        wait_for_count(&recorder, 0);
        let again = handle
            .control
            .start_cast(PORTAL, gnoblin_capture::Target::View(VIEW))
            .expect("the loop answers");
        assert_eq!(again, Err(gnoblin_wayland::CastRefusal::NoTarget));
    });
}

/// The replacement source the test gives the View.
const NEW_PARENT: NodeId = NodeId::new(0x106);
const NEW_PLACE: NodeId = NodeId::new(0x107);

/// A View whose *source* is destroyed is still a View: its stream is not the source's to end. The stream carries
/// none of the destroyed source's pixels from the frame after the destroy (nothing stale is repeated), the node stays
/// in the daemon and the indicator stays on, and when the shell gives the View a new source the stream carries that
/// source's pixels again without a new session.
#[test]
fn a_cast_of_a_view_whose_source_is_destroyed_carries_none_of_its_pixels_and_recovers_with_a_new_source() {
    let daemon = Daemon::start();
    let recorder = Arc::new(Recorder::default());
    let (session, owner) = start_casting(&daemon, Some(recorder.clone()), None);
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let mut window = Window::map(&mut app, SOURCE);
        arrange(&owner);
        let stream = cast(handle, VIEW);
        let consumer = FrameConsumer::connect(&daemon.socket(), stream.node);
        consumer.wait_for_frame("of the window's colour", |frame| frame.is_solid(rgb(SOURCE)));

        // The window is the compositor's node, so a shell cannot destroy it with its holder: it goes back under the
        // root in the same transaction that destroys the holder.
        let (window_node, _, root) = owner.window();
        owner.transact(|| {
            vec![
                TreeOp::Insert {
                    node: window_node,
                    parent: root,
                    index: None,
                },
                TreeOp::Destroy { node: PARENT },
            ]
        });
        owner.frames(3);
        // Frames the stream queued before the destroy may still be on their way: let them arrive, then count only what
        // the stream sends after the compositor has drawn the View with no source.
        consumer.listen(Duration::from_millis(300));
        consumer.forget();
        consumer.listen(Duration::from_millis(400));
        assert!(
            consumer.frames().iter().all(|frame| frame.count_of(rgb(SOURCE)) == 0),
            "no frame carries a pixel of the destroyed source"
        );
        assert!(
            consumer.observer.node(&node_name(&stream)).is_some(),
            "the View is still there, so its stream is"
        );
        wait_for_count(&recorder, 1);

        // The window went back to the root when its holder was destroyed. A new holder, shifted like the first, is a
        // new source; a repaint makes the picture a colour the old source never showed.
        let (window_node, placed, root) = owner.window();
        owner.transact(|| {
            let set = |node, key: &str, value| TreeOp::Set {
                node,
                key: key.to_owned(),
                value,
                timing: None,
            };
            vec![
                TreeOp::New {
                    class: "node".to_owned(),
                    id: NEW_PARENT,
                },
                TreeOp::Insert {
                    node: NEW_PARENT,
                    parent: root,
                    index: None,
                },
                TreeOp::New {
                    class: "node".to_owned(),
                    id: NEW_PLACE,
                },
                TreeOp::Insert {
                    node: NEW_PLACE,
                    parent: NEW_PARENT,
                    index: None,
                },
                TreeOp::Insert {
                    node: window_node,
                    parent: NEW_PLACE,
                    index: None,
                },
                set(NEW_PLACE, "x", TreeValue::Double(-f64::from(placed.x))),
                set(NEW_PLACE, "y", TreeValue::Double(-f64::from(placed.y))),
                set(VIEW, "source", TreeValue::Node(Some(NEW_PARENT))),
            ]
        });
        window.paint(&mut app, REPAINTED);
        owner.frames(3);
        consumer.wait_for_frame("of the new source's colour", |frame| frame.is_solid(rgb(REPAINTED)));
    });
}

/// A View that is not in the scene is not a target: the start is refused by name and makes no node.
#[test]
fn a_cast_of_a_view_that_is_not_there_is_refused() {
    let daemon = Daemon::start();
    let (session, _owner) = start_casting(&daemon, None, None);
    session.run(move |handle| {
        let refused = handle
            .control
            .start_cast(PORTAL, gnoblin_capture::Target::View(VIEW))
            .expect("the loop answers");
        assert_eq!(refused, Err(gnoblin_wayland::CastRefusal::NoTarget));
        let observer = gnoblin_pipewire_export::testing::Observer::connect(&daemon.socket());
        observer.settle();
        assert_eq!(observer.node("gnoblin-capture-1"), None, "no node was made");
    });
}

/// The stream is the View at the size it is drawn at: when the owner shrinks the View, the consumer is renegotiated
/// and receives whole pictures of the new size.
#[test]
fn a_cast_of_a_view_follows_the_size_of_the_view() {
    let daemon = Daemon::start();
    let (session, owner) = start_casting(&daemon, None, None);
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let _window = Window::map(&mut app, SOURCE);
        arrange(&owner);
        let stream = cast(handle, VIEW);
        let consumer = FrameConsumer::connect(&daemon.socket(), stream.node);
        consumer.wait_for_frame("of the View's first size", |frame| {
            frame.is_solid(rgb(SOURCE)) && (frame.width, frame.height) == (SIDE, SIDE)
        });

        owner.transact(|| {
            vec![TreeOp::Set {
                node: VIEW,
                key: "height".to_owned(),
                value: TreeValue::Double(f64::from(SIDE / 2)),
                timing: None,
            }]
        });
        owner.frames(3);
        consumer.wait_for_frame("of the View's new size", |frame| {
            frame.is_solid(rgb(SOURCE)) && (frame.width, frame.height) == (SIDE, SIDE / 2)
        });
    });
}

/// A View's `fps` bounds how often its picture changes, and so how often the stream carries something new: the
/// window is repainted, the session's clock has not moved, and the stream still carries the old picture; when the
/// slot ends it carries the new one.
#[test]
fn a_cast_of_a_view_carries_a_new_picture_no_more_often_than_the_views_fps() {
    let daemon = Daemon::start();
    let clock = Arc::new(ManualClock::new());
    let (session, owner) = start_casting(&daemon, None, Some(clock.clone()));
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let mut window = Window::map(&mut app, SOURCE);
        arrange(&owner);
        owner.transact(|| {
            vec![TreeOp::Set {
                node: VIEW,
                key: "fps".to_owned(),
                value: TreeValue::Uint(2),
                timing: None,
            }]
        });
        let stream = cast(handle, VIEW);
        let consumer = FrameConsumer::connect(&daemon.socket(), stream.node);
        consumer.wait_for_frame("of the first colour", |frame| frame.is_solid(rgb(SOURCE)));

        window.paint(&mut app, REPAINTED);
        owner.frames(3);
        consumer.forget();
        consumer.listen(Duration::from_millis(600));
        assert!(
            consumer.frames().iter().all(|frame| frame.is_solid(rgb(SOURCE))),
            "no time has passed on the session's clock, so the stream keeps the View's picture"
        );

        clock.advance(Duration::from_millis(500));
        owner.frames(3);
        consumer.wait_for_frame("of the repainted colour", |frame| frame.is_solid(rgb(REPAINTED)));
    });
}

// A View of a window whose buffer is a GPU buffer (a dmabuf). The GPU is the stand-in of `tests/mock_gpu`, which reads
// the client's plane back when the compositor asks for the picture, so what is asserted is the compositor's handling of
// the pixels it gets back: they are drawn, and a secure window's are masked. That a real GPU buffer reads back the
// right way up is the renderer's tests and `tests/dmabuf_capture_gpu.rs`.

/// The GPU-backed window's colour: opaque, as an `argb8888` word, and unlike the background, the mask and the shm
/// window's.
const GPU_COLOUR: u32 = 0xFF30_90D0;

/// A `side` x `side` GPU buffer of `word`, made with `create_immed`, with the plane file behind it.
fn gpu_buffer(
    qh: &QueueHandle<App>,
    global: &dmabuf_global::ZwpLinuxDmabufV1,
    side: u32,
    word: u32,
) -> (File, wl_buffer::WlBuffer) {
    let stride = side * 4;
    let file = tempfile::tempfile().expect("a plane file");
    file.set_len(u64::from(stride) * u64::from(side))
        .expect("a file that long");
    file.write_all_at(&word.to_ne_bytes().repeat((side * side) as usize), 0)
        .expect("pixels");
    let fd = std::os::fd::OwnedFd::from(file.try_clone().expect("a second handle"));
    let params = global.create_params(qh, ());
    params.add(fd.as_fd(), 0, 0, stride, 0, 0);
    let buffer = params.create_immed(
        side as i32,
        side as i32,
        smithay::backend::allocator::Fourcc::Argb8888 as u32,
        params::Flags::empty(),
        qh,
        (),
    );
    params.destroy();
    (file, buffer)
}

/// Maps a toplevel whose buffer is a GPU buffer of `word`, and returns what keeps it alive.
fn map_gpu_window(client: &mut Connected<App>, word: u32) -> (Window, File, wl_buffer::WlBuffer) {
    let wm_base: xdg_wm_base::XdgWmBase = client
        .globals
        .bind(&client.qh, 1..=1, ())
        .expect("the session advertises xdg_wm_base");
    let surface = client.compositor.create_surface(&client.qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &client.qh, ());
    let toplevel = xdg_surface.get_toplevel(&client.qh, ());
    toplevel.set_title("gpu view source".to_owned());
    surface.commit();
    client.roundtrip();
    let window = Window {
        surface,
        objects: (xdg_surface, toplevel),
        buffers: Vec::new(),
    };
    let global: dmabuf_global::ZwpLinuxDmabufV1 = client
        .globals
        .bind(&client.qh, 3..=3, ())
        .expect("the session advertises zwp_linux_dmabuf_v1");
    let (plane, buffer) = gpu_buffer(&client.qh, &global, SIDE, word);
    window.surface.attach(Some(&buffer), 0, 0);
    window.surface.damage(0, 0, SIDE as i32, SIDE as i32);
    window.surface.commit();
    client.roundtrip();
    (window, plane, buffer)
}

/// A session whose GPU is the stand-in, with the PipeWire daemon casts go to when there is one.
fn start_gpu(daemon: Option<&Daemon>, mock: mock_gpu::Mock) -> (Session, Owner) {
    let (hooks, answers) = hooks();
    let owner = owner_of(&hooks, answers);
    let extras = Extras {
        pipewire_socket: daemon.map(Daemon::socket),
        dmabuf_provider: Some(Box::new(mock)),
        ..Extras::default()
    };
    (Session::start_with_extras(None, hooks, extras), owner)
}

/// A client that reads the View into a GPU buffer: the privileged role, which sees the GPU and the view source both.
struct GpuViewer {
    client: Bare<App>,
    session: ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1,
    plane: File,
    buffer: wl_buffer::WlBuffer,
    #[allow(dead_code)]
    keep: (ExtImageCaptureSourceV1, dmabuf_global::ZwpLinuxDmabufV1),
}

impl GpuViewer {
    fn open(handle: &Handle) -> Self {
        let mut client = handle.connect_bare(ClientRole::Privileged, App::default());
        let sources: GnoblinViewSourceManagerV1 = client
            .globals
            .bind(&client.qh, 1..=1, ())
            .expect("the owner's role binds the view source manager");
        let copies: ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1 =
            client.globals.bind(&client.qh, 1..=1, ()).expect("the copy manager");
        let global: dmabuf_global::ZwpLinuxDmabufV1 = client.globals.bind(&client.qh, 3..=3, ()).expect("dmabuf");
        let source = sources.create_source((VIEW.get() >> 32) as u32, VIEW.get() as u32, &client.qh, ());
        let session = copies.create_session(
            &source,
            ext_image_copy_capture_manager_v1::Options::empty(),
            &client.qh,
            (),
        );
        client.wait_until("the capture session's constraints", |state| state.session_done >= 1);
        let (plane, buffer) = gpu_buffer(&client.qh, &global, SIDE, 0);
        Self {
            client,
            session,
            plane,
            buffer,
            keep: (source, global),
        }
    }

    /// Asks for one frame into the GPU buffer; `true` when it was delivered.
    fn capture(&mut self) -> bool {
        let (ready, failed) = (self.client.state.frame_ready, self.client.state.frame_failed);
        let frame = self.session.create_frame(&self.client.qh, ());
        frame.attach_buffer(&self.buffer);
        frame.capture();
        self.client.wait_until("an answer to the frame request", |state| {
            state.frame_ready > ready || state.frame_failed > failed
        });
        let delivered = self.client.state.frame_ready > ready;
        frame.destroy();
        delivered
    }

    /// How many pixels of the buffer are `word`.
    fn count_of(&self, word: u32) -> usize {
        let mut bytes = vec![0u8; (SIDE * SIDE * 4) as usize];
        self.plane.read_exact_at(&mut bytes, 0).expect("the plane");
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| u32::from_ne_bytes(**pixel) == word)
            .count()
    }

    /// Captures until every pixel of the buffer is `word`.
    fn capture_until(&mut self, word: u32) {
        let deadline = Instant::now() + TIMEOUT;
        while Instant::now() < deadline {
            if self.capture() && self.count_of(word) == (SIDE * SIDE) as usize {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("the GPU capture never showed {word:#010x}");
    }
}

/// The View's PNG at the shell's request, and its pixel at `(x, y)`.
fn png_of_view(handle: &Handle, owner: &Owner, name: &str, x: u32, y: u32) -> u32 {
    let runtime = std::fs::canonicalize(handle.socket().parent().expect("the socket's directory")).expect("a path");
    let target = runtime.join(name);
    let answer = owner.capture(VIEW, target.to_str().expect("a utf-8 path"));
    assert_eq!(
        answer.error.as_str(),
        "",
        "the capture call is answered with the path written"
    );
    png_pixel(&target, x, y).1
}

/// A View of a GPU-backed window that nobody marked secure shows the window's real pixels everywhere a View is read:
/// to a view-role session into shared memory, to a session into a GPU buffer, and in the PNG the shell asks for.
#[test]
fn a_view_of_a_gpu_window_shows_its_real_pixels_to_every_reader() {
    let (mock, gpu) = mock_gpu::Mock::capable();
    let (session, owner) = start_gpu(None, mock);
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let _window = map_gpu_window(&mut app, GPU_COLOUR);
        arrange(&owner);

        let mut viewer = Viewer::open(handle);
        viewer.capture_until(GPU_COLOUR);
        let mut gpu_viewer = GpuViewer::open(handle);
        gpu_viewer.capture_until(GPU_COLOUR);
        assert_eq!(
            png_of_view(handle, &owner, "gpu-view.png", SIDE / 2, SIDE / 2),
            GPU_COLOUR & 0x00FF_FFFF,
            "the PNG holds the GPU window's pixels, not a mask"
        );
        assert!(
            gpu.reads() > 0,
            "the pixels were read back from the client's GPU buffer"
        );
        assert!(
            !gpu.written().is_empty(),
            "the View's picture was written into the client's GPU buffer by the GPU"
        );
    });
}

/// A secure GPU-backed window is never in a View's capture, whatever reads it: the readback happens for the View's own
/// display, but the capture picture holds the mask.
#[test]
fn a_secure_gpu_window_never_reaches_a_capture_of_a_view() {
    let (mock, gpu) = mock_gpu::Mock::capable();
    let (session, owner) = start_gpu(None, mock);
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let _window = map_gpu_window(&mut app, GPU_COLOUR);
        arrange(&owner);
        let (window, ..) = owner.window();
        mark_secure(&owner, window, true);

        let mut viewer = Viewer::open(handle);
        viewer.capture_until(MASKED);
        assert_eq!(
            viewer.target.count_of(GPU_COLOUR),
            0,
            "no pixel of the window in the shm capture"
        );
        let mut gpu_viewer = GpuViewer::open(handle);
        gpu_viewer.capture_until(MASKED);
        assert_eq!(
            gpu_viewer.count_of(GPU_COLOUR),
            0,
            "no pixel of the window in the GPU capture"
        );
        let png = png_of_view(handle, &owner, "gpu-secure.png", SIDE / 2, SIDE / 2);
        assert_ne!(
            png,
            GPU_COLOUR & 0x00FF_FFFF,
            "the PNG does not hold the window's pixels"
        );
        assert_eq!(png, MASKED & 0x00FF_FFFF, "it holds the mask");
        assert!(
            gpu.reads() > 0,
            "the window was read for the View's own display, and still masked in the capture"
        );
    });
}

/// A cast of a View of a GPU-backed window delivers the window's pixels.
#[test]
fn a_cast_of_a_view_of_a_gpu_window_delivers_its_pixels() {
    let daemon = Daemon::start();
    let (mock, _gpu) = mock_gpu::Mock::capable();
    let (session, owner) = start_gpu(Some(&daemon), mock);
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let _window = map_gpu_window(&mut app, GPU_COLOUR);
        arrange(&owner);
        let stream = cast(handle, VIEW);
        let consumer = FrameConsumer::connect(&daemon.socket(), stream.node);
        consumer.wait_for_frame("of the GPU window's colour", |frame| frame.is_solid(rgb(GPU_COLOUR)));
    });
}

/// A cast of a View of a secure GPU-backed window delivers the mask and never a pixel of the window.
#[test]
fn a_cast_of_a_view_masks_a_secure_gpu_window() {
    let daemon = Daemon::start();
    let (mock, _gpu) = mock_gpu::Mock::capable();
    let (session, owner) = start_gpu(Some(&daemon), mock);
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let _window = map_gpu_window(&mut app, GPU_COLOUR);
        arrange(&owner);
        let (window, ..) = owner.window();
        mark_secure(&owner, window, true);
        let stream = cast(handle, VIEW);
        let consumer = FrameConsumer::connect(&daemon.socket(), stream.node);
        consumer.wait_for_frame("of the mask", |frame| frame.is_solid(rgb(MASKED)));
        consumer.forget();
        consumer.listen(Duration::from_millis(600));
        let frames = consumer.frames();
        assert!(!frames.is_empty(), "the stream keeps delivering frames while masked");
        assert!(
            frames.iter().all(|frame| frame.count_of(rgb(GPU_COLOUR)) == 0),
            "no frame holds a pixel of the secure GPU window"
        );
    });
}

macro_rules! ignore_gpu_events {
    ($($interface:ty),* $(,)?) => {$(
        impl Dispatch<$interface, ()> for App {
            fn event(
                _: &mut Self,
                _: &$interface,
                _: <$interface as wayland_client::Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}

ignore_gpu_events!(dmabuf_global::ZwpLinuxDmabufV1, params::ZwpLinuxBufferParamsV1);

// A View drawn on the GPU. The session draws with the GLES renderer, so the composer describes each View as a scene the
// renderer draws into a texture and keeps no picture of it on the CPU. Everything a consumer reads is a readback made
// once per revision, only for a View something is reading, in the loop turn the request arrives in. The same cases as
// above run against it, and what they assert is the same.

/// A session on the GLES renderer, or `None` (with the reason printed) on a machine that has no surfaceless EGL context.
fn start_gles(observer: Option<Arc<dyn Observer>>) -> Option<(Session, Owner)> {
    use gnoblin_backend::render::renderer::{AnyRenderer, RendererKind};
    if let Err(error) = AnyRenderer::new(RendererKind::Gles, gnoblin_types::Size::new(8, 8)) {
        println!("skipping the GPU View test: {error} (no surfaceless EGL context on this machine)");
        return None;
    }
    let (hooks, answers) = hooks();
    let owner = owner_of(&hooks, answers);
    let extras = Extras {
        observer,
        gles: true,
        ..Extras::default()
    };
    Some((Session::start_with_extras(None, hooks, extras), owner))
}

/// A window's pixels reach a `view`-role client, and a repainted window is delivered again, with no picture of the View
/// kept on the CPU for the display.
#[test]
fn a_view_drawn_on_the_gpu_is_read_by_a_view_role_client_and_follows_its_source() {
    let Some((session, owner)) = start_gles(None) else {
        return;
    };
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let mut window = Window::map(&mut app, SOURCE);
        arrange(&owner);

        // The session was opened after the View was drawn, so its first frame waits for the read-back of this turn.
        let mut viewer = Viewer::open(handle);
        viewer.capture_until(SOURCE);
        let last = SIDE - 1;
        for (x, y) in [(0, 0), (SIDE / 2, SIDE / 2), (last, 0), (0, last), (last, last)] {
            assert_eq!(
                viewer.target.word(x, y) & 0x00FF_FFFF,
                SOURCE & 0x00FF_FFFF,
                "pixel ({x}, {y}) of the capture is the window's colour"
            );
        }
        window.paint(&mut app, REPAINTED);
        viewer.capture_until(REPAINTED);
    });
}

/// A canvas inside a View is captured with its painted pixels, and again when its content is written again.
#[test]
fn a_canvas_in_a_view_drawn_on_the_gpu_is_captured_with_its_painted_pixels() {
    let Some((session, owner)) = start_gles(None) else {
        return;
    };
    session.run(move |handle| {
        arrange_canvas(&owner);
        let mut viewer = Viewer::open(handle);
        viewer.capture_until_pixel(SIDE - 2, SIDE - 2, SOURCE);
        assert_eq!(
            viewer.differing(0, 24, SIDE, SIDE - 24, SOURCE),
            0,
            "below the text and the icon is fill"
        );
        let text = viewer.differing(2, 0, 30, 24, SOURCE);
        assert!(text >= 20, "the text paints pixels over the fill: {text} differ");
    });
}

/// The shell's `capture` writes a PNG of a View drawn on the GPU: it waits for the readback of the turn it was made
/// in, is answered with the path, and the capture session is shown open until the file exists.
#[test]
fn a_still_of_a_view_drawn_on_the_gpu_is_written_and_answered() {
    let recorder = Arc::new(Recorder::default());
    let Some((session, owner)) = start_gles(Some(recorder.clone())) else {
        return;
    };
    session.run(move |handle| {
        arrange_canvas(&owner);
        let runtime = std::fs::canonicalize(handle.socket().parent().expect("the socket's directory")).expect("a path");
        let target = runtime.join("gpu-view.png");
        recorder.target.set(target.clone()).expect("the target is set once");
        let answer = owner.capture(VIEW, target.to_str().expect("a utf-8 path"));
        assert_eq!(
            (answer.error.as_str(), answer.result.as_str()),
            ("", target.to_str().expect("a utf-8 path")),
            "the shell is answered with the path written"
        );
        let (size, colour) = png_pixel(&target, SIDE - 2, SIDE - 2);
        assert_eq!(size, (SIDE, SIDE), "the PNG is the View's size");
        assert_eq!(colour, SOURCE & 0x00FF_FFFF, "the PNG holds the View's pixels");
        let seen = recorder.seen.lock().expect("the recorder lock").clone();
        assert_eq!(
            seen.last(),
            Some(&(0, true)),
            "the capture closes once the file exists: {seen:?}"
        );
    });
}

/// A secure window's pixels are masked in the PNG of a View drawn on the GPU, and come back when the mark is cleared,
/// while the display keeps showing them: the capture reads the scene's redacted variant, never the display texture.
#[test]
fn a_png_of_a_view_drawn_on_the_gpu_holds_none_of_a_secure_windows_pixels() {
    let Some((session, owner)) = start_gles(None) else {
        return;
    };
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let _window = Window::map(&mut app, SOURCE);
        arrange(&owner);
        let (window, ..) = owner.window();
        let source = SOURCE & 0x00FF_FFFF;

        assert!(
            still_of(&owner, handle, VIEW, "open.png")
                .iter()
                .all(|word| *word == source),
            "an unmarked window is captured as it is drawn"
        );
        mark_secure(&owner, window, true);
        assert!(
            still_of(&owner, handle, VIEW, "secure.png")
                .iter()
                .all(|word| *word == MASKED & 0x00FF_FFFF),
            "every pixel of the secure window is the mask"
        );
        mark_secure(&owner, window, false);
        assert!(
            still_of(&owner, handle, VIEW, "cleared.png")
                .iter()
                .all(|word| *word == source),
            "a cleared mark shows the window again"
        );
    });
}

/// A `view`-role session's frames never carry a secure window's pixels once the mark is on, however the View is drawn.
#[test]
fn a_view_role_session_of_a_view_drawn_on_the_gpu_never_receives_a_secure_windows_pixels() {
    let Some((session, owner)) = start_gles(None) else {
        return;
    };
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let _window = Window::map(&mut app, SOURCE);
        arrange(&owner);
        let (window, ..) = owner.window();
        let mut viewer = Viewer::open(handle);
        viewer.capture_until(SOURCE);
        mark_secure(&owner, window, true);
        viewer.capture_until(MASKED);
        assert_eq!(
            viewer.target.count_of(SOURCE),
            0,
            "no pixel of the secure window is in the frame"
        );
    });
}

// Secure content in a View on both renderers, and the session lock. Each case runs on the software renderer (the View is
// a picture the composer draws on the CPU) and on GLES (the View is a texture the renderer draws, read back for a
// reader): what a reader may see is the same, so the case is the same. A machine with no surfaceless EGL context runs
// the software half only.

/// A session on the renderer named, with a PipeWire daemon when one is given and the lock's password check when asked.
/// `None` (with the reason printed) for GLES on a machine that has no surfaceless EGL context.
fn start_for(gles: bool, daemon: Option<&Daemon>, lock_check: bool) -> Option<(Session, Owner)> {
    use gnoblin_backend::render::renderer::{AnyRenderer, RendererKind};
    if gles && let Err(error) = AnyRenderer::new(RendererKind::Gles, gnoblin_types::Size::new(8, 8)) {
        println!("skipping the GPU half of the case: {error} (no surfaceless EGL context on this machine)");
        return None;
    }
    let (hooks, answers) = hooks();
    let owner = owner_of(&hooks, answers);
    let extras = Extras {
        gles,
        pipewire_socket: daemon.map(Daemon::socket),
        lock_check: lock_check.then(|| gnoblin_wayland::LockCheck {
            authenticator: Arc::new(gnoblin_lock_auth::FakeAuthenticator::new().accept(USER, PASSWORD)),
            user: Some(USER.to_owned()),
        }),
        ..Extras::default()
    };
    Some((Session::start_with_extras(None, hooks, extras), owner))
}

/// Writes `secure` on `node` and returns when the shell is answered, with no frame waited for after it: the next frame
/// a reader asks for is the first one after the compositor acknowledged the mark.
fn mark_secure_acknowledged(owner: &Owner, node: NodeId, secure: bool) {
    owner.transact(|| {
        vec![TreeOp::Set {
            node,
            key: "secure".to_owned(),
            value: TreeValue::Bool(secure),
            timing: None,
        }]
    });
}

/// A node the shell puts above the View's source, which the mark below goes on.
const ABOVE: NodeId = NodeId::new(0x108);

/// A frame a reader asks for the moment the shell is told a node is secure holds none of that node's pixels, whichever
/// renderer draws the View: the picture a reader copies is never older than the mark the shell was answered for.
#[test]
fn the_first_frame_a_reader_asks_for_after_a_mark_is_acknowledged_holds_none_of_the_pixels() {
    for gles in [false, true] {
        let Some((session, owner)) = start_for(gles, None, false) else {
            continue;
        };
        session.run(move |handle| {
            let mut app = Connected::connect(handle.socket(), App::default());
            let _window = Window::map(&mut app, SOURCE);
            arrange(&owner);
            let (window, ..) = owner.window();
            let mut viewer = Viewer::open(handle);
            viewer.capture_until(SOURCE);

            mark_secure_acknowledged(&owner, window, true);
            for ask in 0..6 {
                if viewer.capture() {
                    assert_eq!(
                        viewer.target.count_of(SOURCE),
                        0,
                        "gles: {gles}: frame {ask} after the acknowledged mark holds a pixel of the secure window"
                    );
                }
            }
            viewer.capture_until(MASKED);
        });
    }
}

/// Everything below a secure node is secure with it, also when the node is above the View's source and not in it: the
/// session, the stream and the PNG all read the mask, and the window shows again when the mark is cleared.
#[test]
fn a_node_above_the_views_source_that_is_secure_masks_every_reader_of_the_view() {
    for gles in [false, true] {
        let daemon = Daemon::start();
        let Some((session, owner)) = start_for(gles, Some(&daemon), false) else {
            continue;
        };
        session.run(move |handle| {
            let mut app = Connected::connect(handle.socket(), App::default());
            let _window = Window::map(&mut app, SOURCE);
            arrange(&owner);
            let (_, _, root) = owner.window();
            put_above(&owner, root);
            let mut viewer = Viewer::open(handle);
            viewer.capture_until(SOURCE);
            let stream = cast(handle, VIEW);
            let consumer = FrameConsumer::connect(&daemon.socket(), stream.node);
            consumer.wait_for_frame("of the window's colour", |frame| frame.is_solid(rgb(SOURCE)));

            mark_secure(&owner, ABOVE, true);
            viewer.capture_until(MASKED);
            assert!(
                still_of(&owner, handle, VIEW, "above.png")
                    .iter()
                    .all(|word| *word == MASKED & 0x00FF_FFFF),
                "gles: {gles}: the PNG of a View whose source is below a secure node is the mask"
            );
            consumer.wait_for_frame("of the mask", |frame| frame.is_solid(rgb(MASKED)));
            consumer.forget();
            consumer.listen(Duration::from_millis(400));
            assert!(
                consumer.frames().iter().all(|frame| frame.count_of(rgb(SOURCE)) == 0),
                "gles: {gles}: no frame of the stream holds a pixel of the window"
            );

            mark_secure(&owner, ABOVE, false);
            viewer.capture_until(SOURCE);
        });
    }
}

/// While the session is locked no reader of a View is served: the session is stopped, the stream ends, a still is
/// refused and writes nothing, and a cast or a session opened during the lock gets nothing. A window that repaints
/// behind the lock is what a reader sees after the unlock, never the picture that was current before it.
#[test]
fn the_lock_cuts_every_reader_of_a_view_and_none_is_served_a_picture_from_before_it() {
    for gles in [false, true] {
        let daemon = Daemon::start();
        let Some((session, owner)) = start_for(gles, Some(&daemon), true) else {
            continue;
        };
        session.run(move |handle| {
            let mut app = Connected::connect(handle.socket(), App::default());
            let mut window = Window::map(&mut app, SOURCE);
            arrange(&owner);
            let mut viewer = Viewer::open(handle);
            viewer.capture_until(SOURCE);
            let stream = cast(handle, VIEW);
            let consumer = FrameConsumer::connect(&daemon.socket(), stream.node);
            consumer.wait_for_frame("of the window's colour", |frame| frame.is_solid(rgb(SOURCE)));
            assert!(
                still_of(&owner, handle, VIEW, "before-lock.png")
                    .iter()
                    .all(|word| *word == SOURCE & 0x00FF_FFFF),
                "the control: before the lock a still is the window"
            );

            let _locker = lock(handle.socket());
            let node = lock_node(&owner);
            wait_lock_state(&owner, node, "locked");
            viewer
                .client
                .wait_until("the capture session to stop", |state| state.session_stopped >= 1);
            assert!(!viewer.capture(), "gles: {gles}: a stopped session delivers no frame");
            assert!(
                consumer.observer.wait_gone(&node_name(&stream)),
                "gles: {gles}: the lock ends the stream"
            );
            let runtime =
                std::fs::canonicalize(handle.socket().parent().expect("the socket's directory")).expect("a path");
            let locked = runtime.join("locked.png");
            let answer = owner.capture(VIEW, locked.to_str().expect("a utf-8 path"));
            assert_eq!(
                answer.error, "locked",
                "gles: {gles}: a still during the lock is refused"
            );
            assert!(!locked.exists(), "gles: {gles}: and writes nothing");
            assert_eq!(
                handle
                    .control
                    .start_cast(PORTAL, gnoblin_capture::Target::View(VIEW))
                    .expect("the loop answers"),
                Err(gnoblin_wayland::CastRefusal::Locked),
                "gles: {gles}: a cast of the locked desktop is refused"
            );
            // The window repaints behind the lock; the picture of before it must not be served after it.
            window.paint(&mut app, REPAINTED);
            owner.frames(3);
            let answer = owner.call(node, "authenticate", PASSWORD);
            assert_eq!((answer.error.as_str(), answer.result.as_str()), ("", "ok"));
            wait_lock_state(&owner, node, "not-locked");
            let mut fresh = Viewer::open(handle);
            for ask in 0..5 {
                if fresh.capture() {
                    assert_eq!(
                        fresh.target.count_of(SOURCE),
                        0,
                        "gles: {gles}: frame {ask} after the unlock holds a pixel of the picture from before the lock"
                    );
                }
                thread::sleep(Duration::from_millis(10));
            }
            fresh.capture_until(REPAINTED);
            assert!(
                still_of(&owner, handle, VIEW, "after-unlock.png")
                    .iter()
                    .all(|word| *word == REPAINTED & 0x00FF_FFFF),
                "gles: {gles}: a still after the unlock is the window as it is now"
            );
        });
    }
}

/// Puts the shell node `ABOVE` between the root and the View's source, and waits for the frames that apply it.
fn put_above(owner: &Owner, root: NodeId) {
    owner.transact(|| {
        vec![
            TreeOp::New {
                class: "node".to_owned(),
                id: ABOVE,
            },
            TreeOp::Insert {
                node: ABOVE,
                parent: root,
                index: None,
            },
            TreeOp::Insert {
                node: PARENT,
                parent: ABOVE,
                index: None,
            },
        ]
    });
    owner.frames(3);
}

// The PipeWire transition, on the real daemon. A consumer that reads slowly leaves the buffers the producer filled
// waiting for it. The rule the tests state:
//
// * A buffer is filled from the picture published at the end of the frame that last applied a mark, never before it.
// * The producer runs the stream's cycle in the pump that fills a buffer, so no buffer it filled waits on its side
//   across a mark or the lock. Before this, the last buffer filled waited for the next pump, and a consumer that took
//   nothing in between was handed the picture from before the mark (1 of 8 buffers in the first run of the test).
// * What a consumer already took before the acknowledgement is the consumer's own, and the producer cannot recall it.
// * The lock ends the stream, which takes its queue with it.
//
// The consumer below takes nothing between its first frame and the acknowledgement, so every buffer it takes
// afterwards was filled before the acknowledgement and not yet taken, or after it.

/// Where the secure mark goes.
#[derive(Clone, Copy, Debug)]
enum Mark {
    /// The window's own node.
    Source,
    /// A shell node above the View's source.
    Ancestor,
}

/// What the cast carries.
#[derive(Clone, Copy, Debug)]
enum Cast {
    View,
    Output,
}

/// Whether a frame holds a pixel of the window, which is the colour only the window draws.
fn shows_window(frame: &gnoblin_pipewire_export::testing::ReceivedFrame) -> bool {
    frame.count_of(rgb(SOURCE)) > 0
}

/// A cast of `cast` whose consumer is slow, a mark of `mark` acknowledged while buffers are queued: no buffer the consumer
/// takes afterwards holds a pixel of the window.
#[test]
fn no_buffer_queued_before_a_mark_is_delivered_after_it_on_a_real_cast() {
    for gles in [false, true] {
        for cast in [Cast::View, Cast::Output] {
            for mark in [Mark::Source, Mark::Ancestor] {
                let daemon = Daemon::start();
                let Some((session, owner)) = start_for(gles, Some(&daemon), false) else {
                    continue;
                };
                session.run(move |handle| {
                    let label = format!("gles: {gles}, {cast:?}, {mark:?}");
                    let mut app = Connected::connect(handle.socket(), App::default());
                    let _window = Window::map(&mut app, SOURCE);
                    arrange(&owner);
                    let (window, _, root) = owner.window();
                    let marked = match mark {
                        Mark::Source => window,
                        Mark::Ancestor => {
                            put_above(&owner, root);
                            ABOVE
                        }
                    };
                    let target = match cast {
                        Cast::View => gnoblin_capture::Target::View(VIEW),
                        Cast::Output => gnoblin_capture::Target::Output(gnoblin_types::OutputId::new(0)),
                    };
                    let stream = handle
                        .control
                        .start_cast(PORTAL, target)
                        .expect("the loop answers a stream start")
                        .expect("the target is cast");
                    let consumer = FrameConsumer::connect(&daemon.socket(), stream.node);
                    consumer.wait_for_frame("of the window", shows_window);

                    // The consumer reads nothing now, and the producer goes on filling buffers: they queue.
                    thread::sleep(Duration::from_millis(600));
                    mark_secure_acknowledged(&owner, marked, true);
                    // From here on, whatever the consumer takes was queued before the acknowledgement or filled after it.
                    consumer.forget();
                    owner.frames(3);
                    consumer.listen(Duration::from_millis(900));
                    let frames = consumer.frames();
                    let leaked = frames.iter().filter(|frame| shows_window(frame)).count();
                    assert_eq!(
                        leaked,
                        0,
                        "{label}: {leaked} of {} buffers delivered after the acknowledged mark hold a pixel of the window",
                        frames.len()
                    );
                    assert!(
                        frames.iter().any(|frame| frame.count_of(rgb(MASKED)) > 0),
                        "{label}: the stream goes on, with the mask in the window's place"
                    );
                });
            }
        }
    }
}

/// A mark cleared again shows the window in the stream, so the discard cost nothing but the frames it dropped.
#[test]
fn a_cast_shows_the_window_again_when_the_mark_is_cleared() {
    let daemon = Daemon::start();
    let Some((session, owner)) = start_for(false, Some(&daemon), false) else {
        return;
    };
    session.run(move |handle| {
        let mut app = Connected::connect(handle.socket(), App::default());
        let _window = Window::map(&mut app, SOURCE);
        arrange(&owner);
        let (window, ..) = owner.window();
        let stream = cast(handle, VIEW);
        let consumer = FrameConsumer::connect(&daemon.socket(), stream.node);
        consumer.wait_for_frame("of the window", shows_window);
        mark_secure_acknowledged(&owner, window, true);
        consumer.wait_for_frame("of the mask", |frame| frame.is_solid(rgb(MASKED)));
        mark_secure_acknowledged(&owner, window, false);
        consumer.wait_for_frame("of the window again", |frame| frame.is_solid(rgb(SOURCE)));
    });
}

/// The lock with buffers queued: nothing the producer filled before the lock reaches a consumer after it, and the
/// stream's node is gone.
#[test]
fn no_buffer_queued_before_the_lock_is_delivered_after_it_on_a_real_cast() {
    for gles in [false, true] {
        for cast in [Cast::View, Cast::Output] {
            let daemon = Daemon::start();
            let Some((session, owner)) = start_for(gles, Some(&daemon), true) else {
                continue;
            };
            session.run(move |handle| {
                let label = format!("gles: {gles}, {cast:?}");
                let mut app = Connected::connect(handle.socket(), App::default());
                let _window = Window::map(&mut app, SOURCE);
                arrange(&owner);
                let target = match cast {
                    Cast::View => gnoblin_capture::Target::View(VIEW),
                    Cast::Output => gnoblin_capture::Target::Output(gnoblin_types::OutputId::new(0)),
                };
                let stream = handle
                    .control
                    .start_cast(PORTAL, target)
                    .expect("the loop answers a stream start")
                    .expect("the target is cast");
                let consumer = FrameConsumer::connect(&daemon.socket(), stream.node);
                consumer.wait_for_frame("of the window", shows_window);
                thread::sleep(Duration::from_millis(600));

                let _locker = lock(handle.socket());
                let node = lock_node(&owner);
                wait_lock_state(&owner, node, "locked");
                consumer.forget();
                assert!(
                    consumer.observer.wait_gone(&node_name(&stream)),
                    "{label}: the lock ends the stream: its node is gone"
                );
                consumer.listen(Duration::from_millis(600));
                let frames = consumer.frames();
                let leaked = frames.iter().filter(|frame| shows_window(frame)).count();
                assert_eq!(
                    leaked,
                    0,
                    "{label}: {leaked} of {} buffers delivered after the lock hold a pixel of the window",
                    frames.len()
                );
            });
        }
    }
}
