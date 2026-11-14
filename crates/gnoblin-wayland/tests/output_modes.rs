//! Output modes on the real compositor: `screen.apply` through the transactional engine, a mirror that shows the exact
//! picture of another output, one desktop across several outputs (the span), and the profiles that bring an
//! arrangement back after an unplug.
//!
//! Everything here runs the real `State`, the real frame loop and the software renderer on virtual (headless) outputs,
//! and reads pixels back through the backend. A stand-in shell sends the `apply` calls over the tree and places
//! windows. What it does not prove: a DRM modeset on hardware, or the GLES renderer's readback of a mirror.

mod session;

use std::time::{Duration, Instant};

use gnoblin_backend::InputEvent;
use gnoblin_backend::headless::{DEFAULT_CONNECTOR, DEFAULT_MODE, HeadlessHandle};
use gnoblin_output::DisplayMode;
use gnoblin_render::transaction::TreeValue;
use gnoblin_render_frame::image::Image;
use gnoblin_types::{DeviceId, NodeId, OutputId, Point, Rect, Scale};
use gnoblin_wayland::TreeInput;
use session::shell::Shell;
use session::{Client, Handle, Session};

const RED_BGRA: [u8; 4] = [0, 0, 255, 255];
const RED: [u8; 4] = [255, 0, 0, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];
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
            .find(|node| shell.published(*node, "connector") == Some(TreeValue::Text(connector.to_owned())))
    })
}

/// Waits until the session's rectangle for `output` is `rect`.
#[track_caller]
fn wait_rect(handle: &Handle, output: OutputId, rect: Rect) {
    until(&format!("{output:?} at {rect:?}"), || {
        (handle.windows().output_rect(output) == Some(rect)).then_some(())
    });
}

/// The backend's picture of an output, once `accept` takes it.
#[track_caller]
fn picture(handle: &Handle, output: OutputId, accept: impl Fn(&Image) -> bool) -> Image {
    until("the expected picture", || {
        handle.control.screenshot(output).filter(|image| accept(image))
    })
}

fn pixel(image: &Image, x: i32, y: i32) -> [u8; 4] {
    image.pixel(Point::new(x, y)).expect("a pixel inside the picture")
}

/// Puts a window node at `(x, y)` with the size its buffer has, as the shell's placement does.
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

fn plug(outputs: &HeadlessHandle, connector: &str, width: u32, height: u32, scale: Scale) {
    outputs
        .connect_output(connector, DisplayMode::new(width, height, 60_000), scale)
        .expect("a mode that covers pixels");
}

#[test]
fn apply_changes_mode_scale_and_position_and_a_refused_modeset_keeps_the_last_good_arrangement() {
    let (session, outputs, shell) = start();
    session.run(move |handle| {
        shell.attach();
        plug(&outputs, "Virtual-2", 640, 480, Scale::integral(1));
        let second = id_of(handle, "Virtual-2");
        let node = node_of(&shell, "Virtual-2");
        let first_width = DEFAULT_MODE.width;
        wait_rect(handle, second, Rect::new(first_width as i32, 0, 640, 480));

        // A mode and a position, in one request: the output is driven at the new mode and moved.
        shell
            .apply(node, "mode=800x600@60\nposition=0,768")
            .expect("the change applies");
        wait_rect(handle, second, Rect::new(0, 768, 800, 600));
        let shot = picture(handle, second, |image| image.size().width == 800);
        assert_eq!(shot.size().height, 600, "the backend drives the new mode");

        // A modeset the driver refuses: the answer says so under the key, and nothing moved.
        outputs.refuse_configure_after(0);
        let refused = shell
            .apply(node, "mode=1280x720@60")
            .expect_err("the backend refuses the mode");
        assert!(refused.starts_with("mode: "), "the refusal names the key: {refused}");
        assert_eq!(handle.windows().output_rect(second), Some(Rect::new(0, 768, 800, 600)));
        assert_eq!(handle.control.screenshot(second).expect("a picture").size().width, 800);

        // The refused mode never became the profile: the next change keeps the last good one, so no modeset is tried
        // for it (a stored 1280x720 would be driven now).
        shell
            .apply(node, "enabled=true")
            .expect("a request that changes nothing applies");
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(handle.windows().output_rect(second), Some(Rect::new(0, 768, 800, 600)));

        // A layout the engine refuses is refused by name, before any hardware is touched.
        let overlap = shell
            .apply(node, "position=0,0")
            .expect_err("the output would overlap the first");
        assert!(
            overlap.starts_with("position: ") && overlap.contains("overlap"),
            "{overlap}"
        );
        assert_eq!(handle.windows().output_rect(second), Some(Rect::new(0, 768, 800, 600)));

        // A scale the output is then composed at, and the node says so.
        shell.apply(node, "scale=1.5").expect("a scale applies");
        wait_rect(handle, second, Rect::new(0, 768, 533, 400));
        shell.wait_published(node, "output_scale", &TreeValue::Double(1.5));
    });
}

#[test]
fn a_request_with_a_bad_value_or_an_unsupported_transform_changes_nothing() {
    let (session, _outputs, shell) = start();
    session.run(move |handle| {
        shell.attach();
        let first = id_of(handle, DEFAULT_CONNECTOR);
        let node = node_of(&shell, DEFAULT_CONNECTOR);
        let rect = handle.windows().output_rect(first);
        let transform = shell
            .apply(node, "transform=90")
            .expect_err("the compositor draws only normal");
        assert!(transform.starts_with("transform: "), "{transform}");
        let mode = shell.apply(node, "mode=banana").expect_err("not a mode");
        assert!(mode.starts_with("mode: "), "{mode}");
        let only = shell
            .apply(node, "enabled=false")
            .expect_err("the last output cannot go off");
        assert!(only.starts_with("enabled: "), "{only}");
        let itself = shell
            .apply(node, "mirror_of=HEADLESS-1")
            .expect_err("an output cannot mirror itself");
        assert!(itself.starts_with("mirror_of: "), "{itself}");
        let missing = shell.apply(node, "mirror_of=DP-9").expect_err("no such output");
        assert!(missing.starts_with("mirror_of: "), "{missing}");
        assert_eq!(handle.windows().output_rect(first), rect, "nothing changed");
    });
}

/// Maps a red 200x150 window and places it at `at`.
fn red_window(handle: &Handle, shell: &Shell, at: (f64, f64)) -> (Client, NodeId) {
    let mut client = Client::connect(handle.socket());
    let _window = client.map_solid_window("org.example.red", "Red", (200, 150), RED_BGRA);
    let node = shell.wait_window_nodes(1)[0];
    place(shell, node, at, (200.0, 150.0));
    (client, node)
}

#[test]
fn a_mirror_of_equal_mode_shows_the_same_pixels_and_is_one_monitor_for_the_pointer() {
    let (session, outputs, shell) = start();
    session.run(move |handle| {
        shell.attach();
        plug(
            &outputs,
            "Virtual-2",
            DEFAULT_MODE.width,
            DEFAULT_MODE.height,
            Scale::integral(1),
        );
        let first = id_of(handle, DEFAULT_CONNECTOR);
        let second = id_of(handle, "Virtual-2");
        let node = node_of(&shell, "Virtual-2");
        let (_client, _window) = red_window(handle, &shell, (100.0, 100.0));
        let source = picture(handle, first, |image| pixel(image, 150, 150) == RED);
        assert_eq!(
            pixel(&picture(handle, second, |_| true), 150, 150)[..3],
            BLACK[..3],
            "before the mirror, the second output shows its own empty desktop"
        );
        assert_eq!(handle.windows().outputs().len(), 2);

        shell.apply(node, "mirror_of=HEADLESS-1").expect("the mirror applies");

        // The pair is one monitor: one rectangle, and the mirrored output has none of its own.
        until("one logical monitor", || {
            (handle.windows().outputs().len() == 1).then_some(())
        });
        assert_eq!(handle.windows().output_rect(second), None);
        assert_eq!(
            handle.windows().output_rect(first),
            Some(Rect::new(0, 0, DEFAULT_MODE.width, DEFAULT_MODE.height))
        );
        shell.wait_published(node, "mirror_of", &TreeValue::Text(DEFAULT_CONNECTOR.to_owned()));

        // Exact pixels: every byte of the mirror's picture is the source's.
        let mirrored = picture(handle, second, |image| pixel(image, 150, 150) == RED);
        assert_eq!(mirrored.size(), source.size());
        // The mirror shows what the source's screen shows, pointer included, and the screenshot above leaves the pointer
        // out: the only pixels that may differ are the pointer's, in the corner it rests in before it has moved.
        let width = source.size().width as usize;
        let differing: Vec<(usize, usize)> = source
            .data()
            .chunks(4)
            .zip(mirrored.data().chunks(4))
            .enumerate()
            .filter(|(_, (left, right))| left != right)
            .map(|(index, _)| (index % width, index / width))
            .collect();
        assert!(
            differing.iter().all(|(x, y)| *x < 24 && *y < 32),
            "an equal mode mirrors pixel for pixel apart from the pointer; {} differ, e.g. {:?}",
            differing.len(),
            differing.last()
        );

        // And it follows: the window moves, the mirror moves with it.
        place(&shell, shell.wait_window_nodes(1)[0], (300.0, 200.0), (200.0, 150.0));
        picture(handle, second, |image| {
            pixel(image, 350, 250) == RED && pixel(image, 150, 150)[..3] == BLACK[..3]
        });

        // Stopping the mirror gives the output its own place again, to the right of the first.
        shell.apply(node, "mirror_of=").expect("the mirror ends");
        wait_rect(
            handle,
            second,
            Rect::new(DEFAULT_MODE.width as i32, 0, DEFAULT_MODE.width, DEFAULT_MODE.height),
        );
        assert_eq!(handle.windows().outputs().len(), 2);
    });
}

#[test]
fn a_mirror_on_a_wider_panel_is_scaled_to_fit_and_letterboxed() {
    let (session, outputs, shell) = start();
    session.run(move |handle| {
        shell.attach();
        // 800x400 is 2:1; the first output is 1024x768, 4:3. The height is the limit: 400 / 768 of the picture, centred.
        plug(&outputs, "Virtual-2", 800, 400, Scale::integral(1));
        let first = id_of(handle, DEFAULT_CONNECTOR);
        let second = id_of(handle, "Virtual-2");
        let node = node_of(&shell, "Virtual-2");
        let (_client, _window) = red_window(handle, &shell, (100.0, 100.0));
        picture(handle, first, |image| pixel(image, 150, 150) == RED);

        shell.apply(node, "mirror_of=HEADLESS-1").expect("the mirror applies");

        // The window (100..300, 100..250 on the source) lands at x = 133 + 100 * 400 / 768 = 185 to 289 and
        // y = 52 to 130. A copy that did not scale would put red at (230, 200) and none at (230, 90).
        let shot = picture(handle, second, |image| pixel(image, 230, 90) == RED);
        assert_eq!(shot.size().width, 800);
        assert_eq!(shot.size().height, 400);
        assert_eq!(
            pixel(&shot, 230, 200)[..3],
            BLACK[..3],
            "the unscaled place of the window is empty"
        );
        assert_eq!(pixel(&shot, 20, 90)[..3], BLACK[..3], "the left bar is black");
        assert_eq!(pixel(&shot, 790, 90)[..3], BLACK[..3], "the right bar is black");
        assert_eq!(
            pixel(&shot, 184, 90)[..3],
            BLACK[..3],
            "red starts at the scaled edge, not before"
        );
        assert_eq!(pixel(&shot, 186, 90), RED);
    });
}

#[test]
fn an_output_that_is_off_shows_black_and_has_no_place_in_the_desktop() {
    let (session, outputs, shell) = start();
    session.run(move |handle| {
        shell.attach();
        plug(&outputs, "Virtual-2", 640, 480, Scale::integral(1));
        let second = id_of(handle, "Virtual-2");
        let node = node_of(&shell, "Virtual-2");
        let (_client, window) = red_window(handle, &shell, (1100.0, 100.0));
        picture(handle, second, |image| pixel(image, 100, 150) == RED);
        place(&shell, window, (100.0, 100.0), (200.0, 150.0));

        shell.apply(node, "enabled=false").expect("the output goes off");
        until("one monitor", || (handle.windows().outputs().len() == 1).then_some(()));
        shell.wait_published(node, "enabled", &TreeValue::Bool(false));
        let shot = picture(handle, second, |image| {
            image.data().chunks(4).all(|pixel| pixel[..3] == BLACK[..3])
        });
        assert_eq!(shot.size().width, 640, "the output keeps its mode while it is off");
    });
}

#[test]
fn a_window_across_the_seam_is_continuous_and_the_pointer_crosses_it_at_mixed_scales() {
    let (session, outputs, shell) = start();
    session.run(move |handle| {
        shell.attach();
        // The second output is 1536x1152 at 1.5: the same 1024x768 logical, so the desktop is 2048 wide.
        plug(&outputs, "Virtual-2", 1536, 1152, Scale::from_percent(150));
        let first = id_of(handle, DEFAULT_CONNECTOR);
        let second = id_of(handle, "Virtual-2");
        wait_rect(handle, second, Rect::new(DEFAULT_MODE.width as i32, 0, 1024, 768));
        let mut client = Client::connect(handle.socket());
        let _window = client.map_solid_window("org.example.red", "Red", (200, 150), RED_BGRA);
        let node = shell.wait_window_nodes(1)[0];
        // 100 pixels on the first output, 100 on the second.
        place(&shell, node, (924.0, 100.0), (200.0, 150.0));

        let left = picture(handle, first, |image| pixel(image, 1023, 150) == RED);
        assert_eq!(
            pixel(&left, 923, 150)[..3],
            BLACK[..3],
            "the window starts at its own left edge"
        );
        // The second output composes at 1.5: 100 logical pixels of the window are 150 physical ones.
        let right = picture(handle, second, |image| pixel(image, 0, 225) == RED);
        assert_eq!(
            pixel(&right, 0, 225),
            RED,
            "the seam has no gap: the window continues on the next output"
        );
        assert_eq!(pixel(&right, 146, 225), RED);
        assert_eq!(
            pixel(&right, 154, 225)[..3],
            BLACK[..3],
            "and stops where the window does"
        );
        assert_eq!(pixel(&right, 0, 149)[..3], BLACK[..3]);

        // The pointer is in one space: moving from the first output to the second is one motion over one window.
        let seat = client.seat_devices();
        assert!(seat.1.is_some(), "the seat offers a pointer");
        handle.control.inject_input(InputEvent::PointerMotion {
            device: DeviceId::VIRTUAL,
            output: first,
            position: Point::new(1000, 150),
        });
        shell.wait_input(node, "enter", 1);
        handle.control.inject_input(InputEvent::PointerMotion {
            device: DeviceId::VIRTUAL,
            output: second,
            position: Point::new(26, 150),
        });
        handle.control.inject_input(InputEvent::PointerMotionRelative {
            device: DeviceId::VIRTUAL,
            dx: 40,
            dy: 0,
        });
        shell.wait_input(node, "motion", 2);
        let motions: Vec<(f64, f64)> = shell
            .inputs(node)
            .into_iter()
            .filter_map(|input| match input {
                TreeInput::Motion { x, y } => Some((x, y)),
                _ => None,
            })
            .collect();
        // Window-local x is the global x less 924: 1000 on the first output, 1050 and 1090 on the second.
        assert!(motions.contains(&(126.0, 50.0)), "{motions:?}");
        assert!(motions.contains(&(166.0, 50.0)), "{motions:?}");
    });
}

#[test]
fn a_profile_returns_when_its_monitor_does_and_not_for_another_on_the_same_port() {
    let (session, outputs, shell) = start();
    session.run(move |handle| {
        shell.attach();
        let first_width = DEFAULT_MODE.width as i32;
        let mode = DisplayMode::new(800, 600, 60_000);
        outputs
            .connect_identified_output("Virtual-2", mode, Scale::integral(1), "DEL:a0b1:42")
            .expect("a mode that covers pixels");
        let second = id_of(handle, "Virtual-2");
        let node = node_of(&shell, "Virtual-2");
        shell
            .apply(node, "mode=640x480@60\nposition=0,768")
            .expect("the profile applies");
        wait_rect(handle, second, Rect::new(0, 768, 640, 480));

        // Unplugged: the desktop is the first output alone, and the profile is parked.
        outputs.disconnect_output(second);
        until("the first output alone", || {
            (handle.windows().outputs().len() == 1).then_some(())
        });

        // The same monitor again, at the mode it connects with: it gets its mode and its place back.
        outputs
            .connect_identified_output("Virtual-2", mode, Scale::integral(1), "DEL:a0b1:42")
            .expect("a mode that covers pixels");
        let back = id_of(handle, "Virtual-2");
        wait_rect(handle, back, Rect::new(0, 768, 640, 480));
        let shot = picture(handle, back, |image| image.size().width == 640);
        assert_eq!(
            shot.size().height,
            480,
            "the backend was driven back to the stored mode"
        );

        // A different monitor on the same connector starts from the defaults.
        outputs.disconnect_output(back);
        until("the first output alone", || {
            (handle.windows().outputs().len() == 1).then_some(())
        });
        outputs
            .connect_identified_output("Virtual-2", mode, Scale::integral(1), "SAM:0001:7")
            .expect("a mode that covers pixels");
        let stranger = id_of(handle, "Virtual-2");
        wait_rect(handle, stranger, Rect::new(first_width, 0, 800, 600));
    });
}
