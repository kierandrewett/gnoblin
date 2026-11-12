//! A real GPU buffer, shared by a real client, drawn by the real compositor: the end-to-end proof
//! `tests/dmabuf.rs` cannot give.
//!
//! A client allocates a GBM buffer object on the render node the compositor names as its main device,
//! fills it with one colour, exports it as a dmabuf and shares it through `zwp_linux_buffer_params_v1`.
//! The compositor imports it into its GLES renderer, draws the window from the imported texture, and
//! the test reads the output back through the control handle and probes the window's pixels. Every
//! step is the production path; nothing here is a stand-in.
//!
//! # Where it runs, and where it cannot
//!
//! It needs a DRM render node and an EGL display that imports dmabufs. The Agent37 build instances
//! have neither (`/dev/dri` does not exist in their gVisor sandbox, so GBM cannot allocate and no
//! driver can import), so there the test prints why and returns: it is compiled and linked on every
//! gate and is only *run* on a machine with a GPU or a virtual one (`vkms`, `virtio-gpu`). A skip is
//! deliberate and loud, never a silent pass: the message names what was missing.
//!
//! The behaviour that does not need a device (the protocol, the offer, every refusal, the surface
//! life of a dmabuf window) is in `tests/dmabuf.rs` and runs everywhere.

use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::{Headless, HeadlessConfig};
use gnoblin_wayland::{Config, ControlHandle, LoopError, Server, SessionSnapshot};
use smithay::backend::allocator::dmabuf::AsDmabuf;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Allocator, Buffer as _, Fourcc, Modifier};
use smithay::backend::drm::{DrmDeviceFd, DrmNode};
use smithay::utils::DeviceFd;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1 as params, zwp_linux_dmabuf_feedback_v1 as feedback,
    zwp_linux_dmabuf_v1 as dmabuf_global,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

const TIMEOUT: Duration = Duration::from_secs(10);

/// What the client records of the compositor's feedback.
#[derive(Default)]
struct App {
    main_device: Option<u64>,
    feedback_done: bool,
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

macro_rules! ignore_events {
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

ignore_events!(
    wl_compositor::WlCompositor,
    wl_surface::WlSurface,
    wl_buffer::WlBuffer,
    xdg_toplevel::XdgToplevel,
    dmabuf_global::ZwpLinuxDmabufV1,
    params::ZwpLinuxBufferParamsV1,
);

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
            feedback::Event::Done => state.feedback_done = true,
            _ => {}
        }
    }
}

struct StopOnDrop(ControlHandle);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.stop();
    }
}

fn roundtrip(queue: &mut EventQueue<App>, state: &mut App) {
    queue.roundtrip(state).expect("a round trip");
}

/// The GBM device on the render node numbered `device`, or why there is none.
fn gbm_on(device: u64) -> Result<GbmDevice<DrmDeviceFd>, String> {
    let node = DrmNode::from_dev_id(device).map_err(|error| format!("no DRM node numbered {device:#x}: {error}"))?;
    let path = node
        .dev_path()
        .ok_or_else(|| format!("DRM node {device:#x} has no device path"))?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|error| format!("{} cannot be opened: {error}", path.display()))?;
    GbmDevice::new(DrmDeviceFd::new(DeviceFd::from(OwnedFd::from(file))))
        .map_err(|error| format!("no GBM device on {}: {error}", path.display()))
}

#[test]
fn a_gpu_buffer_is_drawn_from_the_texture_the_compositor_imports() {
    let backend = match Headless::with_config(HeadlessConfig::gles()) {
        Ok(backend) => backend,
        Err(error) => {
            println!("skipping the GPU dmabuf test: no GLES renderer on this machine ({error})");
            return;
        }
    };
    let directory = tempfile::tempdir().expect("a temporary directory");
    let socket = directory.path().join("wayland-0.sock");
    let config = Config {
        socket_path: Some(socket.clone()),
        ..Config::default()
    };
    let server = Server::new(Box::new(backend), config).expect("the session starts");
    if !server.state().serves_dmabuf() {
        println!(
            "skipping the GPU dmabuf test: the compositor's EGL display imports no dmabuf format \
             (a software rasteriser, or a machine with no render node)"
        );
        return;
    }
    let control = server.control();
    let observation: Arc<Mutex<SessionSnapshot>> = server.observation();
    let skipped = Arc::new(Mutex::new(None::<String>));

    let worker = thread::spawn({
        let control = control.clone();
        let skipped = Arc::clone(&skipped);
        move || {
            let _stop = StopOnDrop(control.clone());
            let stream = UnixStream::connect(&socket).expect("the session is listening");
            let connection = Connection::from_socket(stream).expect("a client connection");
            let (globals, mut queue) = registry_queue_init::<App>(&connection).expect("the global list");
            let qh = queue.handle();
            let mut app = App::default();

            // Version 4 (default feedback) is served only with a DRM render node to name as the main device. A software
            // rasteriser that does import dmabufs (Mesa's llvmpipe on a machine with no `/dev/dri`) passes the gate above
            // and still serves version 3 only, which is a skip, not a failure.
            let global: dmabuf_global::ZwpLinuxDmabufV1 = match globals.bind(&qh, 4..=4, ()) {
                Ok(global) => global,
                Err(error) => {
                    *skipped.lock().expect("the skip note") = Some(format!(
                        "the compositor serves no zwp_linux_dmabuf_v1 version 4 ({error}): no render node"
                    ));
                    return;
                }
            };
            let _feedback = global.get_default_feedback(&qh, ());
            let deadline = Instant::now() + TIMEOUT;
            while !app.feedback_done {
                assert!(Instant::now() < deadline, "the compositor never sent its feedback");
                roundtrip(&mut queue, &mut app);
            }
            let Some(device) = app.main_device else {
                *skipped.lock().expect("the skip note") = Some("the compositor's feedback names no main device".into());
                return;
            };
            let gbm = match gbm_on(device) {
                Ok(gbm) => gbm,
                Err(reason) => {
                    *skipped.lock().expect("the skip note") = Some(reason);
                    return;
                }
            };

            // One opaque red pixel, repeated: ARGB8888 is B, G, R, A in memory.
            let (width, height) = (64u32, 48u32);
            let mut allocator = GbmAllocator::new(gbm, GbmBufferFlags::WRITE);
            let mut buffer = match allocator.create_buffer(width, height, Fourcc::Argb8888, &[Modifier::Linear]) {
                Ok(buffer) => buffer,
                Err(error) => {
                    *skipped.lock().expect("the skip note") =
                        Some(format!("the GPU allocates no linear ARGB8888 buffer: {error}"));
                    return;
                }
            };
            let pixels: Vec<u8> = [0u8, 0, 255, 255].repeat((width * height) as usize);
            buffer.write(&pixels).expect("a linear buffer takes a write");
            let dmabuf = buffer.export().expect("the buffer exports as a dmabuf");

            let params = global.create_params(&qh, ());
            let format = dmabuf.format();
            let modifier = u64::from(format.modifier);
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
                width as i32,
                height as i32,
                Fourcc::Argb8888 as u32,
                params::Flags::empty(),
                &qh,
                (),
            );

            let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=6, ()).expect("wl_compositor");
            let wm_base: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=6, ()).expect("xdg_wm_base");
            let surface = compositor.create_surface(&qh, ());
            let xdg = wm_base.get_xdg_surface(&surface, &qh, ());
            let toplevel = xdg.get_toplevel(&qh, ());
            toplevel.set_title("gpu window".to_owned());
            surface.commit();
            roundtrip(&mut queue, &mut app);
            surface.attach(Some(&wl), 0, 0);
            surface.damage(0, 0, width as i32, height as i32);
            surface.commit();
            roundtrip(&mut queue, &mut app);

            // The window maps, and the output shows the buffer's colour at its centre.
            let deadline = Instant::now() + TIMEOUT;
            let window = loop {
                let snapshot = observation.lock().expect("the observation lock").clone();
                if let Some(window) = snapshot.windows().find(|window| window.mapped) {
                    break window.geometry;
                }
                assert!(Instant::now() < deadline, "the window never mapped");
                thread::sleep(Duration::from_millis(5));
            };
            let output = *observation
                .lock()
                .expect("the observation lock")
                .outputs()
                .first()
                .expect("an output");
            let centre = (
                window.x + i32::try_from(window.width / 2).expect("a small width"),
                window.y + i32::try_from(window.height / 2).expect("a small height"),
            );
            let deadline = Instant::now() + TIMEOUT;
            loop {
                let image = control
                    .screenshot(output)
                    .expect("the loop answers a screenshot request");
                let pixel = image.colour_at(centre.0, centre.1).expect("a pixel inside the output");
                if pixel == [255, 0, 0, 255] {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "the dmabuf window never showed the buffer's red: {pixel:?} at {centre:?}"
                );
                roundtrip(&mut queue, &mut app);
                thread::sleep(Duration::from_millis(10));
            }
            // The client's buffer and connection live until the probe has seen the colour.
            drop((dmabuf, buffer, wl, connection));
        }
    });
    let outcome: Result<(), LoopError> = server.run();
    if let Err(payload) = worker.join() {
        std::panic::resume_unwind(payload);
    }
    outcome.expect("the loop stops cleanly");
    if let Some(reason) = skipped.lock().expect("the skip note").take() {
        println!("skipping the GPU dmabuf test: {reason}");
    }
}
