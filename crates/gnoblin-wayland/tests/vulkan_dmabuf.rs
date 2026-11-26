//! What the real compositor on the real Vulkan renderer tells a real client about GPU buffers, and what it refuses.
//!
//! A headless session draws with the Vulkan renderer (lavapipe on a machine with no GPU). The renderer answers
//! `Backend::dmabuf` from the formats its device draws, so `zwp_linux_dmabuf_v1` is served: a software driver has no DRM
//! render node, so version 3, which names formats and modifiers and no device. A client then shares buffers through
//! `zwp_linux_buffer_params_v1`, and the compositor's answer is the renderer's: a single linear plane in an offered format
//! is created when it is a real dma-buf (`/dev/udmabuf`), a file that only has the shape of one is refused because it could be
//! resized under the renderer's copy, and a buffer with a tiled modifier is refused with `failed` before any pixel is touched.
//!
//! What this cannot prove on a machine with no `/dev/udmabuf` and no `/dev/dri` (the Agent37 boxes) is the pixels of a real
//! GPU buffer, and the created half of this test: both need a dma-buf source, and a temporary file is refused. The
//! pixel proof of the linear path is `gnoblin-render-vulkan`'s `tests::dmabuf`, which runs where `udmabuf` loads. The test
//! says which of the two it ran.

use gnoblin_wayland_testkit::stock;

use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::{Headless, HeadlessConfig};
use gnoblin_wayland::{Config, ControlHandle, LoopError, Server};
use smithay::backend::allocator::udmabuf::UdmabufAllocator;
use smithay::backend::allocator::{Allocator, Fourcc, Modifier};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1 as params, zwp_linux_dmabuf_v1 as dmabuf_global,
};

const TIMEOUT: Duration = Duration::from_secs(10);
/// `DRM_FORMAT_MOD_LINEAR`.
const LINEAR: u64 = 0;
/// `I915_FORMAT_MOD_X_TILED`: a modifier the renderer does not offer.
const X_TILED: u64 = 0x0100_0000_0000_0001;

#[derive(Default)]
struct App {
    /// The `(format, modifier)` pairs the compositor offered.
    offered: Vec<(u32, u64)>,
    created: usize,
    failed: usize,
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

impl Dispatch<dmabuf_global::ZwpLinuxDmabufV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &dmabuf_global::ZwpLinuxDmabufV1,
        event: dmabuf_global::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let dmabuf_global::Event::Modifier {
            format,
            modifier_hi,
            modifier_lo,
        } = event
        {
            state
                .offered
                .push((format, (u64::from(modifier_hi) << 32) | u64::from(modifier_lo)));
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
            params::Event::Created { .. } => state.created += 1,
            params::Event::Failed => state.failed += 1,
            _ => {}
        }
    }

    // `created` makes the `wl_buffer` a client would attach.
    wayland_client::event_created_child!(App, params::ZwpLinuxBufferParamsV1, [
        params::EVT_CREATED_OPCODE => (wayland_client::protocol::wl_buffer::WlBuffer, ()),
    ]);
}

impl Dispatch<wayland_client::protocol::wl_buffer::WlBuffer, ()> for App {
    fn event(
        _: &mut Self,
        _: &wayland_client::protocol::wl_buffer::WlBuffer,
        _: wayland_client::protocol::wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

struct StopOnDrop(ControlHandle);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// A file of `bytes` bytes that has the shape of a linear dmabuf's plane.
fn plane(bytes: usize) -> OwnedFd {
    let file = tempfile::tempfile().expect("a temporary file");
    file.set_len(bytes as u64).expect("a sized file");
    OwnedFd::from(file)
}

#[test]
fn the_vulkan_renderer_offers_linear_buffers_creates_one_and_refuses_a_tiled_one() {
    let backend = match Headless::with_config(HeadlessConfig::vulkan()) {
        Ok(backend) => backend,
        Err(error) => {
            println!("skipping: no Vulkan renderer on this machine ({error})");
            return;
        }
    };
    let directory = tempfile::tempdir().expect("a temporary directory");
    let socket = directory.path().join("wayland-0.sock");
    let config = Config {
        socket_path: Some(socket.clone()),
        ..stock::config()
    };
    let server = Server::new(Box::new(backend), config).expect("the session starts");
    assert!(
        server.state().serves_dmabuf(),
        "the Vulkan renderer offers linear dmabufs, so the compositor serves zwp_linux_dmabuf_v1"
    );
    let control = server.control();

    let worker = thread::spawn(move || {
        let _stop = StopOnDrop(control);
        let stream = UnixStream::connect(&socket).expect("the session is listening");
        let connection = Connection::from_socket(stream).expect("a client connection");
        let (globals, mut queue) = registry_queue_init::<App>(&connection).expect("the global list");
        let qh = queue.handle();
        let mut app = App::default();
        // No render node, no feedback object: version 3 names formats and modifiers.
        let global: dmabuf_global::ZwpLinuxDmabufV1 = globals.bind(&qh, 3..=3, ()).expect("zwp_linux_dmabuf_v1 v3");
        queue.roundtrip(&mut app).expect("a round trip");

        assert!(!app.offered.is_empty(), "the compositor offered formats");
        assert!(
            app.offered.iter().all(|(_, modifier)| *modifier == LINEAR),
            "only the linear modifier is offered: {:?}",
            app.offered
        );
        let argb = Fourcc::Argb8888 as u32;
        assert!(app.offered.contains(&(argb, LINEAR)), "ARGB8888 linear is offered");

        let (width, height) = (16u32, 8u32);
        let stride = width * 4;
        let share = |fd: std::os::fd::BorrowedFd<'_>, modifier: u64| {
            let params = global.create_params(&qh, ());
            params.add(fd, 0, 0, stride, (modifier >> 32) as u32, modifier as u32);
            params.create(width as i32, height as i32, argb, params::Flags::empty());
        };
        let answered = |app: &mut App, queue: &mut wayland_client::EventQueue<App>, count: usize| {
            let deadline = Instant::now() + TIMEOUT;
            while app.created + app.failed < count {
                assert!(
                    Instant::now() < deadline,
                    "the compositor never answered buffer {count}"
                );
                queue.roundtrip(app).expect("a round trip");
            }
        };

        // A file shaped like a linear plane is no dma-buf: the renderer reads the plane in the compositor's own process,
        // so a file the client can resize would fault the copy. It is refused, and the session lives on.
        let file = plane((stride * height) as usize);
        share(std::os::fd::AsFd::as_fd(&file), LINEAR);
        queue.roundtrip(&mut app).expect("a round trip");
        answered(&mut app, &mut queue, 1);
        assert_eq!(
            (app.created, app.failed),
            (0, 1),
            "a plane that is no dma-buf is refused, whatever shape it has"
        );

        // A real dma-buf, which needs `/dev/udmabuf`, is created.
        let mut expected = (0, 1);
        let real = UdmabufAllocator::new()
            .map_err(|error| error.to_string())
            .and_then(|mut allocator| {
                allocator
                    .create_buffer(width, height, Fourcc::Argb8888, &[Modifier::Linear])
                    .map_err(|error| error.to_string())
            });
        let _real = match real {
            Ok(buffer) => {
                let fd = buffer
                    .handles()
                    .next()
                    .expect("a plane")
                    .try_clone_to_owned()
                    .expect("a copy");
                share(std::os::fd::AsFd::as_fd(&fd), LINEAR);
                answered(&mut app, &mut queue, 2);
                assert_eq!(
                    (app.created, app.failed),
                    (1, 1),
                    "a single linear dma-buf plane in an offered format is created"
                );
                expected = (1, 1);
                Some((buffer, fd))
            }
            Err(reason) => {
                println!("skipping the created half: no dma-buf source here ({reason}); it runs where udmabuf loads");
                None
            }
        };

        let tiled = plane((stride * height) as usize);
        share(std::os::fd::AsFd::as_fd(&tiled), X_TILED);
        answered(&mut app, &mut queue, 2 + usize::from(expected.0 == 1));
        expected.1 += 1;
        assert_eq!(
            (app.created, app.failed),
            expected,
            "a tiled buffer is refused cleanly, and the session lives on"
        );
        queue.roundtrip(&mut app).expect("the connection is still up");
    });
    let outcome: Result<(), LoopError> = server.run();
    if let Err(payload) = worker.join() {
        std::panic::resume_unwind(payload);
    }
    outcome.expect("the loop stops cleanly");
}
