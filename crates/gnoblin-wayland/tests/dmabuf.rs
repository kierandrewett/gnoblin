//! `zwp_linux_dmabuf_v1` against the real compositor, with a stand-in importer.
//!
//! The protocol layer under test is the real one: Smithay's parameter validation, the global's
//! version and feedback, the compositor's own checks, the queue that answers each buffer once per
//! loop turn, the surface machinery that maps, damages, releases and unmaps a window whose buffer
//! is a GPU buffer. What is *not* real is the GPU. The Agent37 build instances have no DRM
//! device (`/dev/dri`, `/dev/udmabuf` and `/dev/dma_heap` are absent in their gVisor sandbox), so
//! no process there can allocate a dmabuf and no driver can import one. A `DmabufProvider` stands in
//! for the renderer: it names the formats and device a GPU would and accepts or refuses each
//! buffer. The plane files are ordinary files of the right size, which is all Smithay's checks read.
//!
//! What a real GPU adds is covered by `tests/dmabuf_gpu.rs`, which allocates a GBM buffer, shares it
//! and reads the compositor's pixels back; it skips, loudly, on a machine with no render node.
//!
//! Failure modes, each with its test here (the list is in `protocols::zwp_linux_dmabuf_v1`):
//!
//! - `a_software_session_does_not_serve_dmabuf`
//! - `a_modifier_that_was_not_offered_is_refused`, `a_wrong_plane_count_is_refused`
//! - `a_buffer_the_importer_refuses_fails_the_request`
//! - `a_zero_sized_buffer_is_a_protocol_error`, `an_unoffered_format_is_a_protocol_error`,
//!   `a_second_use_of_the_params_is_a_protocol_error`, `an_incomplete_params_is_a_protocol_error`,
//!   `a_plane_that_does_not_fit_its_file_is_a_protocol_error`
//! - `a_version_4_client_is_told_the_main_device`, `a_version_3_client_gets_formats_and_modifiers`
//! - the surface life of a dmabuf: `a_dmabuf_window_maps_releases_and_unmaps`
//! - the fence wait: `a_commit_waits_for_the_buffers_fences`

use std::io::Read;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::Headless;
use gnoblin_backend::{DmabufImportError, DmabufProvider, DmabufSupport};
use gnoblin_render_frame::error::RenderError;
use gnoblin_wayland::{Config, ControlHandle, LoopError, Server, SessionSnapshot};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Buffer as _, Format, Fourcc, Modifier};
use wayland_client::backend::WaylandError;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_callback, wl_compositor, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, DispatchError, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1 as params, zwp_linux_dmabuf_feedback_v1 as feedback,
    zwp_linux_dmabuf_v1 as dmabuf_global,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// How long a test waits for the session to reach a state before failing.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The render node number the stand-in reports: major 226 (DRM), minor 128.
const DEVICE: u64 = (226 << 8) | 128;

/// The pairs the stand-in offers.
fn offered() -> Vec<Format> {
    vec![
        Format {
            code: Fourcc::Argb8888,
            modifier: Modifier::Linear,
        },
        Format {
            code: Fourcc::Xrgb8888,
            modifier: Modifier::Linear,
        },
        Format {
            code: Fourcc::Xrgb8888,
            modifier: Modifier::Invalid,
        },
    ]
}

/// What the stand-in importer saw and what it answers.
#[derive(Default)]
struct Imports {
    /// `(width, height, format)` of every buffer that reached the importer.
    seen: Vec<(i32, i32, Fourcc)>,
    /// Whether the importer refuses what it is asked.
    refuse: bool,
}

struct Mock {
    device: Option<u64>,
    imports: Arc<Mutex<Imports>>,
}

impl DmabufProvider for Mock {
    fn support(&mut self) -> Option<DmabufSupport> {
        Some(DmabufSupport::new(self.device, offered()))
    }

    fn import(&mut self, dmabuf: &Dmabuf) -> Result<(), DmabufImportError> {
        let mut imports = self.imports.lock().expect("the import log");
        imports
            .seen
            .push((dmabuf.size().w, dmabuf.size().h, dmabuf.format().code));
        if imports.refuse {
            Err(DmabufImportError::Refused(RenderError::Failed))
        } else {
            Ok(())
        }
    }
}

/// What a worker thread can reach of a running session.
#[derive(Clone)]
struct Handle {
    socket: PathBuf,
    control: ControlHandle,
    observation: Arc<Mutex<SessionSnapshot>>,
    imports: Arc<Mutex<Imports>>,
}

impl Handle {
    fn wait_for(&self, predicate: impl Fn(&SessionSnapshot) -> bool) -> SessionSnapshot {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let observation = self.observation.lock().expect("the observation lock").clone();
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

    fn seen(&self) -> Vec<(i32, i32, Fourcc)> {
        self.imports.lock().expect("the import log").seen.clone()
    }

    fn refuse_imports(&self, refuse: bool) {
        self.imports.lock().expect("the import log").refuse = refuse;
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
    /// A session whose backend has no GPU: the software renderer.
    fn software() -> Self {
        Self::start(Headless::new(), Arc::default())
    }

    /// A session whose importer is the stand-in: `device` is the render node it reports.
    fn with_importer(device: Option<u64>) -> Self {
        let imports = Arc::new(Mutex::new(Imports::default()));
        let provider = Mock {
            device,
            imports: Arc::clone(&imports),
        };
        Self::start(Headless::new().with_dmabuf_provider(Box::new(provider)), imports)
    }

    fn start(backend: Headless, imports: Arc<Mutex<Imports>>) -> Self {
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
            imports,
        };
        Self {
            directory,
            server,
            handle,
        }
    }

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

/// What the test's client records.
#[derive(Default)]
struct App {
    /// `(format, modifier)` of every `format` and `modifier` event of a version 3 global.
    formats: Vec<(u32, u64)>,
    /// The feedback the global sent: the main device and the number of table entries.
    main_device: Option<u64>,
    table_entries: Option<u32>,
    tranche_indices: Vec<u16>,
    feedback_done: bool,
    /// The buffer a `created` event delivered, and how many `failed` events arrived.
    created: Option<wl_buffer::WlBuffer>,
    failed: u32,
    /// The buffers that were released, by protocol id.
    released: Vec<u32>,
    callbacks: u32,
}

struct Client {
    #[allow(dead_code)]
    connection: Connection,
    queue: EventQueue<App>,
    state: App,
    qh: QueueHandle<App>,
    globals: wayland_client::globals::GlobalList,
}

impl Client {
    fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("the session is listening");
        let connection = Connection::from_socket(stream).expect("a client connection");
        let (globals, queue) = registry_queue_init::<App>(&connection).expect("the global list");
        let qh = queue.handle();
        Self {
            connection,
            queue,
            state: App::default(),
            qh,
            globals,
        }
    }

    fn roundtrip(&mut self) {
        self.queue.roundtrip(&mut self.state).expect("a round trip");
    }

    fn has_dmabuf(&self) -> bool {
        self.globals
            .contents()
            .with_list(|list| list.iter().any(|global| global.interface == "zwp_linux_dmabuf_v1"))
    }

    /// The version the compositor advertises `zwp_linux_dmabuf_v1` at.
    fn dmabuf_version(&self) -> Option<u32> {
        self.globals.contents().with_list(|list| {
            list.iter()
                .find(|global| global.interface == "zwp_linux_dmabuf_v1")
                .map(|global| global.version)
        })
    }

    fn dmabuf(&self, version: u32) -> dmabuf_global::ZwpLinuxDmabufV1 {
        self.globals
            .bind(&self.qh, version..=version, ())
            .expect("the session advertises zwp_linux_dmabuf_v1")
    }

    /// Round-trips until the server closes the connection with a protocol error, and returns the
    /// error's interface and code. The server answers a buffer on its next loop turn, so the error
    /// may arrive after the first round trip.
    fn protocol_error(&mut self) -> (String, u32) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match self.queue.roundtrip(&mut self.state) {
                Ok(_) => {}
                Err(DispatchError::Backend(WaylandError::Protocol(error))) => {
                    return (error.object_interface.to_string(), error.code);
                }
                Err(other) => panic!("the round trip failed for another reason: {other}"),
            }
            assert!(Instant::now() < deadline, "the server never posted a protocol error");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_until(&mut self, done: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        while !done(&self.state) {
            assert!(Instant::now() < deadline, "the client never saw the expected events");
            self.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// A plane file of `length` bytes: an ordinary file, which Smithay measures and nothing reads.
    fn plane(length: u64) -> OwnedFd {
        let file = tempfile::tempfile().expect("a plane file");
        file.set_len(length).expect("a file that long");
        OwnedFd::from(file)
    }

    /// Params with one plane of a `width` x `height` buffer at `stride`.
    fn params_with_plane(
        &self,
        global: &dmabuf_global::ZwpLinuxDmabufV1,
        width: i32,
        height: i32,
        modifier: Modifier,
    ) -> params::ZwpLinuxBufferParamsV1 {
        let stride = width as u32 * 4;
        let params = global.create_params(&self.qh, ());
        let fd = Self::plane(u64::from(stride) * height.max(1) as u64);
        let modifier = u64::from(modifier);
        params.add(fd.as_fd(), 0, 0, stride, (modifier >> 32) as u32, modifier as u32);
        params
    }

    /// Maps a toplevel with a dmabuf buffer of `width` x `height`, made with `create_immed`.
    fn mapped_dmabuf_toplevel(
        &mut self,
        global: &dmabuf_global::ZwpLinuxDmabufV1,
        width: i32,
        height: i32,
    ) -> (wl_surface::WlSurface, xdg_toplevel::XdgToplevel) {
        let compositor: wl_compositor::WlCompositor = self.globals.bind(&self.qh, 1..=6, ()).expect("wl_compositor");
        let wm_base: xdg_wm_base::XdgWmBase = self.globals.bind(&self.qh, 1..=6, ()).expect("xdg_wm_base");
        let surface = compositor.create_surface(&self.qh, ());
        let xdg = wm_base.get_xdg_surface(&surface, &self.qh, ());
        let toplevel = xdg.get_toplevel(&self.qh, ());
        toplevel.set_title("dmabuf window".to_owned());
        surface.commit();
        self.roundtrip();
        let buffer = self.immed_buffer(global, width, height);
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, width, height);
        surface.commit();
        self.roundtrip();
        (surface, toplevel)
    }

    /// A buffer made with `create_immed` from an offered pair.
    fn immed_buffer(&self, global: &dmabuf_global::ZwpLinuxDmabufV1, width: i32, height: i32) -> wl_buffer::WlBuffer {
        let params = self.params_with_plane(global, width, height, Modifier::Linear);
        let buffer = params.create_immed(
            width,
            height,
            Fourcc::Argb8888 as u32,
            params::Flags::empty(),
            &self.qh,
            (),
        );
        params.destroy();
        buffer
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

impl Dispatch<wl_compositor::WlCompositor, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_compositor::WlCompositor,
        _: wl_compositor::Event,
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
        _: wl_surface::Event,
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
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.callbacks += 1;
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for App {
    fn event(
        state: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            state.released.push(buffer.id().protocol_id());
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

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for App {
    fn event(
        _: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        _: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<dmabuf_global::ZwpLinuxDmabufV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &dmabuf_global::ZwpLinuxDmabufV1,
        event: dmabuf_global::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            dmabuf_global::Event::Modifier {
                format,
                modifier_hi,
                modifier_lo,
            } => state
                .formats
                .push((format, (u64::from(modifier_hi) << 32) | u64::from(modifier_lo))),
            dmabuf_global::Event::Format { .. } => {}
            _ => {}
        }
    }
}

impl Dispatch<params::ZwpLinuxBufferParamsV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &params::ZwpLinuxBufferParamsV1,
        event: params::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            params::Event::Created { buffer } => state.created = Some(buffer),
            params::Event::Failed => state.failed += 1,
            _ => {}
        }
    }

    wayland_client::event_created_child!(App, params::ZwpLinuxBufferParamsV1, [
        params::EVT_CREATED_OPCODE => (wl_buffer::WlBuffer, ()),
    ]);
}

impl Dispatch<feedback::ZwpLinuxDmabufFeedbackV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &feedback::ZwpLinuxDmabufFeedbackV1,
        event: feedback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            feedback::Event::MainDevice { device } => {
                state.main_device = Some(u64::from_ne_bytes(device.try_into().expect("a dev_t")));
            }
            feedback::Event::FormatTable { fd, size } => {
                let mut table = Vec::new();
                std::fs::File::from(fd)
                    .take(u64::from(size))
                    .read_to_end(&mut table)
                    .expect("the format table is readable");
                // Each entry is a format (u32), padding (u32) and a modifier (u64).
                state.table_entries = Some(u32::try_from(table.len() / 16).expect("a small table"));
            }
            feedback::Event::TrancheFormats { indices } => {
                state.tranche_indices = indices
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| u16::from_ne_bytes(*pair))
                    .collect();
            }
            feedback::Event::Done => state.feedback_done = true,
            _ => {}
        }
    }
}

#[test]
fn a_software_session_does_not_serve_dmabuf() {
    let session = Session::software();
    assert!(
        !session.server.state().serves_dmabuf(),
        "the software renderer cannot import a GPU buffer, so the session must not promise it"
    );
    session.run(|handle| {
        let client = Client::connect(&handle.socket);
        assert!(!client.has_dmabuf(), "the registry must not list zwp_linux_dmabuf_v1");
    });
}

#[test]
fn a_version_4_client_is_told_the_main_device() {
    let session = Session::with_importer(Some(DEVICE));
    assert!(session.server.state().serves_dmabuf());
    session.run(|handle| {
        let mut client = Client::connect(&handle.socket);
        assert!(
            client.dmabuf_version().is_some_and(|version| version >= 4),
            "a session with a render node serves feedback: version 4 or later"
        );
        let global = client.dmabuf(4);
        let _feedback = global.get_default_feedback(&client.qh, ());
        client.wait_until(|app| app.feedback_done);
        assert_eq!(
            client.state.main_device,
            Some(DEVICE),
            "the main device is the renderer's node"
        );
        assert_eq!(
            client.state.table_entries,
            Some(offered().len() as u32),
            "the format table has one entry per offered pair"
        );
        assert_eq!(
            client.state.tranche_indices.len(),
            offered().len(),
            "the main tranche names every offered pair"
        );
    });
}

#[test]
fn a_version_3_client_gets_formats_and_modifiers() {
    let session = Session::with_importer(Some(DEVICE));
    session.run(|handle| {
        let mut client = Client::connect(&handle.socket);
        let _global = client.dmabuf(3);
        client.roundtrip();
        let mut seen = client.state.formats.clone();
        seen.sort_unstable();
        let mut expected: Vec<(u32, u64)> = offered()
            .iter()
            .map(|pair| (pair.code as u32, u64::from(pair.modifier)))
            .collect();
        expected.sort_unstable();
        assert_eq!(seen, expected, "version 3 lists every offered pair as a modifier event");
    });
}

#[test]
fn a_session_without_a_render_node_serves_version_3() {
    let session = Session::with_importer(None);
    session.run(|handle| {
        let client = Client::connect(&handle.socket);
        assert_eq!(
            client.dmabuf_version(),
            Some(3),
            "with no device to name, feedback (version 4) cannot be served"
        );
    });
}

#[test]
fn a_dmabuf_window_maps_releases_and_unmaps() {
    let session = Session::with_importer(Some(DEVICE));
    session.run(|handle| {
        let mut client = Client::connect(&handle.socket);
        let global = client.dmabuf(4);
        let (surface, _toplevel) = client.mapped_dmabuf_toplevel(&global, 64, 48);
        let observed = handle.wait_for(|snapshot| snapshot.windows().any(|window| window.mapped));
        let window = observed
            .windows()
            .find(|window| window.mapped)
            .expect("a mapped window");
        assert_eq!(
            (window.geometry.width, window.geometry.height),
            (64, 48),
            "the window is as large as its dmabuf, which Smithay sizes from the buffer alone"
        );
        assert_eq!(
            handle.seen(),
            vec![(64, 48, Fourcc::Argb8888)],
            "the importer saw the buffer once, before the client used it"
        );

        // A frame callback requested with a commit is answered, whatever the buffer's kind.
        let _callback = surface.frame(&client.qh, ());
        surface.damage(0, 0, 64, 48);
        surface.commit();
        client.wait_until(|app| app.callbacks >= 1);

        // A second buffer replaces the first, and the first is released.
        let first = client.immed_buffer(&global, 64, 48);
        let second = client.immed_buffer(&global, 64, 48);
        surface.attach(Some(&first), 0, 0);
        surface.commit();
        surface.attach(Some(&second), 0, 0);
        surface.damage(0, 0, 64, 48);
        surface.commit();
        let first_id = first.id().protocol_id();
        client.wait_until(|app| app.released.contains(&first_id));

        // A commit with no buffer unmaps the window.
        surface.attach(None, 0, 0);
        surface.commit();
        client.roundtrip();
        handle.wait_for(|snapshot| snapshot.windows().all(|window| !window.mapped));
    });
}

/// A commit that attaches a dmabuf waits for the buffer's fences, so the previous frame stays up until the
/// new one is complete.
///
/// The stand-in fence is a pipe's read end: it polls as not ready until something is written, which is how a
/// dma-buf with an unsignalled fence behaves. Smithay cannot measure a pipe, so it skips the size check, and
/// the compositor sees a buffer that is not ready.
#[test]
fn a_commit_waits_for_the_buffers_fences() {
    use std::io::Write as _;
    let session = Session::with_importer(Some(DEVICE));
    session.run(|handle| {
        let mut client = Client::connect(&handle.socket);
        let global = client.dmabuf(4);
        let (surface, _toplevel) = client.mapped_dmabuf_toplevel(&global, 64, 48);
        let shown = client.immed_buffer(&global, 64, 48);
        surface.attach(Some(&shown), 0, 0);
        surface.damage(0, 0, 64, 48);
        surface.commit();
        client.roundtrip();
        let shown_id = shown.id().protocol_id();

        let (reader, mut writer) = std::io::pipe().expect("a pipe");
        let fence = OwnedFd::from(reader);
        let params = global.create_params(&client.qh, ());
        params.add(fence.as_fd(), 0, 0, 64 * 4, 0, 0);
        let pending = params.create_immed(64, 48, Fourcc::Argb8888 as u32, params::Flags::empty(), &client.qh, ());
        surface.attach(Some(&pending), 0, 0);
        surface.damage(0, 0, 64, 48);
        surface.commit();

        // While the fence is unsignalled the commit is held, so the buffer on show is not replaced.
        let held_until = Instant::now() + Duration::from_millis(300);
        while Instant::now() < held_until {
            client.roundtrip();
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !client.state.released.contains(&shown_id),
            "the commit was applied before the buffer's fence signalled"
        );

        // The fence signals; the commit is applied and the buffer on show is released.
        writer.write_all(&[1]).expect("the fence is signalled");
        client.wait_until(|app| app.released.contains(&shown_id));
    });
}

#[test]
fn a_buffer_the_importer_refuses_fails_the_request() {
    let session = Session::with_importer(Some(DEVICE));
    session.run(|handle| {
        let mut client = Client::connect(&handle.socket);
        let global = client.dmabuf(4);
        handle.refuse_imports(true);

        // `create` is allowed to fail: the client hears `failed` and nothing is created.
        let params = client.params_with_plane(&global, 32, 32, Modifier::Linear);
        params.create(32, 32, Fourcc::Argb8888 as u32, params::Flags::empty());
        client.wait_until(|app| app.failed == 1);
        assert!(client.state.created.is_none(), "a refused buffer is never created");
        assert_eq!(handle.seen().len(), 1, "the importer was asked once");

        // `create_immed` cannot fail: the client is told by a protocol error, `invalid_wl_buffer`.
        let params = client.params_with_plane(&global, 32, 32, Modifier::Linear);
        let _buffer = params.create_immed(32, 32, Fourcc::Argb8888 as u32, params::Flags::empty(), &client.qh, ());
        let (interface, code) = client.protocol_error();
        assert_eq!(interface, "zwp_linux_buffer_params_v1");
        assert_eq!(code, params::Error::InvalidWlBuffer as u32);
    });
}

#[test]
fn an_accepted_buffer_is_created() {
    let session = Session::with_importer(Some(DEVICE));
    session.run(|handle| {
        let mut client = Client::connect(&handle.socket);
        let global = client.dmabuf(4);
        let params = client.params_with_plane(&global, 32, 16, Modifier::Linear);
        params.create(32, 16, Fourcc::Xrgb8888 as u32, params::Flags::empty());
        client.wait_until(|app| app.created.is_some());
        assert_eq!(client.state.failed, 0);
        assert_eq!(handle.seen(), vec![(32, 16, Fourcc::Xrgb8888)]);
    });
}

#[test]
fn a_modifier_that_was_not_offered_is_refused() {
    let session = Session::with_importer(Some(DEVICE));
    session.run(|handle| {
        let mut client = Client::connect(&handle.socket);
        let global = client.dmabuf(4);
        // Argb8888 is offered as linear only; the implicit modifier is a different promise, which
        // Smithay does not check (it knows the code, not the pair).
        let params = client.params_with_plane(&global, 32, 32, Modifier::Invalid);
        params.create(32, 32, Fourcc::Argb8888 as u32, params::Flags::empty());
        client.wait_until(|app| app.failed == 1);
        assert!(
            handle.seen().is_empty(),
            "an unoffered pair is refused before the importer is asked"
        );
    });
}

#[test]
fn a_wrong_plane_count_is_refused() {
    let session = Session::with_importer(Some(DEVICE));
    session.run(|handle| {
        let mut client = Client::connect(&handle.socket);
        let global = client.dmabuf(4);
        let params = global.create_params(&client.qh, ());
        for plane in 0..2 {
            let fd = Client::plane(32 * 4 * 32);
            params.add(fd.as_fd(), plane, 0, 32 * 4, 0, 0);
        }
        params.create(32, 32, Fourcc::Argb8888 as u32, params::Flags::empty());
        client.wait_until(|app| app.failed == 1);
        assert!(
            handle.seen().is_empty(),
            "a linear buffer of two planes is refused before the importer is asked"
        );
    });
}

#[test]
fn a_zero_sized_buffer_is_a_protocol_error() {
    let session = Session::with_importer(Some(DEVICE));
    session.run(|handle| {
        let mut client = Client::connect(&handle.socket);
        let global = client.dmabuf(4);
        let params = client.params_with_plane(&global, 32, 32, Modifier::Linear);
        params.create(0, 32, Fourcc::Argb8888 as u32, params::Flags::empty());
        let (interface, code) = client.protocol_error();
        assert_eq!(interface, "zwp_linux_buffer_params_v1");
        assert_eq!(code, params::Error::InvalidDimensions as u32);
    });
}

#[test]
fn an_unoffered_format_is_a_protocol_error() {
    let session = Session::with_importer(Some(DEVICE));
    session.run(|handle| {
        let mut client = Client::connect(&handle.socket);
        let global = client.dmabuf(4);
        let params = client.params_with_plane(&global, 32, 32, Modifier::Linear);
        // Abgr8888 is a real format the stand-in does not offer.
        params.create(32, 32, Fourcc::Abgr8888 as u32, params::Flags::empty());
        let (interface, code) = client.protocol_error();
        assert_eq!(interface, "zwp_linux_buffer_params_v1");
        assert_eq!(code, params::Error::InvalidFormat as u32);
    });
}

#[test]
fn a_second_use_of_the_params_is_a_protocol_error() {
    let session = Session::with_importer(Some(DEVICE));
    session.run(|handle| {
        let mut client = Client::connect(&handle.socket);
        let global = client.dmabuf(4);
        let params = client.params_with_plane(&global, 32, 32, Modifier::Linear);
        params.create(32, 32, Fourcc::Argb8888 as u32, params::Flags::empty());
        params.create(32, 32, Fourcc::Argb8888 as u32, params::Flags::empty());
        let (_, code) = client.protocol_error();
        assert_eq!(code, params::Error::AlreadyUsed as u32);
    });
}

#[test]
fn an_incomplete_params_is_a_protocol_error() {
    let session = Session::with_importer(Some(DEVICE));
    session.run(|handle| {
        let mut client = Client::connect(&handle.socket);
        let global = client.dmabuf(4);
        let params = global.create_params(&client.qh, ());
        params.create(32, 32, Fourcc::Argb8888 as u32, params::Flags::empty());
        let (_, code) = client.protocol_error();
        assert_eq!(code, params::Error::Incomplete as u32);
    });
}

#[test]
fn a_plane_that_does_not_fit_its_file_is_a_protocol_error() {
    let session = Session::with_importer(Some(DEVICE));
    session.run(|handle| {
        let mut client = Client::connect(&handle.socket);
        let global = client.dmabuf(4);
        // The file holds 32 rows; the buffer claims 64.
        let params = global.create_params(&client.qh, ());
        let fd = Client::plane(32 * 4 * 32);
        params.add(fd.as_fd(), 0, 0, 32 * 4, 0, 0);
        params.create(32, 64, Fourcc::Argb8888 as u32, params::Flags::empty());
        let (_, code) = client.protocol_error();
        assert_eq!(code, params::Error::OutOfBounds as u32);
    });
}
