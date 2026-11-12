//! A capture into a real GPU buffer, by the real compositor on a real GLES context: the hardware proof
//! `tests/dmabuf_capture.rs` cannot give (beads gnoblin-lubw).
//!
//! A client maps a window with a top half of one colour and a bottom half of another, opens an `ext-image-copy-capture`
//! session on the output, allocates a GBM buffer on the render node the session names, shares it, and captures into it.
//! The GL blit the compositor makes (`Gles::copy_frame_into_dmabuf`) is then read back through GBM and the window's
//! two halves are probed: that proves the frame lands the right way up in a *dmabuf* texture, which the renderer's own
//! tests can only state on a texture of theirs.
//!
//! It needs a DRM render node, a GBM device and an EGL display that imports dmabufs. The Agent37 instances have none
//! (gVisor, no `/dev/dri`), so there it prints what was missing and returns; it is compiled and linked on every gate and
//! only *run* on a machine with a GPU or a virtual one (`vkms`, `virtio-gpu`). A skip is deliberate and loud.

use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::{Headless, HeadlessConfig};
use gnoblin_wayland::{Config, ControlHandle, LoopError, Server};
use smithay::backend::allocator::dmabuf::AsDmabuf;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Allocator, Buffer as _, Fourcc, Modifier};
use smithay::backend::drm::{DrmDeviceFd, DrmNode};
use smithay::utils::DeviceFd;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
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

const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Default)]
struct App {
    size: Option<(u32, u32)>,
    device: Option<u64>,
    done: usize,
    ready: usize,
    failed: usize,
}

struct StopOnDrop(ControlHandle);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.stop();
    }
}

fn gbm_on(device: u64) -> Result<GbmDevice<DrmDeviceFd>, String> {
    let node = DrmNode::from_dev_id(device).map_err(|error| format!("no DRM node numbered {device:#x}: {error}"))?;
    let path = node
        .dev_path()
        .ok_or_else(|| format!("DRM node {device:#x} has no path"))?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|error| format!("{} cannot be opened: {error}", path.display()))?;
    GbmDevice::new(DrmDeviceFd::new(DeviceFd::from(OwnedFd::from(file))))
        .map_err(|error| format!("no GBM device on {}: {error}", path.display()))
}

#[test]
fn a_frame_is_blitted_into_a_real_gpu_buffer_the_right_way_up() {
    let backend = match Headless::with_config(HeadlessConfig::gles()) {
        Ok(backend) => backend,
        Err(error) => {
            println!("skipping the GPU capture test: no GLES renderer on this machine ({error})");
            return;
        }
    };
    let directory = tempfile::tempdir().expect("a temporary directory");
    let socket = directory.path().join("wayland-0.sock");
    let privileged = directory.path().join("wayland-0-privileged.sock");
    let config = Config {
        socket_path: Some(socket.clone()),
        privileged_socket_path: Some(privileged.clone()),
        ..Config::default()
    };
    let server = Server::new(Box::new(backend), config).expect("the session starts");
    if !server.state().serves_dmabuf() {
        println!("skipping the GPU capture test: the EGL display imports no dmabuf format (no render node)");
        return;
    }
    server.control().set_reduced_motion(true);
    let control = server.control();
    let observation = server.observation();
    let skipped = Arc::new(Mutex::new(None::<String>));
    let worker = thread::spawn({
        let control = control.clone();
        let skipped = Arc::clone(&skipped);
        move || {
            let _stop = StopOnDrop(control);
            let connect = |path: &std::path::Path| {
                let stream = UnixStream::connect(path).expect("the session is listening");
                let connection = Connection::from_socket(stream).expect("a client connection");
                let (globals, queue) = registry_queue_init::<App>(&connection).expect("the global list");
                (connection, globals, queue)
            };
            // The window: red on top, blue below.
            let (_c1, globals, mut queue) = connect(&socket);
            let qh = queue.handle();
            let mut app = App::default();
            let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=6, ()).expect("wl_compositor");
            let wm_base: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=6, ()).expect("xdg_wm_base");
            let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).expect("wl_shm");
            let surface = compositor.create_surface(&qh, ());
            let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
            let toplevel = xdg.get_toplevel(&qh, ());
            toplevel.set_title("halves".to_owned());
            surface.commit();
            queue.roundtrip(&mut app).expect("a round trip");
            let (width, height) = (100i32, 80i32);
            let mut file = tempfile::tempfile().expect("a shared memory file");
            let mut bytes = Vec::new();
            for row in 0..height {
                let colour: [u8; 4] = if row < height / 2 {
                    [0, 0, 255, 255]
                } else {
                    [255, 0, 0, 255]
                };
                bytes.extend(colour.repeat(width as usize));
            }
            std::io::Write::write_all(&mut file, &bytes).expect("pixels");
            let pool = shm.create_pool(std::os::fd::AsFd::as_fd(&file), bytes.len() as i32, &qh, ());
            let buffer = pool.create_buffer(0, width, height, width * 4, wl_shm::Format::Argb8888, &qh, ());
            surface.attach(Some(&buffer), 0, 0);
            surface.damage(0, 0, width, height);
            surface.commit();
            queue.roundtrip(&mut app).expect("a round trip");
            let window = {
                let deadline = Instant::now() + TIMEOUT;
                loop {
                    let snapshot = observation.lock().expect("the observation lock").clone();
                    if let Some(window) = snapshot.windows().find(|window| window.mapped) {
                        break window.geometry;
                    }
                    assert!(Instant::now() < deadline, "the window never mapped");
                    thread::sleep(Duration::from_millis(5));
                }
            };

            // The capture client.
            let (_c2, cglobals, mut cqueue) = connect(&privileged);
            let cqh = cqueue.handle();
            let mut capture = App::default();
            let output: wl_output::WlOutput = cglobals.bind(&cqh, 1..=4, ()).expect("wl_output");
            let sources: ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1 =
                cglobals.bind(&cqh, 1..=1, ()).expect("the output source manager");
            let copies: ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1 =
                cglobals.bind(&cqh, 1..=1, ()).expect("the copy manager");
            let source = sources.create_source(&output, &cqh, ());
            let session = copies.create_session(&source, ext_image_copy_capture_manager_v1::Options::empty(), &cqh, ());
            while capture.done == 0 {
                cqueue.roundtrip(&mut capture).expect("a round trip");
            }
            let Some(device) = capture.device else {
                *skipped.lock().expect("the skip note") = Some("the session offered no dma_buf device".into());
                return;
            };
            let gbm = match gbm_on(device) {
                Ok(gbm) => gbm,
                Err(reason) => {
                    *skipped.lock().expect("the skip note") = Some(reason);
                    return;
                }
            };
            let (out_width, out_height) = capture.size.expect("the session's size");
            let mut allocator = GbmAllocator::new(gbm, GbmBufferFlags::WRITE | GbmBufferFlags::RENDERING);
            let target = match allocator.create_buffer(out_width, out_height, Fourcc::Argb8888, &[Modifier::Linear]) {
                Ok(target) => target,
                Err(error) => {
                    *skipped.lock().expect("the skip note") = Some(format!("no linear ARGB8888 buffer: {error}"));
                    return;
                }
            };
            let dmabuf = target.export().expect("the buffer exports");
            let dma: dmabuf_global::ZwpLinuxDmabufV1 = cglobals.bind(&cqh, 3..=3, ()).expect("zwp_linux_dmabuf_v1");
            let params = dma.create_params(&cqh, ());
            let modifier = u64::from(dmabuf.format().modifier);
            for (index, ((fd, offset), stride)) in
                dmabuf.handles().zip(dmabuf.offsets()).zip(dmabuf.strides()).enumerate()
            {
                params.add(
                    fd,
                    index as u32,
                    offset,
                    stride,
                    (modifier >> 32) as u32,
                    modifier as u32,
                );
            }
            let wl = params.create_immed(
                out_width as i32,
                out_height as i32,
                Fourcc::Argb8888 as u32,
                params::Flags::empty(),
                &cqh,
                (),
            );
            let deadline = Instant::now() + TIMEOUT;
            loop {
                let (ready, failed) = (capture.ready, capture.failed);
                let frame = session.create_frame(&cqh, ());
                frame.attach_buffer(&wl);
                frame.capture();
                while capture.ready == ready && capture.failed == failed {
                    cqueue.roundtrip(&mut capture).expect("a round trip");
                }
                frame.destroy();
                if capture.ready > ready {
                    let (top, bottom) = target
                        .map(0, 0, out_width, out_height, |mapped| {
                            let stride = mapped.stride() as usize;
                            let at = |x: i32, y: i32| {
                                let start = y as usize * stride + x as usize * 4;
                                [
                                    mapped.buffer()[start],
                                    mapped.buffer()[start + 1],
                                    mapped.buffer()[start + 2],
                                    mapped.buffer()[start + 3],
                                ]
                            };
                            (
                                at(window.x + width / 2, window.y + height / 4),
                                at(window.x + width / 2, window.y + 3 * height / 4),
                            )
                        })
                        .expect("a mappable buffer");
                    if top == [0, 0, 255, 255] && bottom == [255, 0, 0, 255] {
                        break;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "the window's halves are not the right way up in the GPU buffer: top {top:?}, bottom {bottom:?}"
                    );
                } else {
                    assert!(Instant::now() < deadline, "the GPU capture never succeeded");
                }
                thread::sleep(Duration::from_millis(10));
            }
            drop((dmabuf, target, wl, buffer, file));
        }
    });
    let outcome: Result<(), LoopError> = server.run();
    if let Err(payload) = worker.join() {
        std::panic::resume_unwind(payload);
    }
    outcome.expect("the loop stops cleanly");
    if let Some(reason) = skipped.lock().expect("the skip note").take() {
        println!("skipping the GPU capture test: {reason}");
    }
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
                state.size = Some((width, height))
            }
            ext_image_copy_capture_session_v1::Event::DmabufDevice { device } => {
                state.device = Some(u64::from_ne_bytes(device.try_into().expect("a dev_t")));
            }
            ext_image_copy_capture_session_v1::Event::Done => state.done += 1,
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
            ext_image_copy_capture_frame_v1::Event::Ready => state.ready += 1,
            ext_image_copy_capture_frame_v1::Event::Failed {
                reason: WEnum::Value(_) | WEnum::Unknown(_),
            } => {
                state.failed += 1;
            }
            _ => {}
        }
    }
}
