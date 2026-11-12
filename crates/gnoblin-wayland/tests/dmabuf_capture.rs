//! Captures into a GPU buffer, against the real compositor, with a stand-in GPU (`tests/mock_gpu`).
//!
//! The protocol layer under test is the real one: the sessions and their constraints, the check that a client's
//! buffer meets them, the queue the loop answers GPU captures from, the lock, the damage report. What is a stand-in is
//! the GPU (see `mock_gpu`): the Agent37 instances have no DRM device, so the stand-in names a render node, accepts the
//! client's buffers and writes the frame into a buffer's plane, which is an ordinary file the test reads back. That a
//! real GL blit lands the same pixels the same way up is `gnoblin-render-gles`'s tests and `tests/dmabuf_capture_gpu.rs`.
//!
//! - `a_session_offers_dma_buf_only_when_the_backend_can_render_into_a_client_buffer`
//! - `a_gpu_buffer_in_an_offered_format_is_copied_and_the_frame_is_ready`
//! - `a_gpu_buffer_in_an_unoffered_format_is_refused`, `a_gpu_buffer_that_is_too_small_is_refused`,
//!   `a_gpu_buffer_is_refused_when_none_was_offered`
//! - `a_screencopy_offers_linux_dmabuf_only_when_capable_and_copies_into_one`
//! - `a_gpu_capture_refused_by_the_driver_fails_the_frame_and_the_next_one_works`

mod mock_gpu;

use std::fs::File;
use std::io::Write;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::FileExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::Headless;
use gnoblin_types::Rect;
use gnoblin_wayland::{Config, ControlHandle, LoopError, Server, SessionSnapshot};
use mock_gpu::{Controls, DEVICE, Mock};
use smithay::backend::allocator::{Fourcc, Modifier};
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_image_capture_source_v1, ext_output_image_capture_source_manager_v1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1, ext_image_copy_capture_manager_v1, ext_image_copy_capture_session_v1,
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1 as params, zwp_linux_dmabuf_v1 as dmabuf_global,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::screencopy::v1::client::{zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1};

const TIMEOUT: Duration = Duration::from_secs(10);

/// The window's pixels: opaque red, as the bytes `argb8888` stores: B, G, R, A.
const RED_BYTES: [u8; 4] = [0, 0, 255, 255];

/// The compositor's background: opaque black.
const BLACK_BYTES: [u8; 4] = [0, 0, 0, 255];

#[derive(Clone)]
struct Handle {
    socket: PathBuf,
    privileged: PathBuf,
    control: ControlHandle,
    observation: Arc<Mutex<SessionSnapshot>>,
    gpu: Controls,
}

impl Handle {
    /// The mapped window's rectangle, once it is.
    fn window(&self) -> Rect {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let snapshot = self.observation.lock().expect("the observation lock").clone();
            if let Some(window) = snapshot.windows().find(|window| window.mapped) {
                return window.geometry;
            }
            assert!(Instant::now() < deadline, "the window never mapped");
            thread::sleep(Duration::from_millis(5));
        }
    }
}

struct StopOnDrop(ControlHandle);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// Runs a session on `backend` and `body` on a worker.
fn run(backend: Headless, gpu: Controls, body: impl FnOnce(&Handle) + Send + 'static) {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let socket = directory.path().join("wayland-0.sock");
    let privileged = directory.path().join("wayland-0-privileged.sock");
    let config = Config {
        socket_path: Some(socket.clone()),
        privileged_socket_path: Some(privileged.clone()),
        ..Config::default()
    };
    let server = Server::new(Box::new(backend), config).expect("the session starts");
    server.control().set_reduced_motion(true);
    let handle = Handle {
        socket,
        privileged,
        control: server.control(),
        observation: server.observation(),
        gpu,
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
    drop(directory);
}

/// A session whose GPU imports client buffers and renders captures into the pairs `capture` names, or into none.
fn with_gpu(capture: Option<Vec<smithay::backend::allocator::Format>>, body: impl FnOnce(&Handle) + Send + 'static) {
    let (mock, controls) = Mock::new(capture);
    run(
        Headless::new().with_dmabuf_provider(Box::new(mock)),
        controls.clone(),
        body,
    );
}

/// What a client recorded.
#[derive(Default)]
struct App {
    buffer_size: Option<(u32, u32)>,
    shm_formats: Vec<u32>,
    dmabuf_device: Option<u64>,
    dmabuf_formats: Vec<(u32, Vec<u64>)>,
    done: usize,
    stopped: usize,
    ready: usize,
    failed: Vec<u32>,
    damage: Vec<(i32, i32, i32, i32)>,
    sc_buffer: bool,
    sc_dmabuf: Vec<(u32, u32, u32)>,
    sc_done: bool,
    sc_ready: usize,
    sc_failed: usize,
    configures: usize,
}

struct Client {
    _connection: Connection,
    queue: EventQueue<App>,
    state: App,
    qh: QueueHandle<App>,
    globals: GlobalList,
}

impl Client {
    fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("the session is listening");
        let connection = Connection::from_socket(stream).expect("a client connection");
        let (globals, queue) = registry_queue_init::<App>(&connection).expect("the global list");
        let qh = queue.handle();
        Self {
            _connection: connection,
            queue,
            state: App::default(),
            qh,
            globals,
        }
    }

    fn roundtrip(&mut self) {
        self.queue.roundtrip(&mut self.state).expect("a round trip");
    }

    fn wait_until(&mut self, what: &str, done: impl Fn(&App) -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        while !done(&self.state) {
            assert!(Instant::now() < deadline, "never saw {what}");
            self.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn bind<I: Proxy + 'static>(&self, version: u32) -> I
    where
        App: Dispatch<I, ()>,
    {
        self.globals
            .bind(&self.qh, version..=version, ())
            .unwrap_or_else(|error| panic!("{} is not advertised: {error}", I::interface().name))
    }

    /// Maps a red `width` x `height` shared-memory window and returns what keeps it alive.
    fn map_window(&mut self, width: i32, height: i32) -> (File, wl_buffer::WlBuffer, wl_surface::WlSurface) {
        let compositor: wl_compositor::WlCompositor = self.globals.bind(&self.qh, 1..=6, ()).expect("wl_compositor");
        let wm_base: xdg_wm_base::XdgWmBase = self.globals.bind(&self.qh, 1..=6, ()).expect("xdg_wm_base");
        let shm: wl_shm::WlShm = self.globals.bind(&self.qh, 1..=1, ()).expect("wl_shm");
        let surface = compositor.create_surface(&self.qh, ());
        let xdg = wm_base.get_xdg_surface(&surface, &self.qh, ());
        let toplevel = xdg.get_toplevel(&self.qh, ());
        toplevel.set_title("window".to_owned());
        surface.commit();
        self.roundtrip();
        let length = (width * 4 * height) as usize;
        let mut file = tempfile::tempfile().expect("a shared memory file");
        file.set_len(length as u64).expect("a file that big");
        file.write_all(&RED_BYTES.repeat((width * height) as usize))
            .expect("pixels");
        let pool = shm.create_pool(file.as_fd(), length as i32, &self.qh, ());
        let buffer = pool.create_buffer(0, width, height, width * 4, wl_shm::Format::Argb8888, &self.qh, ());
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, width, height);
        surface.commit();
        self.roundtrip();
        (file, buffer, surface)
    }

    /// A `width` x `height` GPU buffer of `format` and `modifier`, with the plane file that backs it.
    fn gpu_buffer(&self, width: i32, height: i32, format: Fourcc, modifier: Modifier) -> (File, wl_buffer::WlBuffer) {
        let global: dmabuf_global::ZwpLinuxDmabufV1 = self.bind(3);
        let stride = width as u32 * 4;
        let file = tempfile::tempfile().expect("a plane file");
        file.set_len(u64::from(stride) * height as u64)
            .expect("a file that long");
        let fd = OwnedFd::from(file.try_clone().expect("a second handle"));
        let params = global.create_params(&self.qh, ());
        let modifier = u64::from(modifier);
        params.add(fd.as_fd(), 0, 0, stride, (modifier >> 32) as u32, modifier as u32);
        let buffer = params.create_immed(width, height, format as u32, params::Flags::empty(), &self.qh, ());
        params.destroy();
        (file, buffer)
    }
}

/// One pixel of a plane file as the bytes `argb8888` stores.
fn pixel(file: &File, width: i32, x: i32, y: i32) -> [u8; 4] {
    let mut bytes = [0u8; 4];
    file.read_exact_at(&mut bytes, (y as u64 * width as u64 + x as u64) * 4)
        .expect("a pixel");
    bytes
}

/// Opens an image-copy session on the first output and waits for its constraints.
fn open_session(
    capture: &mut Client,
) -> (
    ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1,
    ext_image_capture_source_v1::ExtImageCaptureSourceV1,
) {
    let output: wl_output::WlOutput = capture.globals.bind(&capture.qh, 1..=4, ()).expect("wl_output");
    let sources: ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1 = capture.bind(1);
    let copies: ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1 = capture.bind(1);
    let source = sources.create_source(&output, &capture.qh, ());
    let done = capture.state.done;
    let session = copies.create_session(
        &source,
        ext_image_copy_capture_manager_v1::Options::empty(),
        &capture.qh,
        (),
    );
    capture.wait_until("the session's constraints", |state| state.done > done);
    (session, source)
}

/// Asks for one frame into `buffer` and waits for the answer; `true` when it was delivered.
fn capture_into(
    capture: &mut Client,
    session: &ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1,
    buffer: &wl_buffer::WlBuffer,
) -> bool {
    let (ready, failed) = (capture.state.ready, capture.state.failed.len());
    let frame = session.create_frame(&capture.qh, ());
    frame.attach_buffer(buffer);
    frame.capture();
    capture.wait_until("an answer to the frame", |state| {
        state.ready > ready || state.failed.len() > failed
    });
    let delivered = capture.state.ready > ready;
    frame.destroy();
    delivered
}

#[test]
fn a_session_offers_dma_buf_only_when_the_backend_can_render_into_a_client_buffer() {
    // A software session: no GPU, shared memory only.
    let (software, no_gpu) = Mock::import_only();
    drop(software);
    run(Headless::new(), no_gpu, |handle| {
        let mut capture = Client::connect(&handle.privileged);
        let _ = open_session(&mut capture);
        assert!(!capture.state.shm_formats.is_empty(), "shared memory is offered");
        assert!(capture.state.dmabuf_device.is_none(), "no device without a GPU");
        assert!(capture.state.dmabuf_formats.is_empty(), "and no formats");
    });
    // A GPU that imports client buffers and cannot render into one.
    let (mock, controls) = Mock::import_only();
    run(
        Headless::new().with_dmabuf_provider(Box::new(mock)),
        controls,
        |handle| {
            let mut capture = Client::connect(&handle.privileged);
            let _ = open_session(&mut capture);
            assert!(
                capture.state.dmabuf_device.is_none(),
                "an import-only GPU offers no capture buffer"
            );
            assert!(capture.state.dmabuf_formats.is_empty());
        },
    );
    // A GPU that renders into a client's buffer: the device and the formats it can render into.
    with_gpu(Some(mock_gpu::renderable()), |handle| {
        let mut capture = Client::connect(&handle.privileged);
        let _ = open_session(&mut capture);
        assert_eq!(capture.state.dmabuf_device, Some(DEVICE), "the render node");
        let offered: Vec<u32> = capture.state.dmabuf_formats.iter().map(|(code, _)| *code).collect();
        assert!(offered.contains(&(Fourcc::Argb8888 as u32)) && offered.contains(&(Fourcc::Xrgb8888 as u32)));
        let linear = u64::from(Modifier::Linear);
        assert!(
            capture
                .state
                .dmabuf_formats
                .iter()
                .all(|(_, modifiers)| modifiers == &[linear]),
            "each format carries the modifiers the renderer can render into: {:?}",
            capture.state.dmabuf_formats
        );
        assert!(!capture.state.shm_formats.is_empty(), "shared memory is still offered");
    });
}

#[test]
fn a_gpu_buffer_in_an_offered_format_is_copied_and_the_frame_is_ready() {
    with_gpu(Some(mock_gpu::renderable()), |handle| {
        let mut app = Client::connect(&handle.socket);
        let (_pixels, _buffer, _surface) = app.map_window(100, 80);
        let window = handle.window();
        let mut capture = Client::connect(&handle.privileged);
        let (session, _source) = open_session(&mut capture);
        let (width, height) = capture.state.buffer_size.expect("the session's size");
        let (plane, buffer) = capture.gpu_buffer(width as i32, height as i32, Fourcc::Argb8888, Modifier::Linear);
        // The window may not have been drawn yet: ask until the output shows it.
        let deadline = Instant::now() + TIMEOUT;
        loop {
            assert!(
                capture_into(&mut capture, &session, &buffer),
                "a frame into a GPU buffer is delivered"
            );
            let centre = pixel(&plane, width as i32, window.x + 50, window.y + 40);
            if centre == RED_BYTES {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the capture never showed the window: {centre:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            pixel(&plane, width as i32, 1, 1),
            BLACK_BYTES,
            "the background is where the window is not"
        );
        assert!(!capture.state.damage.is_empty(), "the frame reports what it wrote");
        let written = handle.gpu.written();
        assert!(
            !written.is_empty(),
            "the copy went through the GPU stand-in, not shared memory"
        );
        assert_eq!(written[0].format, Fourcc::Argb8888);
        assert_eq!(written[0].buffer.width, width);
    });
}

#[test]
fn a_gpu_buffer_in_an_unoffered_format_is_refused() {
    // Only argb8888 / linear is offered for capture.
    let only_argb = vec![smithay::backend::allocator::Format {
        code: Fourcc::Argb8888,
        modifier: Modifier::Linear,
    }];
    with_gpu(Some(only_argb), |handle| {
        let mut capture = Client::connect(&handle.privileged);
        let (session, _source) = open_session(&mut capture);
        let (width, height) = capture.state.buffer_size.expect("the session's size");
        let (plane, xrgb) = capture.gpu_buffer(width as i32, height as i32, Fourcc::Xrgb8888, Modifier::Linear);
        assert!(
            !capture_into(&mut capture, &session, &xrgb),
            "a format that was not offered is refused"
        );
        let (_plane2, modifier) = capture.gpu_buffer(width as i32, height as i32, Fourcc::Argb8888, Modifier::Invalid);
        assert!(
            !capture_into(&mut capture, &session, &modifier),
            "and so is an unoffered modifier"
        );
        assert!(handle.gpu.written().is_empty(), "nothing was written");
        assert_eq!(
            pixel(&plane, width as i32, 3, 3),
            [0; 4],
            "the refused buffer is untouched"
        );
        // The session survives: an offered buffer is delivered.
        let (_plane3, good) = capture.gpu_buffer(width as i32, height as i32, Fourcc::Argb8888, Modifier::Linear);
        assert!(capture_into(&mut capture, &session, &good), "the next frame works");
    });
}

#[test]
fn a_gpu_buffer_that_is_too_small_is_refused() {
    with_gpu(Some(mock_gpu::renderable()), |handle| {
        let mut capture = Client::connect(&handle.privileged);
        let (session, _source) = open_session(&mut capture);
        let (_plane, small) = capture.gpu_buffer(16, 16, Fourcc::Argb8888, Modifier::Linear);
        assert!(
            !capture_into(&mut capture, &session, &small),
            "a buffer smaller than the session is refused"
        );
        assert!(handle.gpu.written().is_empty());
        assert_eq!(capture.state.failed.len(), 1, "with one `failed`");
    });
}

#[test]
fn a_gpu_buffer_is_refused_when_none_was_offered() {
    // The GPU imports the client's buffer, but cannot render into it, so the session offered shared memory only.
    let (mock, controls) = Mock::import_only();
    run(
        Headless::new().with_dmabuf_provider(Box::new(mock)),
        controls,
        |handle| {
            let mut capture = Client::connect(&handle.privileged);
            let (session, _source) = open_session(&mut capture);
            let (width, height) = capture.state.buffer_size.expect("the session's size");
            let (_plane, buffer) = capture.gpu_buffer(width as i32, height as i32, Fourcc::Argb8888, Modifier::Linear);
            assert!(
                !capture_into(&mut capture, &session, &buffer),
                "a GPU buffer nobody offered is refused"
            );
            assert!(handle.gpu.written().is_empty());
        },
    );
}

#[test]
fn a_gpu_capture_refused_by_the_driver_fails_the_frame_and_the_next_one_works() {
    with_gpu(Some(mock_gpu::renderable()), |handle| {
        let mut app = Client::connect(&handle.socket);
        let (_pixels, _buffer, _surface) = app.map_window(100, 80);
        let _ = handle.window();
        let mut capture = Client::connect(&handle.privileged);
        let (session, _source) = open_session(&mut capture);
        let (width, height) = capture.state.buffer_size.expect("the session's size");
        let (_plane, buffer) = capture.gpu_buffer(width as i32, height as i32, Fourcc::Argb8888, Modifier::Linear);
        handle.gpu.refuse_writes(true);
        assert!(
            !capture_into(&mut capture, &session, &buffer),
            "a copy the driver refuses fails the frame"
        );
        handle.gpu.refuse_writes(false);
        let deadline = Instant::now() + TIMEOUT;
        while !capture_into(&mut capture, &session, &buffer) {
            assert!(Instant::now() < deadline, "the session never recovered");
            thread::sleep(Duration::from_millis(10));
        }
    });
}

#[test]
fn a_screencopy_offers_linux_dmabuf_only_when_capable_and_copies_into_one() {
    // No capture capability: `buffer` and `buffer_done`, no `linux_dmabuf`.
    let (mock, controls) = Mock::import_only();
    run(
        Headless::new().with_dmabuf_provider(Box::new(mock)),
        controls,
        |handle| {
            let mut capture = Client::connect(&handle.privileged);
            let output: wl_output::WlOutput = capture.globals.bind(&capture.qh, 1..=4, ()).expect("wl_output");
            let manager: zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1 = capture.bind(3);
            let _frame = manager.capture_output(0, &output, &capture.qh, ());
            capture.wait_until("buffer_done", |state| state.sc_done);
            assert!(capture.state.sc_buffer, "shared memory is announced");
            assert!(capture.state.sc_dmabuf.is_empty(), "no GPU buffer is");
        },
    );
    with_gpu(Some(mock_gpu::renderable()), |handle| {
        let mut app = Client::connect(&handle.socket);
        let (_pixels, _buffer, _surface) = app.map_window(100, 80);
        let window = handle.window();
        let mut capture = Client::connect(&handle.privileged);
        let output: wl_output::WlOutput = capture.globals.bind(&capture.qh, 1..=4, ()).expect("wl_output");
        let manager: zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1 = capture.bind(3);
        let deadline = Instant::now() + TIMEOUT;
        loop {
            capture.state.sc_done = false;
            capture.state.sc_dmabuf.clear();
            let frame = manager.capture_output(0, &output, &capture.qh, ());
            capture.wait_until("buffer_done", |state| state.sc_done);
            let (code, width, height) = *capture
                .state
                .sc_dmabuf
                .iter()
                .find(|(code, _, _)| *code == Fourcc::Argb8888 as u32)
                .expect("argb8888 is offered as linux_dmabuf");
            let (plane, buffer) = capture.gpu_buffer(width as i32, height as i32, Fourcc::Argb8888, Modifier::Linear);
            assert_eq!(code, Fourcc::Argb8888 as u32);
            let ready = capture.state.sc_ready;
            frame.copy(&buffer);
            capture.wait_until("ready or failed", |state| state.sc_ready > ready || state.sc_failed > 0);
            assert_eq!(capture.state.sc_failed, 0, "the copy into the GPU buffer is delivered");
            if pixel(&plane, width as i32, window.x + 50, window.y + 40) == RED_BYTES {
                break;
            }
            assert!(Instant::now() < deadline, "the screencopy never showed the window");
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !handle.gpu.written().is_empty(),
            "the copy went through the GPU stand-in"
        );
    });
}

macro_rules! ignore_events {
    ($($interface:ty),* $(,)?) => {$(
        impl Dispatch<$interface, ()> for App {
            fn event(
                _: &mut Self,
                _: &$interface,
                _: <$interface as Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}

ignore_events!(
    wl_compositor::WlCompositor,
    wl_surface::WlSurface,
    wl_buffer::WlBuffer,
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_output::WlOutput,
    xdg_toplevel::XdgToplevel,
    dmabuf_global::ZwpLinuxDmabufV1,
    params::ZwpLinuxBufferParamsV1,
    ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
    ext_image_capture_source_v1::ExtImageCaptureSourceV1,
    ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1,
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
);

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
        state: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
            state.configures += 1;
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
                state.buffer_size = Some((width, height));
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat {
                format: WEnum::Value(format),
            } => state.shm_formats.push(format as u32),
            ext_image_copy_capture_session_v1::Event::DmabufDevice { device } => {
                state.dmabuf_device = Some(u64::from_ne_bytes(device.try_into().expect("a dev_t")));
            }
            ext_image_copy_capture_session_v1::Event::DmabufFormat { format, modifiers } => {
                let modifiers = modifiers
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|bytes| u64::from_ne_bytes(*bytes))
                    .collect();
                state.dmabuf_formats.push((format, modifiers));
            }
            ext_image_copy_capture_session_v1::Event::Done => state.done += 1,
            ext_image_copy_capture_session_v1::Event::Stopped => state.stopped += 1,
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
            ext_image_copy_capture_frame_v1::Event::Damage { x, y, width, height } => {
                state.damage.push((x, y, width, height));
            }
            ext_image_copy_capture_frame_v1::Event::Ready => state.ready += 1,
            ext_image_copy_capture_frame_v1::Event::Failed { reason } => {
                state.failed.push(match reason {
                    WEnum::Value(reason) => reason as u32,
                    WEnum::Unknown(raw) => raw,
                });
            }
            _ => {}
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
            zwlr_screencopy_frame_v1::Event::Buffer { .. } => state.sc_buffer = true,
            zwlr_screencopy_frame_v1::Event::LinuxDmabuf { format, width, height } => {
                state.sc_dmabuf.push((format, width, height));
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => state.sc_done = true,
            zwlr_screencopy_frame_v1::Event::Ready { .. } => state.sc_ready += 1,
            zwlr_screencopy_frame_v1::Event::Failed => state.sc_failed += 1,
            _ => {}
        }
    }
}
