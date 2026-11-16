//! A nested session presenting through its host's `zwp_linux_dmabuf_v1`, and falling back to shared memory.
//!
//! # What this proves, and what it cannot
//!
//! The Agent37 build instances have no DRM device (`/dev/dri` is absent in their gVisor sandbox), so no process there can
//! allocate a GPU buffer and no driver can import one: llvmpipe and lavapipe draw, but they cannot export or import a
//! dmabuf without a device node. The two halves of the path that need a GPU are therefore stand-ins:
//!
//! - the **host's import** is `tests/mock_gpu`, which accepts the buffers a client sends (their planes are files the test
//!   holds) and answers from them;
//! - the **session's allocation and fill** is [`StandIn`] below, which makes a buffer from a file and writes the frame the
//!   renderer drew into it.
//!
//! **Everything else is real**: the host's `zwp_linux_dmabuf_v1` (feedback, format table, `create_params`, `create`,
//! `created` and `failed`), the nested backend's buffer ring, the attach, damage and commit, the host's release, the
//! pacing, the resize, and the fallback. What stays unproven is GBM allocation on a real device, and the GLES renderer's
//! copy into a buffer it allocated: `gnoblin-render-gles`'s own tests and `tests/dmabuf_capture_gpu.rs` prove the copy on a
//! machine with a GPU, and `docs/real-hardware-verification.md` lists the owner-run check of the whole path.

mod mock_gpu;
mod nested_host;
mod stock;

use std::fs::File;
use std::os::unix::fs::FileExt;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::dmabuf::DmabufProvider;
use gnoblin_backend::nested::{DmabufPresenter, Nested, NestedConfig, PresentRefusal};
use gnoblin_backend::render::renderer::AnyRenderer;
use gnoblin_backend::{Backend, DmabufCopy, DmabufImportError, DmabufSupport};
use gnoblin_render_frame::error::RenderError;
use gnoblin_types::{Point, Rect, Scale, Size};
use mock_gpu::Mock;
use nested_host::*;
use smithay::backend::allocator::dmabuf::{Dmabuf, DmabufFlags};
use smithay::backend::allocator::{Buffer as _, Format};

/// Every buffer a stand-in allocated: its size and the file behind its plane.
type Made = Arc<Mutex<Vec<(Size, File)>>>;

/// The session's side of a GPU: a presenter that allocates from files and fills them with the frame.
struct StandIn {
    /// Every buffer it allocated: its size and the file behind its plane.
    made: Made,
    /// The stand-in's own copy engine, which writes a picture into a plane the way a GPU copy would.
    copy: Mock,
}

impl StandIn {
    fn new() -> (Self, Made) {
        let made = Arc::new(Mutex::new(Vec::new()));
        let (copy, _controls) = Mock::capable();
        (
            Self {
                made: Arc::clone(&made),
                copy,
            },
            made,
        )
    }
}

impl DmabufPresenter for StandIn {
    fn can_fill(&self, _renderer: &mut AnyRenderer) -> bool {
        true
    }

    fn allocate(&mut self, _device: Option<u64>, size: Size, formats: &[Format]) -> Result<Dmabuf, PresentRefusal> {
        let format = formats
            .first()
            .ok_or_else(|| PresentRefusal::Allocation("no format".to_owned()))?;
        let stride = size.width * 4;
        let file = tempfile::tempfile().map_err(|error| PresentRefusal::Allocation(error.to_string()))?;
        file.set_len(u64::from(stride) * u64::from(size.height))
            .map_err(|error| PresentRefusal::Allocation(error.to_string()))?;
        let plane = file
            .try_clone()
            .map_err(|error| PresentRefusal::Allocation(error.to_string()))?;
        let mut builder = Dmabuf::builder(
            (
                i32::try_from(size.width).unwrap_or(0),
                i32::try_from(size.height).unwrap_or(0),
            ),
            format.code,
            format.modifier,
            DmabufFlags::empty(),
        );
        assert!(builder.add_plane(std::os::fd::OwnedFd::from(plane), 0, stride));
        self.made.lock().expect("the list").push((size, file));
        builder
            .build()
            .ok_or_else(|| PresentRefusal::Allocation("a buffer needs a plane".to_owned()))
    }

    fn fill(&mut self, renderer: &mut AnyRenderer, buffer: &Dmabuf) -> Result<(), PresentRefusal> {
        let picture = renderer.capture(true).map_err(PresentRefusal::Fill)?;
        let size = buffer.size();
        let region = Rect::new(0, 0, size.w.unsigned_abs(), size.h.unsigned_abs());
        self.copy
            .write(
                buffer,
                &picture,
                DmabufCopy {
                    region,
                    from: Point::ZERO,
                },
            )
            .map_err(|_| PresentRefusal::Fill(RenderError::Failed))
    }
}

/// A host GPU that imports nothing a client sends: the host answers `failed` to every `create`.
struct RefusesEveryBuffer {
    inner: Mock,
}

impl DmabufProvider for RefusesEveryBuffer {
    fn support(&mut self) -> Option<DmabufSupport> {
        self.inner.support()
    }

    fn import(&mut self, _dmabuf: &Dmabuf) -> Result<(), DmabufImportError> {
        Err(DmabufImportError::Refused(RenderError::Failed))
    }
}

/// Draws frames, serving the host's connection, until `done` says what the test waited for happened.
fn present_until(nested: &mut Nested, probe: &mut Probe, what: &str, done: impl Fn(&Nested) -> bool) {
    let output = only_output(nested).id;
    let deadline = Instant::now() + TIMEOUT;
    loop {
        nested.roundtrip().expect("the host answers a round trip");
        let _ = nested.dispatch(probe);
        let _ = nested.render(probe, output);
        if done(nested) {
            return;
        }
        assert!(Instant::now() < deadline, "{what}");
        thread::sleep(Duration::from_millis(5));
    }
}

/// The pixel at `(x, y)` of a stand-in buffer's plane, as `[r, g, b, a]`.
fn plane_pixel(file: &File, width: u32, x: u32, y: u32) -> [u8; 4] {
    let mut bytes = [0u8; 4];
    file.read_exact_at(&mut bytes, u64::from(y) * u64::from(width) * 4 + u64::from(x) * 4)
        .expect("a pixel of the plane");
    // `argb8888` is B, G, R, A in memory.
    [bytes[2], bytes[1], bytes[0], bytes[3]]
}

#[test]
fn a_frame_is_presented_through_the_hosts_dmabuf() {
    let (mock, host_gpu) = Mock::capable();
    let session = Session::start_gpu(Box::new(mock));
    session.run(move |host| {
        let (stand_in, made) = StandIn::new();
        let mut nested = Nested::connect(&NestedConfig::new(host.socket()))
            .expect("the session connects")
            .with_dmabuf_presenter(Box::new(stand_in));
        let mut probe = Probe::drawing(RED);
        let output = only_output(&nested);
        present_until(
            &mut nested,
            &mut probe,
            "no frame reached the host as a GPU buffer",
            |nested| nested.gpu_presents(output.id) > 0,
        );
        assert!(!nested.gpu_retired(output.id), "the host took what it was given");
        assert!(
            host_gpu.imports() >= 2,
            "the host imported both buffers of the ring: {}",
            host_gpu.imports()
        );
        let made = made.lock().expect("the list");
        assert!(
            made.len() >= 2
                && made
                    .iter()
                    .all(|(size, _)| *size == Size::new(output.mode.width, output.mode.height)),
            "the buffers are the size of the output's mode: {:?}",
            made.iter().map(|(size, _)| *size).collect::<Vec<_>>()
        );
        // The frame was drawn into the buffer the host imported, so the host reads the session's red from it.
        let drawn: Vec<[u8; 4]> = made
            .iter()
            .map(|(size, file)| plane_pixel(file, size.width, 5, 5))
            .filter(|pixel| *pixel != [0, 0, 0, 0])
            .collect();
        assert!(
            drawn.contains(&[255, 0, 0, 255]),
            "a buffer holds the red the session drew: {drawn:?}"
        );
    });
}

#[test]
fn frames_keep_coming_through_the_ring_and_the_host_releases_each_buffer() {
    let (mock, _host_gpu) = Mock::capable();
    let session = Session::start_gpu(Box::new(mock));
    session.run(move |host| {
        let (stand_in, _made) = StandIn::new();
        let mut nested = Nested::connect(&NestedConfig::new(host.socket()))
            .expect("the session connects")
            .with_dmabuf_presenter(Box::new(stand_in));
        let mut probe = Probe::drawing(RED);
        let output = only_output(&nested).id;
        // More frames than the ring has buffers: each needs the host's release of an earlier one.
        present_until(
            &mut nested,
            &mut probe,
            "the ring stalled: the host never released a buffer",
            |nested| nested.gpu_presents(output) >= 6,
        );
        assert!(!nested.gpu_retired(output));
    });
}

#[test]
fn a_host_that_refuses_the_buffers_gets_the_frames_through_shared_memory() {
    let (mock, _controls) = Mock::capable();
    let session = Session::start_gpu(Box::new(RefusesEveryBuffer { inner: mock }));
    session.run(move |host| {
        let (stand_in, _made) = StandIn::new();
        let mut nested = Nested::connect(&NestedConfig::new(host.socket()))
            .expect("the session connects")
            .with_dmabuf_presenter(Box::new(stand_in));
        let mut probe = Probe::drawing(RED);
        let output = only_output(&nested).id;
        present_until(
            &mut nested,
            &mut probe,
            "the host's refusal never reached the target",
            |nested| nested.gpu_retired(output),
        );
        assert_eq!(
            nested.gpu_presents(output),
            0,
            "no frame was sent as a buffer the host refused"
        );
        // The session still shows: frames go through shared memory, and the host composes them.
        host.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        draw_rendered(&mut nested, &mut probe);
        host.wait_for_colour(window_owner(host), centre(&mapped_host_window(host)), RED_PIXEL);
    });
}

/// The host's only window, once it is drawn.
fn mapped_host_window(host: &Handle) -> gnoblin_wayland::ObservedWindow {
    host.wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
        .only()
        .expect("one mapped window")
        .clone()
}

#[test]
fn a_host_with_no_dmabuf_gets_shared_memory() {
    let session = Session::start();
    session.run(|host| {
        let (stand_in, made) = StandIn::new();
        let mut nested = Nested::connect(&NestedConfig::new(host.socket()))
            .expect("the session connects")
            .with_dmabuf_presenter(Box::new(stand_in));
        let mut probe = Probe::drawing(RED);
        let output = only_output(&nested).id;
        draw_rendered(&mut nested, &mut probe);
        draw_rendered(&mut nested, &mut probe);
        assert_eq!(nested.gpu_presents(output), 0, "this host imports no GPU buffer");
        assert!(nested.gpu_retired(output), "so the path was never on");
        assert!(
            made.lock().expect("the list").is_empty(),
            "and nothing was allocated for it"
        );
    });
}

#[test]
fn a_session_that_does_not_ask_for_the_gpu_path_never_binds_it() {
    let (mock, host_gpu) = Mock::capable();
    let session = Session::start_gpu(Box::new(mock));
    session.run(move |host| {
        let (stand_in, made) = StandIn::new();
        let config = NestedConfig {
            dmabuf: false,
            ..NestedConfig::new(host.socket())
        };
        let mut nested = Nested::connect(&config)
            .expect("the session connects")
            .with_dmabuf_presenter(Box::new(stand_in));
        let mut probe = Probe::drawing(RED);
        let output = only_output(&nested).id;
        draw_rendered(&mut nested, &mut probe);
        draw_rendered(&mut nested, &mut probe);
        assert_eq!(nested.gpu_presents(output), 0);
        assert_eq!(host_gpu.imports(), 0, "the host imported nothing");
        assert!(made.lock().expect("the list").is_empty());
    });
}

/// A presenter whose device cannot be allocated on, which is what the production one finds on a machine with no render node.
struct NoDevice;

impl DmabufPresenter for NoDevice {
    fn can_fill(&self, _renderer: &mut AnyRenderer) -> bool {
        true
    }

    fn allocate(&mut self, device: Option<u64>, _size: Size, _formats: &[Format]) -> Result<Dmabuf, PresentRefusal> {
        Err(PresentRefusal::NoDevice(format!("no node for {device:?}")))
    }

    fn fill(&mut self, _renderer: &mut AnyRenderer, _buffer: &Dmabuf) -> Result<(), PresentRefusal> {
        Err(PresentRefusal::Fill(RenderError::Unsupported))
    }
}

#[test]
fn a_host_whose_device_cannot_be_allocated_on_gets_shared_memory() {
    let (mock, _controls) = Mock::capable();
    let session = Session::start_gpu(Box::new(mock));
    session.run(move |host| {
        let mut nested = Nested::connect(&NestedConfig::new(host.socket()))
            .expect("the session connects")
            .with_dmabuf_presenter(Box::new(NoDevice));
        let mut probe = Probe::drawing(RED);
        let output = only_output(&nested).id;
        draw_rendered(&mut nested, &mut probe);
        draw_rendered(&mut nested, &mut probe);
        assert_eq!(
            nested.gpu_presents(output),
            0,
            "no buffer could be allocated on the host's device"
        );
        assert!(nested.gpu_retired(output), "so the path was given up");
        host.wait_for_colour(window_owner(host), centre(&mapped_host_window(host)), RED_PIXEL);
    });
}

#[test]
fn the_production_presenter_is_only_tried_where_the_renderer_can_take_part() {
    let (mock, _controls) = Mock::capable();
    let session = Session::start_gpu(Box::new(mock));
    session.run(move |host| {
        // The software renderer draws into no dmabuf, so the GBM presenter is never asked for a buffer.
        let mut nested = Nested::connect(&NestedConfig::new(host.socket())).expect("the session connects");
        let mut probe = Probe::drawing(RED);
        let output = only_output(&nested).id;
        draw_rendered(&mut nested, &mut probe);
        draw_rendered(&mut nested, &mut probe);
        assert_eq!(nested.gpu_presents(output), 0);
        host.wait_for_colour(window_owner(host), centre(&mapped_host_window(host)), RED_PIXEL);
    });
}

#[test]
fn a_resize_builds_new_buffers_of_the_new_size() {
    let (mock, _host_gpu) = Mock::capable();
    let session = Session::start_gpu(Box::new(mock));
    session.run(move |host| {
        let (stand_in, made) = StandIn::new();
        let mut nested = Nested::connect(&NestedConfig::new(host.socket()))
            .expect("the session connects")
            .with_dmabuf_presenter(Box::new(stand_in));
        let mut probe = Probe::drawing(RED);
        let first = only_output(&nested);
        present_until(&mut nested, &mut probe, "no GPU frame at the first size", |nested| {
            nested.gpu_presents(first.id) > 0
        });
        let before = nested.gpu_presents(first.id);

        host.control.set_global_scale(Some(2));
        present_until(
            &mut nested,
            &mut probe,
            "the session never followed the host's scale",
            |nested| {
                session_outputs(nested)
                    .first()
                    .is_some_and(|output| output.scale == Scale::integral(2))
            },
        );
        let now = only_output(&nested);
        present_until(&mut nested, &mut probe, "no GPU frame at the new size", |nested| {
            nested.gpu_presents(now.id) > before + 1
        });
        let sizes: Vec<Size> = made.lock().expect("the list").iter().map(|(size, _)| *size).collect();
        assert!(
            sizes.contains(&Size::new(now.mode.width, now.mode.height)),
            "buffers of the new size were made: {sizes:?}"
        );
        assert!(!nested.gpu_retired(now.id), "a resize does not retire the path");
    });
}
