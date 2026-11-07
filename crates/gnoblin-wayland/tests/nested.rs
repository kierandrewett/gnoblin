//! The nested round trip: a session as a client of a session.
//!
//! These tests put the two halves of "Hosting sessions" (design/rust-compositor.md,
//! D4) on one socket: a real `gnoblin-wayland` [`Server`] playing the host with a
//! headless output, and the real [`Nested`] backend connecting to it as a hosted
//! session would. Nothing is simulated on either side of the wire — the host sends the
//! configure, the nested backend draws, commits a `wl_shm` buffer and the host
//! composes it — so what the assertions read from the host is what a person would see.
//!
//! ## What each side is
//!
//! - The **host** is the server's own loop, its window manager and its software
//!   renderer. It is asked about the world through its published snapshot and its
//!   screenshots, never from inside the loop.
//! - The **session** is a [`Nested`] backend, driven by a [`Probe`]: a `Host` that
//!   records the input it is handed and describes a frame of one colour, standing in
//!   for the session's own compositor.
//! - The **seam** is `Backend`: the session's frames, resizes and input are read
//!   through the same trait the headless backend implements, because a session must not
//!   be able to tell which backend its compositor runs on.
//!
//! ## Two threads, on purpose
//!
//! The host's calloop loop is not `Send`, so it runs on the test's thread while the
//! session runs on a worker — which is also the real arrangement: the host and the
//! session are different processes. The worker reaches the host only through the
//! control handle, exactly as `tests/real_client.rs` does.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::Headless;
use gnoblin_backend::nested::{Nested, NestedConfig};
use gnoblin_render_frame::frame::OutputFrame;
use gnoblin_backend::{Backend, BackendEvent, Host, InputEvent, Output, Present};
use gnoblin_types::{Color, OutputId, Point, Rect, Size};
use gnoblin_wayland::{Config, ControlHandle, LoopError, ObservedWindow, Server, SessionSnapshot};

/// How long a test waits for a state before failing.
const TIMEOUT: Duration = Duration::from_secs(20);

/// The red the session draws, which is nothing like the host's black background.
const RED: Color = Color::new(1.0, 0.0, 0.0, 1.0);

/// [`RED`] as a screenshot states it.
const RED_PIXEL: [u8; 4] = [255, 0, 0, 255];

/// The host's background as a screenshot states it.
const BLACK_PIXEL: [u8; 4] = [0, 0, 0, 255];

/// The host's one output, which is the headless backend's default mode.
const HEADLESS_SCREEN: Size = Size::new(
    gnoblin_backend::headless::DEFAULT_MODE.width,
    gnoblin_backend::headless::DEFAULT_MODE.height,
);

/// `BTN_LEFT` from `linux/input-event-codes.h`.
const BTN_LEFT: u32 = 0x110;

/// What the session's own compositor would do: describe a frame and keep the input.
#[derive(Default)]
struct Probe {
    inputs: Vec<InputEvent>,
    colour: Option<Color>,
}

impl Probe {
    /// A session that draws `colour`.
    fn drawing(colour: Color) -> Self {
        Self {
            inputs: Vec::new(),
            colour: Some(colour),
        }
    }

    /// Every pointer motion the session was handed, in arrival order.
    ///
    /// There is more than one: the host sends an enter as soon as the pointer is over the
    /// session's surface, which is before this test moves it.
    fn motions(&self) -> Vec<(OutputId, Point)> {
        self.inputs
            .iter()
            .filter_map(|event| match event {
                InputEvent::PointerMotion { output, position } => Some((*output, *position)),
                _ => None,
            })
            .collect()
    }
}

impl Host for Probe {
    fn begin_frame(&mut self) {}

    fn frame(&mut self, _output: OutputId, frame: &mut OutputFrame) -> bool {
        frame.clear();
        if let Some(colour) = self.colour {
            frame.set_background(colour);
        }
        // The renderer clips this to the output, so the probe does not have to know how
        // big the session's output is.
        frame.push_damage(Rect::new(0, 0, 4096, 4096));
        true
    }

    fn input(&mut self, event: InputEvent) {
        self.inputs.push(event);
    }
}

/// What a worker can reach of a running host.
#[derive(Clone)]
struct Handle {
    socket: PathBuf,
    control: ControlHandle,
    observation: Arc<Mutex<SessionSnapshot>>,
}

impl Handle {
    fn socket(&self) -> &Path {
        &self.socket
    }

    /// The host's window state right now.
    fn windows(&self) -> SessionSnapshot {
        self.observation.lock().expect("the observation lock").clone()
    }

    /// Waits for the host to satisfy `predicate`, or fails with the last state.
    fn wait_for(&self, predicate: impl Fn(&SessionSnapshot) -> bool) -> SessionSnapshot {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let observation = self.windows();
            if predicate(&observation) {
                return observation;
            }
            assert!(
                Instant::now() < deadline,
                "the host never reached the expected state; last state: {observation:#?}"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Waits for the host's screenshot to show `colour` at `point`.
    ///
    /// The host draws on its own iteration, so "the session committed it" and "the host
    /// composed it" are two different moments.
    fn wait_for_colour(&self, output: OutputId, point: Point, colour: [u8; 4]) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let seen = self.control.screenshot(output).and_then(|image| image.pixel(point));
            assert!(
                Instant::now() < deadline,
                "the host never drew {colour:?} at {point:?}; last pixel: {seen:?}"
            );
            if seen == Some(colour) {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
}

/// Stops the host when the worker finishes, including when it panics.
struct StopOnDrop(ControlHandle);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// A host that has been created but is not running yet.
struct Session {
    /// Kept alive for the session's lifetime: dropping it would unlink the socket.
    #[allow(dead_code)]
    directory: tempfile::TempDir,
    server: Server,
    handle: Handle,
}

impl Session {
    /// Creates a host on a socket of its own.
    fn start() -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let socket = directory.path().join("wayland-0.sock");
        let config = Config {
            socket_path: Some(socket.clone()),
            ..Config::default()
        };
        let server = Server::new(Box::new(Headless::new()), config).expect("the host starts");
        let handle = Handle {
            socket,
            control: server.control(),
            observation: server.observation(),
        };
        Self {
            directory,
            server,
            handle,
        }
    }

    /// Runs the host on this thread and `body` on a worker, then joins both.
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
            // A failed assertion in the worker is the test's failure.
            std::panic::resume_unwind(payload);
        }
        outcome.expect("the loop stops cleanly");
    }
}

/// The session's outputs, as the contract hands them out.
///
/// The trait is generic over the compositor it drives and this file's is [`Probe`], so
/// the call names it rather than leaving the inference to guess.
fn session_outputs(nested: &Nested) -> &[Output] {
    <Nested as Backend<Probe>>::outputs(nested)
}

/// The session's last frame, read back through the contract.
fn session_screenshot(nested: &mut Nested, output: OutputId) -> Option<gnoblin_render_frame::image::Image> {
    <Nested as Backend<Probe>>::screenshot(nested, output)
}

/// The session's only output.
fn only_output(nested: &Nested) -> Output {
    session_outputs(nested)
        .first()
        .cloned()
        .expect("the session has one output once the host configured it")
}

/// Renders and presents one frame on every output, then waits for the host to answer.
///
/// Waiting is the point: the backend presents only when the host asked for a frame, so a
/// test that rendered twice without waiting would be testing the pacing, not the frame.
fn draw(nested: &mut Nested, probe: &mut Probe) {
    let outputs: Vec<OutputId> = session_outputs(nested).iter().map(|output| output.id).collect();
    for output in outputs {
        let _ = nested.render(probe, output);
    }
    nested.roundtrip().expect("the host answers a round trip");
}

/// Waits until `predicate` holds for the probe, pumping the session as it goes.
fn pump_until(nested: &mut Nested, probe: &mut Probe, predicate: impl Fn(&Probe) -> bool) -> Vec<BackendEvent> {
    let deadline = Instant::now() + TIMEOUT;
    let mut events = Vec::new();
    loop {
        nested.roundtrip().expect("the host answers a round trip");
        events.extend(nested.dispatch(probe));
        if predicate(probe) {
            return events;
        }
        assert!(
            Instant::now() < deadline,
            "the session never reached the expected state; input so far: {:?}",
            probe.inputs
        );
        thread::sleep(Duration::from_millis(5));
    }
}

/// A point on the host's output that the window does not cover.
///
/// The host's window manager places the window and this test does not assume where: it
/// looks for a place beside or below it and fails loudly if the window covers everything,
/// because "the session's pixels are where its window is" is only testable somewhere the
/// session is not.
fn outside(window: &ObservedWindow, screen: Size) -> Point {
    let right = window.geometry.x + i32::try_from(window.geometry.width).unwrap_or(0);
    let bottom = window.geometry.y + i32::try_from(window.geometry.height).unwrap_or(0);
    let screen_width = i32::try_from(screen.width).unwrap_or(i32::MAX);
    let screen_height = i32::try_from(screen.height).unwrap_or(i32::MAX);
    let middle_y = window.geometry.y + i32::try_from(window.geometry.height / 2).unwrap_or(0);
    let middle_x = window.geometry.x + i32::try_from(window.geometry.width / 2).unwrap_or(0);
    if right + 2 < screen_width {
        Point::new(right + 2, middle_y)
    } else if bottom + 2 < screen_height {
        Point::new(middle_x, bottom + 2)
    } else {
        panic!("the window covers the whole host output, so there is nowhere it is not");
    }
}

/// The centre of a window, in the host output's logical space.
fn centre(window: &ObservedWindow) -> Point {
    Point::new(
        window.geometry.x + i32::try_from(window.geometry.width / 2).unwrap_or(0),
        window.geometry.y + i32::try_from(window.geometry.height / 2).unwrap_or(0),
    )
}

/// A window's extent as the size type the contract states geometry in.
fn extent(window: &ObservedWindow) -> Size {
    Size::new(window.geometry.width, window.geometry.height)
}

#[test]
fn a_nested_output_appears_as_a_mapped_toplevel_with_the_hosts_size() {
    let session = Session::start();
    session.run(|host| {
        let mut nested = Nested::connect(&NestedConfig::new(host.socket())).expect("the session connects");
        let mut probe = Probe::drawing(RED);
        let output = only_output(&nested);
        let connected = nested.dispatch(&mut probe);
        assert!(
            matches!(connected.as_slice(), [BackendEvent::OutputConnected(connected)] if connected.id == output.id),
            "the configure the connect waited for is an output appearing: {connected:?}"
        );

        draw(&mut nested, &mut probe);
        let observation = host.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        let window = observation.only().expect("one mapped window");
        assert_eq!(
            extent(window),
            output.logical_size(),
            "the host's configure is the size the session draws at"
        );
        assert_eq!(&observation.outputs(), &[window_owner(host)], "one host output");
    });
}

/// The host's only output id.
fn window_owner(host: &Handle) -> OutputId {
    host.windows().outputs().first().copied().expect("a host output")
}

#[test]
fn a_host_resize_reconfigures_the_nested_output() {
    let session = Session::start();
    session.run(|host| {
        let mut nested = Nested::connect(&NestedConfig::new(host.socket())).expect("the session connects");
        let mut probe = Probe::drawing(RED);
        draw(&mut nested, &mut probe);
        let observation = host.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        let window = observation.only().expect("one mapped window").clone();
        let _ = nested.dispatch(&mut probe);

        let smaller = Rect::new(window.geometry.x, window.geometry.y, 640, 480);
        host.control.move_window(window.id, smaller);
        let deadline = Instant::now() + TIMEOUT;
        let mut events = Vec::new();
        while only_output(&nested).logical_size() != Size::new(640, 480) {
            assert!(
                Instant::now() < deadline,
                "the session never followed the host's resize; it is still {:?}",
                only_output(&nested).logical_size()
            );
            nested.roundtrip().expect("the host answers a round trip");
            events.extend(nested.dispatch(&mut probe));
        }
        assert!(
            events.iter().any(|event| matches!(
                event,
                BackendEvent::OutputResized { mode, .. }
                    if mode.width == 640 && mode.height == 480
            )),
            "the resize is reported as one output resize: {events:?}"
        );
        assert_eq!(
            extent(
                host.wait_for(|o| o.only().is_some_and(|w| w.geometry.width == 640))
                    .only()
                    .expect("the window")
            ),
            Size::new(640, 480),
            "the session's buffer followed the configure, so the host's window is the new size"
        );
    });
}

#[test]
fn a_frame_drawn_by_the_nested_compositor_appears_in_the_hosts_screenshot() {
    let session = Session::start();
    session.run(|host| {
        let mut nested = Nested::connect(&NestedConfig::new(host.socket())).expect("the session connects");
        let mut probe = Probe::drawing(RED);
        let output = only_output(&nested);
        draw(&mut nested, &mut probe);

        let observation = host.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        let window = observation.only().expect("one mapped window").clone();
        let host_output = window_owner(host);
        let elsewhere = outside(&window, HEADLESS_SCREEN);

        // The session's red is inside its window, and the host's own black is where the
        // session is not — which is what makes "at the right place" an assertion rather
        // than "somewhere on the screen".
        host.wait_for_colour(host_output, centre(&window), RED_PIXEL);
        let screenshot = host.control.screenshot(host_output).expect("a screenshot");
        assert_eq!(
            screenshot.pixel(elsewhere),
            Some(BLACK_PIXEL),
            "the host's own background is at {elsewhere:?}, where the session is not"
        );
        assert_eq!(
            session_screenshot(&mut nested, output.id).map(|image| image.size()),
            Some(output.logical_size()),
            "the session's own readback is what it drew"
        );
    });
}

#[test]
fn input_injected_on_the_host_seat_arrives_as_input_events() {
    let session = Session::start();
    session.run(|host| {
        let mut nested = Nested::connect(&NestedConfig::new(host.socket())).expect("the session connects");
        let mut probe = Probe::drawing(RED);
        draw(&mut nested, &mut probe);
        let observation = host.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        let window = observation.only().expect("one mapped window").clone();
        let host_output = window_owner(host);
        let at = centre(&window);

        host.control.inject_input(InputEvent::PointerMotion {
            output: host_output,
            position: at,
        });
        host.control.inject_input(InputEvent::PointerButton {
            button: BTN_LEFT,
            pressed: true,
        });

        let expected = Point::new(at.x - window.geometry.x, at.y - window.geometry.y);
        let _ = pump_until(&mut nested, &mut probe, |probe| {
            probe.motions().iter().any(|(_, position)| *position == expected)
                && probe.inputs.contains(&InputEvent::PointerButton {
                    button: BTN_LEFT,
                    pressed: true,
                })
        });
        let session_output = only_output(&nested).id;
        assert!(
            probe
                .motions()
                .iter()
                .any(|(output, position)| *output == session_output && *position == expected),
            "a host position arrives as a position on the session's output at {expected:?}: {:?}",
            probe.motions()
        );
        assert!(
            probe.inputs.contains(&InputEvent::PointerButton {
                button: BTN_LEFT,
                pressed: true,
            }),
            "the press arrived: {:?}",
            probe.inputs
        );
    });
}

#[test]
fn a_frame_waits_for_the_hosts_frame_callback() {
    let session = Session::start();
    session.run(|host| {
        let mut nested = Nested::connect(&NestedConfig::new(host.socket())).expect("the session connects");
        let mut probe = Probe::drawing(RED);
        let output = only_output(&nested).id;

        assert_eq!(
            nested.render(&mut probe, output).expect("a first frame"),
            Present::Rendered,
            "the first frame is drawn, because no callback is outstanding"
        );
        assert_eq!(
            nested.render(&mut probe, output).expect("a second frame"),
            Present::Nothing,
            "the host has not answered the first frame, so it has not asked for a second"
        );

        // The answer arrives on the wire, and only then is another frame drawn. The loop
        // is bounded because a host that never answered would be the failure this test
        // exists to catch.
        let deadline = Instant::now() + TIMEOUT;
        loop {
            assert!(
                Instant::now() < deadline,
                "the host never asked for a second frame after the first one it was shown"
            );
            nested.roundtrip().expect("the host answers a round trip");
            if nested.render(&mut probe, output).expect("a third frame") == Present::Rendered {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
    });
}

#[test]
fn killing_the_host_reports_a_disconnect_without_panicking() {
    let session = Session::start();
    session.run(|host| {
        let mut nested = Nested::connect(&NestedConfig::new(host.socket())).expect("the session connects");
        let mut probe = Probe::drawing(RED);
        draw(&mut nested, &mut probe);
        host.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));

        // The worker stops the host, which ends its loop and closes every client
        // connection. `Session::run` joins this worker after the loop returns.
        host.control.stop();

        let deadline = Instant::now() + TIMEOUT;
        let mut events = Vec::new();
        loop {
            assert!(
                Instant::now() < deadline,
                "the session never noticed the host had gone; events: {events:?}"
            );
            match nested.roundtrip() {
                Ok(()) => events.extend(nested.dispatch(&mut probe)),
                Err(_) => {
                    events.extend(nested.dispatch(&mut probe));
                    break;
                }
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            events.contains(&BackendEvent::Disconnected),
            "the session was told its parent is gone: {events:?}"
        );
        assert!(
            session_outputs(&nested).is_empty(),
            "and it has no output left to draw on"
        );
        // A frame after the disconnect is a refusal, not a panic.
        assert!(
            nested.render(&mut probe, OutputId::new(0)).is_err(),
            "rendering an output that is gone is an error the caller handles"
        );
        assert!(
            nested.roundtrip().is_err(),
            "and every later round trip says the same thing"
        );
    });
}
