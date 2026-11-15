//! The pieces of a nested session test that more than one test file shares: a real `gnoblin-wayland` host on the
//! headless backend, a [`Probe`] standing in for the session's own compositor, and the helpers that pump a
//! [`Nested`] backend against that host.
//!
//! Each test file uses its own part of this, so what one does not call is not dead.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::Headless;
use gnoblin_backend::nested::{Nested, NestedConfig};
use gnoblin_backend::{Backend, BackendEvent, Host, InputEvent, Output, Present};
use gnoblin_render_frame::frame::{Layer, OutputFrame, Solid};
use gnoblin_types::{Color, OutputId, Point, Rect, Size};
use gnoblin_wayland::{Config, ControlHandle, LoopError, ObservedWindow, Server, SessionSnapshot};

/// How long a test waits for a state before failing.
pub const TIMEOUT: Duration = Duration::from_secs(20);

/// The red the session draws, which is nothing like the host's black background.
pub const RED: Color = Color::new(1.0, 0.0, 0.0, 1.0);

/// [`RED`] as a screenshot states it.
pub const RED_PIXEL: [u8; 4] = [255, 0, 0, 255];

/// The host's background as a screenshot states it.
pub const BLACK_PIXEL: [u8; 4] = [0, 0, 0, 255];

/// The host's one output, which is the headless backend's default mode.
pub const HEADLESS_SCREEN: Size = Size::new(
    gnoblin_backend::headless::DEFAULT_MODE.width,
    gnoblin_backend::headless::DEFAULT_MODE.height,
);

/// `BTN_LEFT` from `linux/input-event-codes.h`.
pub const BTN_LEFT: u32 = 0x110;

/// What the session's own compositor would do: describe a frame and keep the input.
#[derive(Default)]
pub struct Probe {
    pub inputs: Vec<InputEvent>,
    pub colour: Option<Color>,
    /// A rectangle drawn over the colour, for a test that needs an edge.
    pub shape: Option<Solid>,
}

impl Probe {
    /// A session that draws `colour`.
    pub fn drawing(colour: Color) -> Self {
        Self {
            inputs: Vec::new(),
            colour: Some(colour),
            shape: None,
        }
    }

    /// A session that draws `shape` over a black background: an edge the host can be asked about.
    pub fn drawing_shape(shape: Solid) -> Self {
        Self {
            shape: Some(shape),
            ..Self::drawing(Color::BLACK)
        }
    }

    /// Every pointer motion the session was handed, in arrival order.
    ///
    /// There is more than one: the host sends an enter as soon as the pointer is over the
    /// session's surface, which is before this test moves it.
    pub fn motions(&self) -> Vec<(OutputId, Point)> {
        self.inputs
            .iter()
            .filter_map(|event| match event {
                InputEvent::PointerMotion { output, position, .. } => Some((*output, *position)),
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
        if let Some(shape) = &self.shape {
            frame.push_layer(Layer::Solid(shape.clone()));
        }
        true
    }

    fn input(&mut self, event: InputEvent) {
        self.inputs.push(event);
    }
}

/// What a worker can reach of a running host.
#[derive(Clone)]
pub struct Handle {
    pub socket: PathBuf,
    pub control: ControlHandle,
    pub observation: Arc<Mutex<SessionSnapshot>>,
}

impl Handle {
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// The host's window state right now.
    pub fn windows(&self) -> SessionSnapshot {
        self.observation.lock().expect("the observation lock").clone()
    }

    /// Waits for the host to satisfy `predicate`, or fails with the last state.
    pub fn wait_for(&self, predicate: impl Fn(&SessionSnapshot) -> bool) -> SessionSnapshot {
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
    pub fn wait_for_colour(&self, output: OutputId, point: Point, colour: [u8; 4]) {
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
pub struct StopOnDrop(pub ControlHandle);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// A host that has been created but is not running yet.
pub struct Session {
    /// Kept alive for the session's lifetime: dropping it would unlink the socket.
    #[allow(dead_code)]
    pub directory: tempfile::TempDir,
    pub server: Server,
    pub handle: Handle,
}

impl Session {
    /// Creates a host on a socket of its own.
    pub fn start() -> Self {
        Self::start_with(|config| config)
    }

    /// Creates a host that states its scale to clients only as the integer scale of `wl_output`, as Mutter does until
    /// fractional scaling is enabled: it does not offer `wp_fractional_scale_v1`.
    pub fn start_integer_only() -> Self {
        Self::start_with(|config| Config {
            fractional_scale: false,
            ..config
        })
    }

    /// Creates a host on a socket of its own, with the configuration `adjust` makes of the default one.
    pub fn start_with(adjust: impl FnOnce(Config) -> Config) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let socket = directory.path().join("wayland-0.sock");
        let config = adjust(Config {
            socket_path: Some(socket.clone()),
            ..Config::default()
        });
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
    pub fn run(self, body: impl FnOnce(&Handle) + Send + 'static) {
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
pub fn session_outputs(nested: &Nested) -> &[Output] {
    <Nested as Backend<Probe>>::outputs(nested)
}

/// The session's last frame, read back through the contract.
pub fn session_screenshot(nested: &mut Nested, output: OutputId) -> Option<gnoblin_render_frame::image::Image> {
    <Nested as Backend<Probe>>::screenshot(nested, output)
}

/// The session's only output.
pub fn only_output(nested: &Nested) -> Output {
    session_outputs(nested)
        .first()
        .cloned()
        .expect("the session has one output once the host configured it")
}

/// Renders and presents one frame on every output, then waits for the host to answer.
///
/// Waiting is the point: the backend presents only when the host asked for a frame, so a
/// test that rendered twice without waiting would be testing the pacing, not the frame.
pub fn draw(nested: &mut Nested, probe: &mut Probe) {
    let outputs: Vec<OutputId> = session_outputs(nested).iter().map(|output| output.id).collect();
    for output in outputs {
        let _ = nested.render(probe, output);
    }
    nested.roundtrip().expect("the host answers a round trip");
}

/// Draws a frame on every output, waiting for the host's frame callback of the one before, so the frame is really drawn.
pub fn draw_rendered(nested: &mut Nested, probe: &mut Probe) {
    let outputs: Vec<OutputId> = session_outputs(nested).iter().map(|output| output.id).collect();
    for output in outputs {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            nested.roundtrip().expect("the host answers a round trip");
            let _ = nested.dispatch(probe);
            if nested.render(probe, output).expect("a frame") == Present::Rendered {
                break;
            }
            assert!(Instant::now() < deadline, "the host never asked for another frame");
            thread::sleep(Duration::from_millis(5));
        }
    }
    nested.roundtrip().expect("the host answers a round trip");
}

/// Waits until `predicate` holds for the probe, pumping the session as it goes.
pub fn pump_until(nested: &mut Nested, probe: &mut Probe, predicate: impl Fn(&Probe) -> bool) -> Vec<BackendEvent> {
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
pub fn outside(window: &ObservedWindow, screen: Size) -> Point {
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
pub fn centre(window: &ObservedWindow) -> Point {
    Point::new(
        window.geometry.x + i32::try_from(window.geometry.width / 2).unwrap_or(0),
        window.geometry.y + i32::try_from(window.geometry.height / 2).unwrap_or(0),
    )
}

/// A window's extent as the size type the contract states geometry in.
pub fn extent(window: &ObservedWindow) -> Size {
    Size::new(window.geometry.width, window.geometry.height)
}

/// The host's only output id.
pub fn window_owner(host: &Handle) -> OutputId {
    host.windows().outputs().first().copied().expect("a host output")
}

/// A nested session that draws with `kind`, or `None` when this machine cannot create that renderer: the same
/// skip the headless tests make, so a machine with no EGL or no Vulkan loader still runs the rest.
pub fn connect_drawing_with(host: &Handle, kind: gnoblin_backend::render::renderer::RendererKind) -> Option<Nested> {
    let config = NestedConfig {
        renderer: kind,
        ..NestedConfig::new(host.socket())
    };
    match Nested::connect(&config) {
        Ok(nested) => Some(nested),
        Err(gnoblin_backend::nested::NestedError::Render(error)) => {
            println!("skipping the nested {} test: {error}", kind.name());
            None
        }
        Err(other) => panic!("the session did not connect: {other}"),
    }
}

/// Asks the host to close the window `which` names, as its close button does.
pub fn close_host_window(host: &Handle, which: impl Fn(&ObservedWindow) -> bool) {
    let windows = host.windows();
    let window = windows
        .windows()
        .find(|window| which(window))
        .expect("the window to close");
    host.control
        .window_control(window.id, gnoblin_wayland::WindowChange::Close)
        .expect("the host answers")
        .expect("the host closes the window");
}

/// Pumps the session until the events it collected satisfy `done`, and returns them.
pub fn collect_until(
    nested: &mut Nested,
    probe: &mut Probe,
    done: impl Fn(&[BackendEvent]) -> bool,
) -> Vec<BackendEvent> {
    let deadline = Instant::now() + TIMEOUT;
    let mut events = Vec::new();
    while !done(&events) {
        assert!(
            Instant::now() < deadline,
            "the session never reported what the test waits for: {events:?}"
        );
        nested.roundtrip().expect("the host answers a round trip");
        events.extend(nested.dispatch(probe));
        thread::sleep(Duration::from_millis(5));
    }
    events
}
