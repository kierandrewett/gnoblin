//! Supersampling on the real compositor: an output composed at `mode * s` and shown at its mode (design/supersample.md).
//!
//! Everything runs the real `State`, the real frame loop and the software renderer on virtual (headless) outputs.
//! A stand-in shell sends `screen.apply`, real `wayland-client` connections bind `wl_output`, `zxdg_output_manager_v1`,
//! `wp_fractional_scale_manager_v1` and the capture protocols, and pixels are read back through the backend. What it
//! does not prove: scan-out on a DRM device and the GLES downscale pass (`gnoblin-render-gles`'s own tests do).

mod session;

use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::{DEFAULT_CONNECTOR, DEFAULT_MODE, HeadlessHandle};
use gnoblin_output::DisplayMode;
use gnoblin_render::transaction::TreeValue;
use gnoblin_render_frame::image::Image;
use gnoblin_types::{NodeId, OutputId, Point, Rect, Scale, Size};
use session::shell::Shell;
use session::{Client, Handle, Session};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_compositor, wl_output, wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_image_capture_source_v1, ext_output_image_capture_source_manager_v1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_manager_v1, ext_image_copy_capture_session_v1,
};
use wayland_protocols::wp::fractional_scale::v1::client::{wp_fractional_scale_manager_v1, wp_fractional_scale_v1};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};
use wayland_protocols_wlr::screencopy::v1::client::{zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1};

const RED: [u8; 4] = [255, 0, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];
/// Opaque red and white in the byte order `wl_shm` stores `argb8888` in.
const RED_BGRA: [u8; 4] = [0, 0, 255, 255];
const WHITE_BGRA: [u8; 4] = [255, 255, 255, 255];
const WAIT: Duration = Duration::from_secs(10);

/// A session with a shell attached and outputs a test can plug.
fn start() -> (Session, HeadlessHandle, Shell) {
    let (shell, hooks) = Shell::new();
    let (session, outputs) = Session::start_shelled_detached(hooks);
    (session, outputs, shell)
}

/// Polls `check` until it holds, or fails with `what`.
#[track_caller]
fn until<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(Instant::now() < deadline, "never saw {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The id of the output with this connector, once the session presents on it.
#[track_caller]
fn id_of(handle: &Handle, connector: &str) -> OutputId {
    until(&format!("an output named {connector}"), || {
        handle
            .windows()
            .output_details()
            .iter()
            .find(|output| output.name == connector)
            .map(|output| output.id)
    })
}

/// The screen node whose published connector is `connector`.
#[track_caller]
fn node_of(shell: &Shell, connector: &str) -> NodeId {
    until(&format!("a screen node for {connector}"), || {
        shell
            .screen_nodes()
            .into_iter()
            .rfind(|node| shell.published(*node, "connector") == Some(TreeValue::Text(connector.to_owned())))
    })
}

/// The composed picture of an output, once `accept` takes it.
#[track_caller]
fn picture(handle: &Handle, output: OutputId, accept: impl Fn(&Image) -> bool) -> Image {
    until("the expected picture", || {
        handle.control.screenshot(output).filter(|image| accept(image))
    })
}

/// The picture as the display shows it, once `accept` takes it.
#[track_caller]
fn display_picture(handle: &Handle, output: OutputId, accept: impl Fn(&Image) -> bool) -> Image {
    until("the expected display picture", || {
        handle.control.screenshot_display(output).filter(|image| accept(image))
    })
}

fn pixel(image: &Image, x: i32, y: i32) -> [u8; 4] {
    image.pixel(Point::new(x, y)).expect("a pixel inside the picture")
}

/// Puts a window node at `at` with `size`, as the shell's placement does.
fn place(shell: &Shell, node: NodeId, at: (f64, f64), size: (f64, f64)) {
    shell.write(
        node,
        &[
            ("x", TreeValue::Double(at.0)),
            ("y", TreeValue::Double(at.1)),
            ("width", TreeValue::Double(size.0)),
            ("height", TreeValue::Double(size.1)),
        ],
    );
}

/// What a probe connection has been told.
#[derive(Default, Debug)]
struct Seen {
    output_scale: Option<i32>,
    output_mode: Option<(i32, i32)>,
    xdg_size: Option<(i32, i32)>,
    xdg_position: Option<(i32, i32)>,
    preferred: Option<u32>,
    screencopy_buffer: Option<(u32, u32)>,
    image_copy_buffer: Option<(u32, u32)>,
}

/// A real client connection that binds the protocols supersampling touches and records what they say.
struct Probe {
    queue: EventQueue<Seen>,
    seen: Seen,
    qh: QueueHandle<Seen>,
    compositor: wl_compositor::WlCompositor,
    output: wl_output::WlOutput,
    fractional: Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    screencopy: Option<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1>,
    sources: Option<ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1>,
    image_copy: Option<ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1>,
    _keep: Vec<Box<dyn std::any::Any>>,
}

impl Probe {
    /// Connects to `socket` and binds `wl_output`, its `xdg_output` and the fractional scale manager, and whatever
    /// capture globals the socket offers.
    fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("the session is listening");
        stream.set_read_timeout(Some(WAIT)).expect("a read timeout");
        let connection = Connection::from_socket(stream).expect("a client connection");
        let (globals, queue) = registry_queue_init::<Seen>(&connection).expect("the global list");
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=6, ()).expect("wl_compositor");
        let output: wl_output::WlOutput = globals.bind(&qh, 1..=4, ()).expect("wl_output");
        let xdg: zxdg_output_manager_v1::ZxdgOutputManagerV1 = globals.bind(&qh, 1..=3, ()).expect("xdg-output");
        let xdg_output = xdg.get_xdg_output(&output, &qh, ());
        let mut probe = Self {
            queue,
            seen: Seen::default(),
            fractional: globals.bind(&qh, 1..=1, ()).ok(),
            screencopy: globals.bind(&qh, 1..=3, ()).ok(),
            sources: globals.bind(&qh, 1..=1, ()).ok(),
            image_copy: globals.bind(&qh, 1..=1, ()).ok(),
            qh,
            compositor,
            output,
            _keep: vec![Box::new(connection), Box::new(xdg), Box::new(xdg_output)],
        };
        probe.roundtrip();
        probe
    }

    fn roundtrip(&mut self) {
        self.queue.roundtrip(&mut self.seen).expect("a round trip");
    }

    /// Round-trips until `check` holds for what has been seen.
    #[track_caller]
    fn wait(&mut self, what: &str, check: impl Fn(&Seen) -> bool) {
        let deadline = Instant::now() + WAIT;
        loop {
            self.roundtrip();
            if check(&self.seen) {
                return;
            }
            assert!(Instant::now() < deadline, "never saw {what}: {:?}", self.seen);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// A surface with a fractional scale object, which is told its scale when the object is made.
    fn scaled_surface(&mut self) -> (wl_surface::WlSurface, wp_fractional_scale_v1::WpFractionalScaleV1) {
        let manager = self.fractional.clone().expect("wp_fractional_scale_manager_v1");
        let surface = self.compositor.create_surface(&self.qh, ());
        let object = manager.get_fractional_scale(&surface, &self.qh, ());
        (surface, object)
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Seen {
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

impl Dispatch<wl_output::WlOutput, ()> for Seen {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_output::Event::Scale { factor } => state.output_scale = Some(factor),
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                width,
                height,
                ..
            } if flags.contains(wl_output::Mode::Current) => state.output_mode = Some((width, height)),
            _ => {}
        }
    }
}

impl Dispatch<zxdg_output_v1::ZxdgOutputV1, ()> for Seen {
    fn event(
        state: &mut Self,
        _: &zxdg_output_v1::ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zxdg_output_v1::Event::LogicalSize { width, height } => state.xdg_size = Some((width, height)),
            zxdg_output_v1::Event::LogicalPosition { x, y } => state.xdg_position = Some((x, y)),
            _ => {}
        }
    }
}

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, ()> for Seen {
    fn event(
        state: &mut Self,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            state.preferred = Some(scale);
        }
    }
}

impl Dispatch<zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, ()> for Seen {
    fn event(
        state: &mut Self,
        _: &zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_screencopy_frame_v1::Event::Buffer { width, height, .. } = event {
            state.screencopy_buffer = Some((width, height));
        }
    }
}

impl Dispatch<ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1, ()> for Seen {
    fn event(
        state: &mut Self,
        _: &ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_image_copy_capture_session_v1::Event::BufferSize { width, height } = event {
            state.image_copy_buffer = Some((width, height));
        }
    }
}

delegate_noop!(Seen: wl_compositor::WlCompositor);
delegate_noop!(Seen: ignore wl_surface::WlSurface);
delegate_noop!(Seen: zxdg_output_manager_v1::ZxdgOutputManagerV1);
delegate_noop!(Seen: wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);
delegate_noop!(Seen: zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1);
delegate_noop!(Seen: ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(Seen: ignore ext_image_capture_source_v1::ExtImageCaptureSourceV1);
delegate_noop!(Seen: ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1);

/// Maps a white window with one red column and places it. The buffer is `buffer_width` by `buffer_height` pixels at
/// `buffer_scale`, the red column `column` pixels in from the left edge, and the node is placed at `at` with the
/// buffer's logical size.
fn striped_window(
    handle: &Handle,
    shell: &Shell,
    at: (f64, f64),
    buffer: (i32, i32),
    buffer_scale: i32,
    column: i32,
) -> Client {
    use std::io::Write;
    use std::os::fd::AsFd;

    let mut client = Client::connect(handle.socket());
    let window = client.create_window("org.example.striped", "Striped");
    let stride = buffer.0 * 4;
    let mut file = tempfile::tempfile().expect("a shared memory file");
    for _ in 0..buffer.1 {
        let mut row = Vec::new();
        for x in 0..buffer.0 {
            row.extend_from_slice(if x == column { &RED_BGRA } else { &WHITE_BGRA });
        }
        file.write_all(&row).expect("a filled buffer");
    }
    let pool = client.shm.create_pool(file.as_fd(), stride * buffer.1, &client.qh, ());
    let wl_buffer = pool.create_buffer(
        0,
        buffer.0,
        buffer.1,
        stride,
        wayland_client::protocol::wl_shm::Format::Argb8888,
        &client.qh,
        (),
    );
    window.surface.set_buffer_scale(buffer_scale);
    window.surface.attach(Some(&wl_buffer), 0, 0);
    window.surface.damage_buffer(0, 0, buffer.0, buffer.1);
    window.surface.commit();
    client.roundtrip();
    let node = shell.wait_window_nodes(1)[0];
    place(
        shell,
        node,
        at,
        (f64::from(buffer.0 / buffer_scale), f64::from(buffer.1 / buffer_scale)),
    );
    client
}

/// The average of the `ratio x ratio` block of `image` that is the pixel `(x, y)` of the display frame, per channel,
/// rounded half up: computed here from the composed frame, independently of the compositor's own downsampling.
fn block_average(image: &Image, x: i32, y: i32, ratio: i32) -> [u8; 4] {
    let mut sums = [0u32; 4];
    for row in 0..ratio {
        for column in 0..ratio {
            let texel = pixel(image, x * ratio + column, y * ratio + row);
            for (sum, channel) in sums.iter_mut().zip(texel) {
                *sum += u32::from(channel);
            }
        }
    }
    let count = (ratio * ratio) as u32;
    sums.map(|sum| ((sum + count / 2) / count) as u8)
}

#[test]
fn a_supersampled_output_keeps_its_logical_size_everywhere() {
    let (session, _outputs, shell) = start();
    session.run(move |handle| {
        shell.attach();
        let output = id_of(handle, DEFAULT_CONNECTOR);
        let node = node_of(&shell, DEFAULT_CONNECTOR);
        let logical = Rect::new(0, 0, DEFAULT_MODE.width, DEFAULT_MODE.height);
        let mode = (DEFAULT_MODE.width as i32, DEFAULT_MODE.height as i32);

        let mut probe = Probe::connect(handle.socket());
        let (_surface, _fractional) = probe.scaled_surface();
        probe.wait("the unsupersampled state", |seen| {
            seen.output_scale == Some(1) && seen.xdg_size.is_some() && seen.preferred == Some(120)
        });
        assert_eq!(probe.seen.output_mode, Some(mode));
        assert_eq!(probe.seen.xdg_size, Some(mode));
        assert_eq!(handle.windows().output_rect(output), Some(logical));
        assert_eq!(picture(handle, output, |_| true).size(), Size::new(1024, 768));

        // A factor of two: clients are told two, the mode stays the real one, and the desktop does not change size.
        shell.apply(node, "supersample=2").expect("a factor of two applies");
        shell.wait_published(node, "supersample", &TreeValue::Double(2.0));
        probe.wait("scale 2 and a preferred scale of 240", |seen| {
            seen.output_scale == Some(2) && seen.preferred == Some(240)
        });
        assert_eq!(
            probe.seen.output_mode,
            Some(mode),
            "wl_output keeps reporting the real mode"
        );
        assert_eq!(probe.seen.xdg_size, Some(mode), "xdg_output keeps the logical size");
        assert_eq!(probe.seen.xdg_position, Some((0, 0)));
        assert_eq!(
            handle.windows().output_rect(output),
            Some(logical),
            "the layout rectangle did not move"
        );
        assert_eq!(
            picture(handle, output, |image| image.size().width == 2048).size(),
            Size::new(2048, 1536)
        );

        // The user scale composes with it: logical size halves, clients are told four, the frame is the same size.
        shell.apply(node, "scale=2").expect("a scale applies under a factor");
        let halved = Rect::new(0, 0, 512, 384);
        until("the halved rectangle", || {
            (handle.windows().output_rect(output) == Some(halved)).then_some(())
        });
        probe.wait(
            "scale 4, a preferred scale of 480 and the halved logical size",
            |seen| seen.output_scale == Some(4) && seen.preferred == Some(480) && seen.xdg_size == Some((512, 384)),
        );
        assert_eq!(probe.seen.output_mode, Some(mode));
        assert_eq!(picture(handle, output, |_| true).size(), Size::new(1024 * 2, 768 * 2));

        // Back to one: the scale is two again and the frame is the mode.
        shell.apply(node, "supersample=1").expect("the factor goes back to one");
        probe.wait("scale 2 and a preferred scale of 240", |seen| {
            seen.output_scale == Some(2) && seen.preferred == Some(240)
        });
        assert_eq!(
            picture(handle, output, |image| image.size().width == 1024).size(),
            Size::new(1024, 768)
        );
        shell.wait_published(node, "supersample", &TreeValue::Double(1.0));
    });
}

#[test]
fn the_composed_frame_is_hi_res_and_the_display_frame_is_its_exact_box_average() {
    let (session, _outputs, shell) = start();
    session.run(move |handle| {
        shell.attach();
        let output = id_of(handle, DEFAULT_CONNECTOR);
        let node = node_of(&shell, DEFAULT_CONNECTOR);
        let mut client = Client::connect(handle.socket());
        let _window = client.map_solid_window("org.example.red", "Red", (200, 150), RED_BGRA);
        let window = shell.wait_window_nodes(1)[0];
        place(&shell, window, (100.0, 100.0), (200.0, 150.0));
        picture(handle, output, |image| pixel(image, 150, 150) == RED);

        shell.apply(node, "supersample=2").expect("a factor of two applies");
        let composed = picture(handle, output, |image| {
            image.size().width == 2048 && pixel(image, 300, 300) == RED
        });
        // The window covers 100..300 by 100..250 logical pixels: 200..600 by 200..500 composed pixels, edges exact.
        // The window's corners are rounded, so the edges are probed at their middles.
        assert_eq!(pixel(&composed, 400, 206), RED);
        assert_eq!(pixel(&composed, 400, 494), RED);
        assert_eq!(pixel(&composed, 206, 350), RED);
        assert_eq!(pixel(&composed, 594, 350), RED);
        assert_eq!(pixel(&composed, 400, 199), BLACK);
        assert_eq!(pixel(&composed, 199, 350), BLACK);
        assert_eq!(pixel(&composed, 600, 350), BLACK);

        let shown = display_picture(handle, output, |image| {
            image.size().width == 1024 && pixel(image, 150, 150) == RED
        });
        assert_eq!(shown.size(), Size::new(1024, 768));
        // Every pixel of the display frame is the average of its 2x2 block of the composed frame, checked on a
        // pattern of places that includes the window's edge. The composed frame is read again after the display
        // one, so a frame drawn in between would show as a mismatch rather than be hidden.
        let composed_again = handle.control.screenshot(output).expect("a picture");
        for (x, y) in [
            (0, 0),
            (99, 99),
            (100, 100),
            (150, 120),
            (200, 110),
            (299, 249),
            (300, 250),
            (500, 700),
            (1023, 767),
        ] {
            assert_eq!(
                pixel(&shown, x, y),
                block_average(&composed_again, x, y, 2),
                "display pixel ({x}, {y})"
            );
        }
        // Integer geometry lands on whole display pixels, so the display frame equals what an unsupersampled output
        // would have drawn: the window's edge is sharp.
        assert_eq!(pixel(&shown, 200, 110), RED);
        assert_eq!(pixel(&shown, 200, 99), BLACK);
    });
}

#[test]
fn a_buffer_scale_two_client_is_not_resampled_and_a_one_pixel_line_is_its_coverage() {
    let (session, _outputs, shell) = start();
    session.run(move |handle| {
        shell.attach();
        let output = id_of(handle, DEFAULT_CONNECTOR);
        let node = node_of(&shell, DEFAULT_CONNECTOR);
        shell.apply(node, "supersample=2").expect("a factor of two applies");
        shell.wait_published(node, "supersample", &TreeValue::Double(2.0));

        // A 200x100 pixel buffer at buffer scale 2 is a 100x50 logical window at (20, 20). At a compositing scale of
        // two it is drawn one buffer pixel to one composed pixel. Its red column is buffer pixel 41.
        let _client = striped_window(handle, &shell, (20.0, 20.0), (200, 100), 2, 41);
        let composed = picture(handle, output, |image| {
            image.size().width == 2048 && pixel(image, 81, 60) == RED
        });
        assert_eq!(
            pixel(&composed, 80, 60),
            WHITE,
            "the column beside the line is untouched white"
        );
        assert_eq!(pixel(&composed, 82, 60), WHITE);
        assert_eq!(pixel(&composed, 81, 60), RED, "one buffer pixel is one composed pixel");
        assert_eq!(pixel(&composed, 40, 40), WHITE, "the window's first pixel is at 2 * 20");

        // On the display the line is half a display pixel wide, so its pixel is the average of red and white.
        let shown = display_picture(handle, output, |image| image.size().width == 1024);
        assert_eq!(
            pixel(&shown, 40, 30),
            [255, 128, 128, 255],
            "the line anti-aliases to its coverage on the display frame"
        );
        assert_eq!(pixel(&shown, 39, 30), WHITE);
        assert_eq!(pixel(&shown, 41, 30), WHITE);
    });
}

#[test]
fn a_factor_that_would_pass_the_memory_budget_is_refused_and_a_smaller_one_still_applies() {
    // The output's target is 3 MiB and the frame capture keeps 3 MiB. A factor of 4 composes at 4096x3072, which is 48
    // MiB and the copy it is shown from: 54 MiB more than the 40 MiB budget holds with nothing left to give back.
    // A factor of 2 is 12 MiB more and fits.
    let (shell, hooks) = Shell::new();
    let (session, _outputs) = Session::start_shelled_limited(
        hooks,
        gnoblin_wayland::Limits {
            budget_bytes: 40 << 20,
            client_bytes: 0,
            client_surfaces: 0,
        },
    );
    session.run(move |handle| {
        shell.attach();
        let node = node_of(&shell, DEFAULT_CONNECTOR);
        let refused = shell
            .apply(node, "supersample=4")
            .expect_err("a factor the budget cannot hold");
        assert!(refused.starts_with("memory: "), "{refused}");
        assert!(refused.contains("41943040"), "the message names the budget: {refused}");
        assert_eq!(
            shell.published(node, "supersample"),
            Some(TreeValue::Double(1.0)),
            "the output keeps the factor it had"
        );
        assert_eq!(handle.ledger.snapshot().refused, 1);

        shell.apply(node, "supersample=2").expect("a factor that fits applies");
        shell.wait_published(node, "supersample", &TreeValue::Double(2.0));
    });
}

#[test]
fn a_factor_that_cannot_be_composed_is_refused_by_name_and_changes_nothing() {
    let (session, outputs, shell) = start();
    session.run(move |handle| {
        shell.attach();
        let output = id_of(handle, DEFAULT_CONNECTOR);
        let node = node_of(&shell, DEFAULT_CONNECTOR);

        // 1024x768 at 16 is 16384x12288, above the 8192 limit; the message names the factor, the mode and the limit.
        let big = shell.apply(node, "supersample=16").expect_err("above the limit");
        assert!(big.starts_with("supersample: "), "{big}");
        assert!(
            big.contains("16384x12288") && big.contains("8192x8192") && big.contains("1024x768"),
            "{big}"
        );

        // Values that are not a factor throw at the key.
        for text in [
            "supersample=0.5",
            "supersample=NaN",
            "supersample=-2",
            "supersample=17",
            "supersample=two",
        ] {
            let error = shell.apply(node, text).expect_err(text);
            assert!(error.starts_with("supersample: "), "{text}: {error}");
        }

        // A factor whose virtual size is not whole: a 1367 pixel wide output at 1.5 would be 2050.5 wide.
        plug_odd(&outputs);
        let odd = id_of(handle, "Virtual-2");
        let odd_node = node_of(&shell, "Virtual-2");
        let fraction = shell.apply(odd_node, "supersample=1.5").expect_err("not a whole size");
        assert!(
            fraction.starts_with("supersample: ") && fraction.contains("2050.5"),
            "{fraction}"
        );

        // A backend that refuses the change leaves the output and the profile as they were.
        outputs.refuse_configure_after(0);
        let refused = shell.apply(node, "supersample=2").expect_err("the backend refuses");
        assert!(refused.starts_with("supersample: "), "{refused}");
        assert_eq!(
            picture(handle, output, |_| true).size(),
            Size::new(1024, 768),
            "the frame kept its size"
        );
        shell.wait_published(node, "supersample", &TreeValue::Double(1.0));
        // The refusal did not become the profile: an unrelated change does not drive a factor of two.
        shell
            .apply(node, "enabled=true")
            .expect("a request that changes nothing applies");
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(picture(handle, output, |_| true).size(), Size::new(1024, 768));
        assert_eq!(picture(handle, odd, |_| true).size(), Size::new(1367, 768));
    });
}

/// Plugs a 1367 by 768 output, whose width at a factor of 1.5 is not a whole number of pixels.
fn plug_odd(outputs: &HeadlessHandle) {
    outputs
        .connect_output("Virtual-2", DisplayMode::new(1367, 768, 60_000), Scale::integral(1))
        .expect("a mode that covers pixels");
}

#[test]
fn an_odd_factor_is_accepted_when_the_size_is_whole_and_composes_at_that_size() {
    let (session, _outputs, shell) = start();
    session.run(move |handle| {
        shell.attach();
        let output = id_of(handle, DEFAULT_CONNECTOR);
        let node = node_of(&shell, DEFAULT_CONNECTOR);
        shell.apply(node, "supersample=3").expect("an odd factor is accepted");
        shell.wait_published(node, "supersample", &TreeValue::Double(3.0));
        assert_eq!(
            picture(handle, output, |image| image.size().width == 3072).size(),
            Size::new(3072, 2304)
        );
        let shown = display_picture(handle, output, |image| image.size().width == 1024);
        assert_eq!(shown.size(), Size::new(1024, 768));
        shell
            .apply(node, "supersample=1.5")
            .expect("a fractional factor with a whole size is accepted");
        assert_eq!(
            picture(handle, output, |image| image.size().width == 1536).size(),
            Size::new(1536, 1152)
        );
        assert_eq!(
            display_picture(handle, output, |image| image.size().width == 1024).size(),
            Size::new(1024, 768),
            "the bilinear fallback still lands on the mode"
        );
    });
}

#[test]
fn a_mode_change_a_hotplug_and_a_mirror_keep_the_factor() {
    let (session, outputs, shell) = start();
    session.run(move |handle| {
        shell.attach();
        outputs
            .connect_output("Virtual-2", DisplayMode::new(640, 480, 60_000), Scale::integral(1))
            .expect("a mode that covers pixels");
        let second = id_of(handle, "Virtual-2");
        let node = node_of(&shell, "Virtual-2");
        shell.apply(node, "supersample=2").expect("a factor of two applies");
        assert_eq!(
            picture(handle, second, |image| image.size().width == 1280).size(),
            Size::new(1280, 960)
        );

        // A mode change keeps it: the frame is the new mode at twice the size.
        shell.apply(node, "mode=800x600@60").expect("a mode applies");
        assert_eq!(
            picture(handle, second, |image| image.size().width == 1600).size(),
            Size::new(1600, 1200)
        );
        shell.wait_published(node, "supersample", &TreeValue::Double(2.0));

        // Unplugging and plugging the same monitor on the same port brings the factor back with the mode.
        outputs.disconnect_output(second);
        until("the output to be gone", || {
            (handle.windows().outputs().len() == 1).then_some(())
        });
        outputs
            .connect_output("Virtual-2", DisplayMode::new(800, 600, 60_000), Scale::integral(1))
            .expect("a mode that covers pixels");
        let back = id_of(handle, "Virtual-2");
        // The node of the monitor that was unplugged is still in the shell's tree until its removal arrives, and it still
        // names the same connector and holds the same factor. The node that was plugged in is the one that is not it.
        let back_node = until("the screen node of the monitor that was plugged in", || {
            shell.screen_nodes().into_iter().rfind(|candidate| {
                *candidate != node
                    && shell.published(*candidate, "connector") == Some(TreeValue::Text("Virtual-2".to_owned()))
            })
        });
        shell.wait_published(back_node, "supersample", &TreeValue::Double(2.0));
        assert_eq!(
            picture(handle, back, |image| image.size().width == 1600).size(),
            Size::new(1600, 1200)
        );

        // A mirror of a supersampled output shows its display frame, fitted to the mirror's own panel.
        let first = id_of(handle, DEFAULT_CONNECTOR);
        let first_node = node_of(&shell, DEFAULT_CONNECTOR);
        shell
            .apply(first_node, "supersample=2")
            .expect("a factor of two applies");
        let mut client = Client::connect(handle.socket());
        let _window = client.map_solid_window("org.example.red", "Red", (200, 150), RED_BGRA);
        let window = shell.wait_window_nodes(1)[0];
        place(&shell, window, (100.0, 100.0), (200.0, 150.0));
        picture(handle, first, |image| {
            image.size().width == 2048 && pixel(image, 300, 300) == RED
        });
        shell
            .apply(back_node, "mirror_of=HEADLESS-1")
            .expect("the mirror applies");
        let mirrored = picture(handle, back, |image| pixel(image, 180, 180) == RED);
        assert_eq!(
            mirrored.size(),
            Size::new(1600, 1200),
            "the mirror keeps composing at its own factor"
        );
    });
}

#[test]
fn a_factor_changes_on_a_live_output_many_times_and_the_frame_follows() {
    let (session, _outputs, shell) = start();
    session.run(move |handle| {
        shell.attach();
        let output = id_of(handle, DEFAULT_CONNECTOR);
        let node = node_of(&shell, DEFAULT_CONNECTOR);
        for (factor, width) in [
            (2, 2048),
            (1, 1024),
            (4, 4096),
            (2, 2048),
            (1, 1024),
            (4, 4096),
            (1, 1024),
        ] {
            shell
                .apply(node, &format!("supersample={factor}"))
                .expect("a recommended factor applies");
            let frame = picture(handle, output, |image| image.size().width == width);
            assert_eq!(frame.size(), Size::new(width, 768 * width / 1024), "factor {factor}");
            assert_eq!(
                display_picture(handle, output, |image| image.size().width == 1024).size(),
                Size::new(1024, 768)
            );
        }
    });
}

#[test]
fn the_capture_protocols_advertise_the_composed_size() {
    let (session, _outputs, shell) = start();
    session.run(move |handle| {
        shell.attach();
        let _output = id_of(handle, DEFAULT_CONNECTOR);
        let node = node_of(&shell, DEFAULT_CONNECTOR);
        shell.apply(node, "supersample=2").expect("a factor of two applies");
        shell.wait_published(node, "supersample", &TreeValue::Double(2.0));

        // The capture globals are the session's own processes': the privileged socket.
        let mut probe = Probe::connect(handle.privileged_socket());
        probe.wait("scale 2", |seen| seen.output_scale == Some(2));
        let screencopy = probe.screencopy.clone().expect("zwlr_screencopy_manager_v1");
        let _frame = screencopy.capture_output(0, &probe.output, &probe.qh, ());
        probe.wait("the screencopy buffer", |seen| seen.screencopy_buffer.is_some());
        assert_eq!(
            probe.seen.screencopy_buffer,
            Some((2048, 1536)),
            "wlr-screencopy asks for a buffer of the composed frame"
        );

        let sources = probe
            .sources
            .clone()
            .expect("ext_output_image_capture_source_manager_v1");
        let image_copy = probe.image_copy.clone().expect("ext_image_copy_capture_manager_v1");
        let source = sources.create_source(&probe.output, &probe.qh, ());
        let _session = image_copy.create_session(
            &source,
            ext_image_copy_capture_manager_v1::Options::empty(),
            &probe.qh,
            (),
        );
        probe.wait("the image-copy buffer size", |seen| seen.image_copy_buffer.is_some());
        assert_eq!(
            probe.seen.image_copy_buffer,
            Some((2048, 1536)),
            "ext-image-copy-capture asks for a buffer of the composed frame"
        );
    });
}
