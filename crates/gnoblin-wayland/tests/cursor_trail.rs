//! A pointer that moves over an empty desktop leaves exactly one cursor image behind it.
//!
//! The owner's report was a black nested desktop with about ten copies of the arrow on it: each time the pointer
//! moved, its old picture stayed. Nothing but the cursor changes on an empty scene, so the damage of a frame is the
//! two cursor rectangles and a frame that is dropped between two others takes the erase of the cursor's old place
//! with it.
//!
//! The host here is the real [`Composer`] with the cursor path the session's `State` uses (`set_cursor` before the
//! damage is taken, the cursor overlay on the frame), so the damage that reaches a backend is the damage a session
//! produces. Every scenario reads each frame back and counts the pixels that differ from the background outside the
//! cursor's rectangle: more than none is a stale cursor.
//!
//! The nested scenarios hold the parent's frame callbacks, which is the only way to make "the parent is behind" a
//! fact of the test rather than a race: a session pointer moves faster than a parent answers frame callbacks.

use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::{Headless, HeadlessConfig, HeadlessHandle};
use gnoblin_backend::nested::{Nested, NestedConfig};
use gnoblin_backend::render::compose::Composer;
use gnoblin_backend::render::renderer::RendererKind;
use gnoblin_backend::{Backend, Host, InputEvent, Output, Present};
use gnoblin_render_frame::frame::{OutputFrame, Overlay};
use gnoblin_render_frame::image::Image;
use gnoblin_types::{OutputId, Point, Rect, Scale, Size};

mod nested_host;
use nested_host::{HEADLESS_SCREEN, Session, TIMEOUT, only_output, window_owner};

/// The cursor's edge in logical pixels: big enough to see, small enough that a trail is many copies.
const CURSOR: u32 = 12;

/// The white of the cursor on the black the session draws behind it.
const WHITE: [u8; 4] = [255, 255, 255, 255];

// ---- the host ---------------------------------------------------------------------------------------------------------

/// A session's compositor reduced to what a cursor needs: the composer, the pointer, and the overlay it is drawn as.
struct SceneHost {
    composer: Composer,
    /// Where the pointer is, in the output's logical space.
    pointer: Option<Point>,
    /// The output as the composer was last told of it, so a new size, scale or supersample factor is followed.
    known: Option<Output>,
}

impl SceneHost {
    fn new() -> Self {
        let mut composer = Composer::new();
        // The windows' fades are not the subject, and a fade would keep damaging after the scenario ends.
        composer.set_reduced_motion(true);
        Self {
            composer,
            pointer: None,
            known: None,
        }
    }

    /// Tells the composer the output when it is new or has changed in any way, and damages it whole, as a session
    /// does for an output it has just laid out: the renderer's buffer is new after a resize.
    fn follow(&mut self, output: &Output) {
        if self.known.as_ref() == Some(output) {
            return;
        }
        let size = output.logical_size();
        self.composer
            .set_output(output.id, Rect::new(0, 0, size.width, size.height));
        self.composer.damage_output(output.id);
        self.known = Some(output.clone());
    }

    /// Moves the pointer, as a device's motion does between two frames.
    fn move_to(&mut self, at: Option<Point>) {
        self.pointer = at;
    }

    /// Where the cursor is drawn, in the output's logical space, or nothing when the pointer is off the output.
    fn cursor(&self) -> Option<Rect> {
        let size = self.known.as_ref()?.logical_size();
        let at = self.pointer?;
        let inside = at.x >= 0 && at.y >= 0 && at.x < size.width as i32 && at.y < size.height as i32;
        inside.then(|| Rect::new(at.x, at.y, CURSOR, CURSOR))
    }
}

impl Host for SceneHost {
    fn begin_frame(&mut self) {
        // What `State::begin_frame_reading` does before the damage is taken: the cursor's old and new places.
        let cursor = self.cursor();
        self.composer.set_cursor(cursor, false);
        self.composer.begin_frame();
    }

    fn frame(&mut self, output: OutputId, frame: &mut OutputFrame) -> bool {
        let changed = self.composer.compose(output, frame);
        let overlay = self.cursor().and_then(|rect| {
            let pixels = WHITE.repeat((CURSOR * CURSOR) as usize);
            let image = Image::new(Size::new(CURSOR, CURSOR), CURSOR as usize * 4, pixels)?;
            Overlay::new(rect, &image)
        });
        frame.set_cursor_image(overlay);
        changed
    }

    fn input(&mut self, _event: InputEvent) {}
}

/// One iteration of the session's loop for every output: the backend's events, the host's damage, a frame per output.
fn iterate(backend: &mut dyn Backend<SceneHost>, scene: &mut SceneHost) -> Vec<Present> {
    let _ = backend.dispatch(scene);
    let outputs: Vec<Output> = backend.outputs().to_vec();
    for output in &outputs {
        scene.follow(output);
    }
    scene.begin_frame();
    let presents = outputs
        .iter()
        .map(|output| backend.render(scene, output.id).expect("a frame"))
        .collect();
    scene.composer.end_frame();
    presents
}

// ---- reading a frame --------------------------------------------------------------------------------------------------

/// What a frame holds besides the background.
#[derive(Debug, PartialEq, Eq)]
struct Marks {
    /// Pixels that are not the background and are outside the cursor's rectangle: a stale cursor.
    stale: usize,
    /// Pixels that are not the background and are inside it: the cursor itself.
    cursor: usize,
}

/// Counts the pixels of `image` that differ from `background`, split by whether they lie in `cursor`.
///
/// `scale` is physical pixels per logical pixel. The rectangle is grown by two pixels each way so the rounding of a
/// fractional scale and the averaging of a supersampled frame cannot count the cursor's own edge as stale.
fn marks(image: &Image, scale: f64, cursor: Option<Rect>, background: [u8; 4]) -> Marks {
    let size = image.size();
    let bounds = cursor.map(|rect| {
        let grown = |value: i32, extent: u32| {
            let low = (f64::from(value) * scale).floor() as i64 - 2;
            let high = (f64::from(value + extent as i32) * scale).ceil() as i64 + 2;
            (low, high)
        };
        (grown(rect.x, rect.width), grown(rect.y, rect.height))
    });
    let mut marks = Marks { stale: 0, cursor: 0 };
    for y in 0..size.height {
        for x in 0..size.width {
            let Some(pixel) = image.pixel(Point::new(x as i32, y as i32)) else {
                continue;
            };
            if pixel == background {
                continue;
            }
            let inside = bounds.is_some_and(|((left, right), (top, bottom))| {
                (left..right).contains(&i64::from(x)) && (top..bottom).contains(&i64::from(y))
            });
            if inside {
                marks.cursor += 1;
            } else {
                marks.stale += 1;
            }
        }
    }
    marks
}

/// Asserts that the frame the backend last drew for `output` holds the cursor at `scene`'s pointer and nowhere else.
#[track_caller]
fn assert_one_cursor(backend: &mut dyn Backend<SceneHost>, scene: &SceneHost, output: &Output, what: &str) {
    let image = backend
        .capture_display(output.id, true)
        .unwrap_or_else(|| panic!("{what}: the backend has no readback"));
    let shown = f64::from(image.size().width) / f64::from(output.logical_size().width);
    let cursor = scene.cursor();
    let found = marks(&image, shown, cursor, [0, 0, 0, 255]);
    assert_eq!(
        found.stale, 0,
        "{what}: {} pixels outside the cursor at {cursor:?} are not the background, which is a stale cursor",
        found.stale
    );
    if cursor.is_some() {
        assert!(found.cursor > 0, "{what}: the cursor at {cursor:?} is not drawn");
    }
}

/// The pointer positions a scenario walks through, as fractions of the output so every size and scale is covered.
/// They include a place at the corner where the cursor hangs over the edge, and a place off the output.
fn walk(logical: Size) -> Vec<Option<Point>> {
    let (w, h) = (logical.width as i32, logical.height as i32);
    vec![
        Some(Point::new(w / 8, h / 8)),
        Some(Point::new(w / 4, h / 3)),
        Some(Point::new(w / 2, h / 2)),
        Some(Point::new(w / 2 + 1, h / 2 + 1)),
        Some(Point::new(w - 5, h - 5)),
        None,
        Some(Point::new(0, 0)),
        Some(Point::new(3 * w / 4, h / 5)),
    ]
}

// ---- headless: every presented frame, every renderer ------------------------------------------------------------------

/// A headless backend that draws with `kind`, or `None` where the machine cannot create the renderer.
fn headless(kind: RendererKind) -> Option<(Headless, HeadlessHandle)> {
    let backend = Headless::with_config(HeadlessConfig {
        renderer: kind,
        ..HeadlessConfig::default()
    })
    .ok()?;
    let handle = backend.handle();
    Some((backend, handle))
}

/// Moves the pointer through [`walk`] on a headless output at `scale` and `supersample`, one frame per position, and
/// reads each frame back.
fn headless_walk(kind: RendererKind, scale: Scale, supersample: Scale) {
    let Some((mut backend, _handle)) = headless(kind) else {
        println!(
            "skipping the headless {} cursor walk: no such renderer here",
            kind.name()
        );
        return;
    };
    let mut scene = SceneHost::new();
    let output = Backend::<SceneHost>::outputs(&backend)[0].clone();
    Backend::<SceneHost>::configure_output(&mut backend, output.id, output.mode, scale, supersample)
        .expect("the output takes the scale and the supersample factor");
    let backend: &mut dyn Backend<SceneHost> = &mut backend;
    let output = backend.outputs()[0].clone();
    let _ = iterate(backend, &mut scene);
    assert_one_cursor(backend, &scene, &output, "the empty scene");
    for (step, position) in walk(output.logical_size()).into_iter().enumerate() {
        scene.move_to(position);
        let _ = iterate(backend, &mut scene);
        let what = format!(
            "{} at scale {scale:?}, supersample {supersample:?}, step {step} ({position:?})",
            kind.name()
        );
        assert_one_cursor(backend, &scene, &output, &what);
    }
}

#[test]
fn headless_software_leaves_one_cursor_at_scale_one() {
    headless_walk(RendererKind::Software, Scale::IDENTITY, Scale::IDENTITY);
}

#[test]
fn headless_software_leaves_one_cursor_at_scale_two_and_supersampled() {
    headless_walk(RendererKind::Software, Scale::integral(2), Scale::IDENTITY);
    headless_walk(RendererKind::Software, Scale::IDENTITY, Scale::integral(2));
}

#[test]
fn headless_software_leaves_one_cursor_at_a_fractional_scale() {
    headless_walk(RendererKind::Software, Scale::new(3, 2), Scale::IDENTITY);
}

#[test]
fn headless_gles_leaves_one_cursor() {
    headless_walk(RendererKind::Gles, Scale::IDENTITY, Scale::IDENTITY);
    headless_walk(RendererKind::Gles, Scale::integral(2), Scale::IDENTITY);
    headless_walk(RendererKind::Gles, Scale::IDENTITY, Scale::integral(2));
}

#[test]
fn headless_vulkan_leaves_one_cursor() {
    headless_walk(RendererKind::Vulkan, Scale::IDENTITY, Scale::IDENTITY);
    headless_walk(RendererKind::Vulkan, Scale::integral(2), Scale::IDENTITY);
}

// ---- nested: a parent that is behind ----------------------------------------------------------------------------------

/// Pumps the session's connection until the parent has answered the last frame.
fn until_answered(nested: &mut Nested, scene: &mut SceneHost, output: OutputId) {
    let deadline = Instant::now() + TIMEOUT;
    while nested.target_of(output).is_some_and(|target| target.awaiting_frame()) {
        assert!(Instant::now() < deadline, "the parent never answered the last frame");
        nested.roundtrip().expect("the parent answers a round trip");
        let _ = nested.dispatch(scene);
        thread::sleep(Duration::from_millis(2));
    }
}

/// Iterates until the output has presented `count` frames in all, answering the parent as it goes.
fn until_presented(nested: &mut Nested, scene: &mut SceneHost, output: OutputId, count: u64, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while nested.target_of(output).map_or(0, |target| target.presents()) < count {
        assert!(
            Instant::now() < deadline,
            "{what}: the session never presented frame {count}: it presented {} and the parent is {}",
            nested.target_of(output).map_or(0, |target| target.presents()),
            if nested.target_of(output).is_some_and(|target| target.awaiting_frame()) {
                "still owed an answer"
            } else {
                "not awaited, so the frame that would show the pointer's last place was never sent"
            }
        );
        nested.roundtrip().expect("the parent answers a round trip");
        let _ = iterate(nested, scene);
        thread::sleep(Duration::from_millis(2));
    }
}

/// Iterates until everything the session drew has been sent and the parent has answered it: nothing is owed either way.
fn settle(nested: &mut Nested, scene: &mut SceneHost, output: OutputId) {
    let deadline = Instant::now() + TIMEOUT;
    for _ in 0..3 {
        loop {
            nested.roundtrip().expect("the parent answers a round trip");
            let _ = iterate(nested, scene);
            let target = nested.target_of(output).expect("the output's target");
            if !target.awaiting_frame() && !target.unpresented() {
                break;
            }
            assert!(Instant::now() < deadline, "the session never settled");
            thread::sleep(Duration::from_millis(2));
        }
    }
}

/// Iterates the session until its only output satisfies `done`, so the host follows every change on the way.
fn follow_until(nested: &mut Nested, scene: &mut SceneHost, what: &str, done: impl Fn(&Output) -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !done(&only_output(nested)) {
        assert!(Instant::now() < deadline, "the session never followed {what}");
        nested.roundtrip().expect("the parent answers a round trip");
        let _ = iterate(nested, scene);
        thread::sleep(Duration::from_millis(5));
    }
}

/// The nested scenario: the parent stops answering frame callbacks, the pointer moves through [`walk`] while it is
/// behind, then the parent answers again. The frame the parent finally shows has the cursor at the last place and
/// nowhere else, both in the session's own readback and in the parent's screenshot of the window.
///
/// `host_scale` is the scale the host is told (`scale=2`) and the number it means. With `move_again` the pointer
/// moves once more as the parent catches up.
fn nested_walk_with_a_slow_parent(
    kind: RendererKind,
    host_scale: Option<(&'static str, f64)>,
    supersample: Scale,
    move_again: bool,
) {
    let backend = Headless::new();
    let hold = backend.handle();
    let session = Session::start_on(backend, |config| config);
    session.run(move |host| {
        let Some(mut nested) = nested_host::connect_drawing_with(host, kind) else {
            return;
        };
        let mut scene = SceneHost::new();
        // The first frame maps the session's window, which is then made small enough to be on the host's screen at
        // every scale the scenario asks for: a window larger than the screen has no pixels to look at.
        let first_output = only_output(&nested);
        until_presented(&mut nested, &mut scene, first_output.id, 1, "the first frame");
        // (A resize replaces the output's target, so counts are always taken relative to the moment of asking.)
        let window = host
            .wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
            .only()
            .expect("the session's window")
            .clone();
        host.control
            .move_window(window.id, Rect::new(window.geometry.x, window.geometry.y, 320, 240));
        follow_until(&mut nested, &mut scene, "the host's resize", |output| {
            output.logical_size() == Size::new(320, 240)
        });
        if let Some((text, _)) = host_scale {
            // The host's own scale follows through to the session's output.
            let before = only_output(&nested).scale;
            host.control
                .output_apply(window_owner(host), text)
                .expect("the host answers")
                .expect("the host applies the scale");
            follow_until(&mut nested, &mut scene, "the host's scale", |output| {
                output.scale != before
            });
        }
        if supersample != Scale::IDENTITY {
            let current = only_output(&nested);
            Backend::<SceneHost>::configure_output(&mut nested, current.id, current.mode, current.scale, supersample)
                .expect("the host window takes the supersample factor");
        }
        let output = only_output(&nested);

        // The picture of the empty scene, and the parent's answer to it: from here the session is idle and unpaced.
        settle(&mut nested, &mut scene, output.id);
        assert_one_cursor(&mut nested, &scene, &output, "the empty scene");
        let window = host
            .wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
            .only()
            .expect("the session's window")
            .clone();

        // The parent stops answering. The first move is presented (nothing is outstanding); the parent is then behind.
        hold.hold_presents(true);
        let steps = walk(output.logical_size());
        let (first, rest) = steps.split_first().expect("a walk");
        let before = nested.target_of(output.id).map_or(0, |target| target.presents());
        scene.move_to(*first);
        until_presented(&mut nested, &mut scene, output.id, before + 1, "the first move");
        assert!(
            nested
                .target_of(output.id)
                .is_some_and(|target| target.awaiting_frame()),
            "the frame that was presented asks the parent for the next one"
        );
        assert_one_cursor(&mut nested, &scene, &output, "the first move");

        // The pointer moves on while the parent is behind: none of these frames can be presented.
        let presented = nested.target_of(output.id).map_or(0, |target| target.presents());
        for position in rest {
            scene.move_to(*position);
            let presents = iterate(&mut nested, &mut scene);
            assert_eq!(presents, [Present::Nothing], "a parent that is behind is sent no frame");
            nested.roundtrip().expect("the parent answers a round trip");
        }
        assert_eq!(
            nested.target_of(output.id).map(|target| target.presents()),
            Some(presented),
            "nothing reached the parent while it was behind"
        );

        // The parent catches up, and answers the frame it was sent while it was held.
        hold.hold_presents(false);
        if move_again {
            // The pointer moves once more as the parent catches up, which is what the owner did: the frame it
            // produces must erase the cursor of every frame that was skipped, not only the last one's.
            until_answered(&mut nested, &mut scene, output.id);
            let size = output.logical_size();
            scene.move_to(Some(Point::new(size.width as i32 / 3, 2 * size.height as i32 / 3)));
        }
        // The session shows where the pointer is now, and nowhere else.
        until_presented(
            &mut nested,
            &mut scene,
            output.id,
            presented + 1,
            "after the parent caught up",
        );
        assert_one_cursor(
            &mut nested,
            &scene,
            &output,
            "the session's own frame after the parent caught up",
        );

        // What the parent shows is the frame the session sent: its screenshot of the window has the cursor at the
        // pointer and the background where the pointer was.
        let host_output = window_owner(host);
        // The host's screenshot is in whichever pixels it was taken in, which the picture's own width says: the
        // host's logical width is its mode's width divided by the scale it was given.
        let shot_width = host.control.screenshot(host_output).expect("a screenshot").size().width;
        let factor =
            f64::from(shot_width) / (f64::from(HEADLESS_SCREEN.width) / host_scale.map_or(1.0, |(_, scale)| scale));
        let pixel_of = |position: Point| {
            let centre = |origin: i32, offset: i32| {
                ((f64::from(origin) + f64::from(offset) + f64::from(CURSOR) / 2.0) * factor).round() as i32
            };
            Point::new(
                centre(window.geometry.x, position.x),
                centre(window.geometry.y, position.y),
            )
        };
        let last = scene.cursor().expect("the walk ends on the output");
        host.wait_for_colour(host_output, pixel_of(Point::new(last.x, last.y)), WHITE);
        let size = output.logical_size();
        let earlier: Vec<Point> = steps
            .iter()
            .flatten()
            .copied()
            .filter(|position| {
                position.x >= 0
                    && position.y >= 0
                    && (position.x as u32) < size.width
                    && (position.y as u32) < size.height
            })
            .filter(|position| (position.x, position.y) != (last.x, last.y))
            .collect();
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let shot = host.control.screenshot(host_output).expect("a screenshot");
            let stale: Vec<Point> = earlier
                .iter()
                .copied()
                .filter(|position| {
                    shot.pixel(pixel_of(*position))
                        .is_some_and(|pixel| pixel != [0, 0, 0, 255])
                })
                .collect();
            if stale.is_empty() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the parent's picture still shows the cursor where the pointer has been: {stale:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    });
}

#[test]
fn a_nested_cursor_leaves_no_trail_when_the_parent_is_slow() {
    nested_walk_with_a_slow_parent(RendererKind::Software, None, Scale::IDENTITY, false);
}

#[test]
fn a_nested_cursor_leaves_no_trail_when_the_parent_is_slow_at_scale_two() {
    nested_walk_with_a_slow_parent(RendererKind::Software, Some(("scale=2", 2.0)), Scale::IDENTITY, false);
}

#[test]
fn a_nested_cursor_leaves_no_trail_when_the_parent_is_slow_at_a_fractional_scale() {
    nested_walk_with_a_slow_parent(RendererKind::Software, Some(("scale=1.5", 1.5)), Scale::IDENTITY, false);
}

#[test]
fn a_nested_cursor_leaves_no_trail_when_the_parent_is_slow_and_the_session_is_supersampled() {
    nested_walk_with_a_slow_parent(RendererKind::Software, None, Scale::integral(2), false);
}

/// The owner's picture: ten arrows. The pointer moved while the parent was behind, then moved again, and the frame
/// that follows must hold one cursor and not the cursors of the frames the session skipped.
#[test]
fn a_nested_cursor_moved_after_skipped_frames_leaves_no_old_arrows() {
    nested_walk_with_a_slow_parent(RendererKind::Software, None, Scale::IDENTITY, true);
}

#[test]
fn a_nested_gles_cursor_leaves_no_trail_when_the_parent_is_slow() {
    nested_walk_with_a_slow_parent(RendererKind::Gles, None, Scale::IDENTITY, false);
}

#[test]
fn a_nested_vulkan_cursor_leaves_no_trail_when_the_parent_is_slow() {
    nested_walk_with_a_slow_parent(RendererKind::Vulkan, None, Scale::IDENTITY, false);
}

/// A cursor-only move is presented, and the present asks the parent for the next frame, so the cursor of a session
/// whose parent answers promptly keeps following the pointer rather than freezing on a frame.
#[test]
fn a_cursor_only_move_is_presented_and_asks_for_the_next_frame() {
    let session = Session::start();
    session.run(|host| {
        let mut nested = Nested::connect(&NestedConfig::new(host.socket())).expect("the session connects");
        let mut scene = SceneHost::new();
        let output = only_output(&nested);
        until_presented(&mut nested, &mut scene, output.id, 1, "the first frame");
        until_answered(&mut nested, &mut scene, output.id);
        let steps = walk(output.logical_size());
        for position in steps {
            let before = nested.target_of(output.id).map_or(0, |target| target.presents());
            scene.move_to(position);
            // The parent is prompt, so a move is presented as soon as the previous frame has been answered.
            until_presented(&mut nested, &mut scene, output.id, before + 1, "a prompt parent");
            assert!(
                nested
                    .target_of(output.id)
                    .is_some_and(|target| target.awaiting_frame()),
                "the present of a cursor-only move asks the parent for the next frame ({position:?})"
            );
            until_answered(&mut nested, &mut scene, output.id);
            assert_one_cursor(&mut nested, &scene, &output, "after a prompt parent's frame");
        }
    });
}

/// The owner's setup end to end: a real session compositor (its own `State`, window manager and cursor) on a nested
/// backend in a window of a host, its pointer driven in bursts with no pause for the host. When the pointer rests the
/// host's picture of the window shows one cursor, and every pixel that is not black lies near the pointer.
#[test]
fn a_real_nested_session_leaves_one_cursor_under_a_busy_pointer() {
    use std::sync::mpsc;

    use gnoblin_wayland::{Config, Server};

    let host_session = Session::start();
    host_session.run(|host| {
        let host_socket = host.socket().to_path_buf();
        let (up, ready) = mpsc::channel();
        thread::spawn(move || {
            let directory = tempfile::tempdir().expect("a temporary directory");
            let nested = Nested::connect(&NestedConfig::new(&host_socket)).expect("the session connects to the host");
            let config = Config {
                socket_path: Some(directory.path().join("session-0.sock")),
                ..Config::default()
            };
            let server = Server::new(Box::new(nested), config).expect("the session starts");
            let control = server.control();
            let observation = server.observation();
            up.send((control.clone(), observation)).expect("the test is waiting");
            let _ = server.run();
            drop(directory);
        });
        let (control, observation) = ready.recv_timeout(TIMEOUT).expect("the session came up");
        struct StopSession(gnoblin_wayland::ControlHandle);
        impl Drop for StopSession {
            fn drop(&mut self) {
                self.0.stop();
            }
        }
        let _stop = StopSession(control.clone());

        let window = host
            .wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
            .only()
            .expect("the session's window")
            .clone();
        let deadline = Instant::now() + TIMEOUT;
        let output = loop {
            if let Some(output) = observation
                .lock()
                .expect("the observation lock")
                .outputs()
                .first()
                .copied()
            {
                break output;
            }
            assert!(Instant::now() < deadline, "the session never published an output");
            thread::sleep(Duration::from_millis(5));
        };
        let size = Size::new(window.geometry.width, window.geometry.height);
        let motion = |at: Point| {
            control.inject_input(InputEvent::PointerMotion {
                device: gnoblin_types::DeviceId::VIRTUAL,
                output,
                position: at,
            });
        };

        // Many small moves, each followed by the least pause there is, so the session renders faster than the host
        // answers its frame callbacks for part of the run.
        let mut at = Point::new(10, 10);
        for burst in 0..40_i32 {
            for step in 0..25_i32 {
                at = Point::new(
                    10 + (burst * 61 + step * 9) % (size.width as i32 - 80),
                    10 + (burst * 47 + step * 7) % (size.height as i32 - 80),
                );
                motion(at);
                thread::sleep(Duration::from_micros(300));
            }
        }

        let host_output = window_owner(host);
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let shot = host.control.screenshot(host_output).expect("a screenshot");
            let (mut near, mut far) = (0_usize, 0_usize);
            for y in 0..shot.size().height as i32 {
                for x in 0..shot.size().width as i32 {
                    if shot
                        .pixel(Point::new(x, y))
                        .is_some_and(|pixel| pixel[..3] != [0, 0, 0])
                    {
                        let (dx, dy) = (x - (window.geometry.x + at.x), y - (window.geometry.y + at.y));
                        if dx.abs() <= 48 && dy.abs() <= 48 {
                            near += 1;
                        } else {
                            far += 1;
                        }
                    }
                }
            }
            if far == 0 && near > 0 {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the host's picture of the session has {far} pixels away from the pointer at {at:?} ({near} near it)"
            );
            thread::sleep(Duration::from_millis(20));
        }
    });
}
