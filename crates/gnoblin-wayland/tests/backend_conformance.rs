//! One contract, three backends: the same scenarios against headless and nested, and the status table held to both.
//!
//! `design/backend-parity.md` carries a table with one row per capability and one cell per backend, and
//! `docs/backends.md` repeats it for people. This file measures the headless and nested columns by running the
//! *same* scenario against each backend through the `Backend` trait: the scenario observes what the backend did
//! (accepted, refused with the shared typed answer, delivered an event) and the table is the oracle, so a backend
//! that quietly stops doing something, or starts doing something its cell calls refused, fails here. The DRM column
//! is measured against its fake kernel in `gnoblin-backend-drm/src/conformance.rs`.
//!
//! # What is compared, and how closely
//!
//! - **Pixels.** The same scene is drawn on every backend and the same points are read back: inside the shape, outside
//!   it and on both sides of each edge. The software renderer must match **exactly**, because a solid rectangle on
//!   whole pixels has no anti-aliasing to differ on. A GPU renderer (GLES on llvmpipe, Vulkan on lavapipe) may differ
//!   by at most [`GPU_TOLERANCE`] per channel, which is the rounding of a float colour to eight bits twice.
//! - **Refusals.** A change a backend cannot make is `BackendError::Unsupported`, the same answer on every backend
//!   that refuses: a different error is a failure, because the vocabulary is the contract.
//! - **Input.** Every `InputEvent` variant a backend can produce is injected where the backend's input comes from (the
//!   headless queue, the host's seat for nested) and must arrive with the same payload.
//!
//! Nested is hosted by this repository's own headless compositor, which is a real Wayland compositor; a host such
//! as Sway is a different implementation of the same protocol, and the table says so (see the design document).

use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::headless::{Headless, HeadlessConfig, HeadlessHandle};
use gnoblin_backend::nested::{Nested, OutputSpec};
use gnoblin_backend::render::renderer::RendererKind;
use gnoblin_backend::{
    Backend, BackendError, BackendEvent, DeviceInfo, DeviceKind, GammaRamp, InputEvent, Present, Stylus, TabletEvent,
    TabletTool, TouchPhase,
};
use gnoblin_output::DisplayMode;
use gnoblin_render_frame::frame::Solid;
use gnoblin_testkit::backend_status::{CellStatus, ColumnCheck, StatusTable};
use gnoblin_types::{DeviceId, OutputId, Point, Rect, Scale};

use gnoblin_wayland_testkit::stock;
mod nested_host;
use nested_host::*;

/// The design document: the table the code is held to is the committed one.
const DESIGN: &str = include_str!("../../../design/backend-parity.md");

/// The user page, which repeats the table in plain words.
const USER_PAGE: &str = include_str!("../../../docs/backends.md");

/// The nested tests, whose names the table's `covered` rows cite.
const NESTED_TESTS: &str = include_str!("nested.rs");

/// The clipboard tests, which run a real client on each side of a nested session.
const SELECTION_BRIDGE_TESTS: &str = include_str!("selection_bridge.rs");

/// The tests of presenting through the host's dmabuf.
const NESTED_DMABUF_TESTS: &str = include_str!("nested_dmabuf.rs");

/// The most a GPU renderer may differ from the software reference at a probe, per channel.
const GPU_TOLERANCE: i32 = 2;

fn table() -> StatusTable {
    StatusTable::parse(DESIGN).expect("design/backend-parity.md carries a parseable status table")
}

/// A refusal is [`CellStatus::Refused`] only when it is the shared typed answer.
#[track_caller]
fn status_of(result: Result<(), BackendError>) -> CellStatus {
    match result {
        Ok(()) => CellStatus::Supported,
        Err(BackendError::Unsupported(_)) => CellStatus::Refused,
        Err(other) => panic!("a refusal outside the shared vocabulary: {other:?}"),
    }
}

/// What a scenario needs from a backend that the trait does not give it.
trait Rig {
    fn backend(&mut self) -> &mut dyn Backend<Probe>;

    /// Delivers what the backend has to deliver, as one turn of the session's loop does.
    fn pump(&mut self, probe: &mut Probe) -> Vec<BackendEvent>;

    /// Draws and presents one frame on `output`, waiting for the backend to be ready to.
    fn present(&mut self, probe: &mut Probe, output: OutputId) -> Present;

    /// Connects a second output at run time and returns it.
    fn connect_output(&mut self, probe: &mut Probe) -> OutputId;

    /// Disconnects `output` at run time.
    fn disconnect_output(&mut self, probe: &mut Probe, output: OutputId);

    /// Produces `event` where this backend's input comes from.
    fn inject(&mut self, probe: &mut Probe, event: &InputEvent);

    /// The first output.
    fn first(&mut self) -> gnoblin_backend::Output {
        self.backend().outputs().first().cloned().expect("an output")
    }
}

// ---- headless --------------------------------------------------------------------------------------------------------

struct HeadlessRig {
    backend: Headless,
    handle: HeadlessHandle,
}

impl HeadlessRig {
    fn new(config: HeadlessConfig) -> Option<Self> {
        let backend = Headless::with_config(config).ok()?;
        let handle = backend.handle();
        Some(Self { backend, handle })
    }
}

impl Rig for HeadlessRig {
    fn backend(&mut self) -> &mut dyn Backend<Probe> {
        &mut self.backend
    }

    fn pump(&mut self, probe: &mut Probe) -> Vec<BackendEvent> {
        Backend::<Probe>::dispatch(&mut self.backend, probe)
    }

    fn present(&mut self, probe: &mut Probe, output: OutputId) -> Present {
        Backend::<Probe>::render(&mut self.backend, probe, output).expect("a frame")
    }

    fn connect_output(&mut self, probe: &mut Probe) -> OutputId {
        self.handle
            .connect_virtual_output("HEADLESS-2", 800, 600)
            .expect("a virtual output");
        connected(&self.pump(probe))
    }

    fn disconnect_output(&mut self, probe: &mut Probe, output: OutputId) {
        self.handle.disconnect_output(output);
        let events = self.pump(probe);
        assert!(events.contains(&BackendEvent::OutputDisconnected(output)), "{events:?}");
    }

    fn inject(&mut self, _probe: &mut Probe, event: &InputEvent) {
        self.backend.push_input(event.clone());
    }
}

/// The output a batch of events connected.
#[track_caller]
fn connected(events: &[BackendEvent]) -> OutputId {
    events
        .iter()
        .find_map(|event| match event {
            BackendEvent::OutputConnected(output) => Some(output.id),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no output was connected: {events:?}"))
}

// ---- nested ----------------------------------------------------------------------------------------------------------

struct NestedRig<'a> {
    nested: Nested,
    host: &'a Handle,
}

impl<'a> NestedRig<'a> {
    fn connect(host: &'a Handle, renderer: RendererKind) -> Option<Self> {
        let nested = connect_drawing_with(host, renderer)?;
        Some(Self { nested, host })
    }

    /// The host's window of the session, once it is mapped.
    fn window(&self) -> gnoblin_wayland::ObservedWindow {
        self.host
            .wait_for(|observation| observation.windows().any(|window| window.mapped))
            .windows()
            .find(|window| window.mapped)
            .expect("a mapped window")
            .clone()
    }
}

impl Rig for NestedRig<'_> {
    fn backend(&mut self) -> &mut dyn Backend<Probe> {
        &mut self.nested
    }

    fn pump(&mut self, probe: &mut Probe) -> Vec<BackendEvent> {
        self.nested.roundtrip().expect("the host answers a round trip");
        let events = self.nested.dispatch(probe);
        thread::sleep(Duration::from_millis(2));
        events
    }

    fn present(&mut self, probe: &mut Probe, output: OutputId) -> Present {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            self.nested.roundtrip().expect("the host answers a round trip");
            let _ = self.nested.dispatch(probe);
            if self.nested.render(probe, output).expect("a frame") == Present::Rendered {
                self.nested.roundtrip().expect("the host answers a round trip");
                return Present::Rendered;
            }
            assert!(Instant::now() < deadline, "the host never asked for another frame");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn connect_output(&mut self, probe: &mut Probe) -> OutputId {
        self.nested.handle().connect_output(OutputSpec::new("NESTED-2"));
        let events = collect_until(&mut self.nested, probe, |events| {
            events
                .iter()
                .any(|event| matches!(event, BackendEvent::OutputConnected(_)))
        });
        connected(&events)
    }

    fn disconnect_output(&mut self, probe: &mut Probe, output: OutputId) {
        self.nested.handle().disconnect_output(output);
        let events = collect_until(&mut self.nested, probe, |events| {
            events.contains(&BackendEvent::OutputDisconnected(output))
        });
        assert!(!events.contains(&BackendEvent::SessionEnded), "{events:?}");
    }

    fn inject(&mut self, _probe: &mut Probe, event: &InputEvent) {
        let window = self.window();
        let host_output = window_owner(self.host);
        let at = |local: Point| Point::new(window.geometry.x + local.x, window.geometry.y + local.y);
        let host_event = match event {
            InputEvent::PointerMotion { position, .. } => InputEvent::PointerMotion {
                device: DeviceId::VIRTUAL,
                output: host_output,
                position: at(*position),
            },
            InputEvent::Touch {
                slot, position, phase, ..
            } => InputEvent::Touch {
                device: DeviceId::VIRTUAL,
                output: host_output,
                slot: *slot,
                position: at(*position),
                phase: *phase,
            },
            InputEvent::Key { .. } => {
                // The keyboard goes to the window that has the focus, so the window is given it first.
                let _ = self.host.control.focus(window.id);
                event.clone()
            }
            InputEvent::PointerButton { .. } | InputEvent::PointerAxis { .. } => event.clone(),
            other => panic!("the host cannot be made to produce {other:?}"),
        };
        self.host.control.inject_input(host_event);
    }
}

// ---- scenarios -------------------------------------------------------------------------------------------------------

/// Whether `got` is `expected` in everything the backend is responsible for: the device and the output are the
/// backend's own numbering, so they are not compared.
fn same_input(got: &InputEvent, expected: &InputEvent) -> bool {
    use InputEvent::*;
    match (got, expected) {
        (PointerMotion { position: a, .. }, PointerMotion { position: b, .. }) => a == b,
        (Tablet(a), Tablet(b)) => a == b,
        (PointerMotionRelative { dx: ax, dy: ay, .. }, PointerMotionRelative { dx: bx, dy: by, .. }) => {
            (ax, ay) == (bx, by)
        }
        (
            PointerButton {
                button: a, pressed: p, ..
            },
            PointerButton {
                button: b, pressed: q, ..
            },
        ) => (a, p) == (b, q),
        (
            Key {
                keycode: a, pressed: p, ..
            },
            Key {
                keycode: b, pressed: q, ..
            },
        ) => (a, p) == (b, q),
        (
            PointerAxis {
                horizontal: ah,
                vertical: av,
                ..
            },
            PointerAxis {
                horizontal: bh,
                vertical: bv,
                ..
            },
        ) => (ah, av) == (bh, bv),
        (
            Touch {
                slot: a,
                position: p,
                phase: f,
                ..
            },
            Touch {
                slot: b,
                position: q,
                phase: g,
                ..
            },
        ) => (a, p, f) == (b, q, g),
        (DeviceAdded { device: a }, DeviceAdded { device: b }) => (&a.name, a.kind) == (&b.name, b.kind),
        (DeviceRemoved { .. }, DeviceRemoved { .. }) | (PointerLeave, PointerLeave) => true,
        (Touchpad(a), Touchpad(b)) => (a.phase, a.kind, a.fingers) == (b.phase, b.kind, b.fingers),
        _ => false,
    }
}

/// Produces `event` and waits for the backend to hand the same event to the host.
#[track_caller]
fn delivered(rig: &mut dyn Rig, probe: &mut Probe, event: &InputEvent) {
    rig.inject(probe, event);
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let _ = rig.pump(probe);
        if probe.inputs.iter().any(|got| same_input(got, event)) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{event:?} never arrived; the backend delivered {:?}",
            probe.inputs
        );
    }
}

/// Outputs, modes, the changes a backend may refuse and the shared vocabulary of the refusals.
fn outputs(rig: &mut dyn Rig, m: &mut ColumnCheck, probe: &mut Probe) {
    let first = rig.first();
    assert!(first.mode.width > 0 && first.mode.height > 0, "an output has pixels");
    assert!(!first.connector.is_empty(), "an output has a name");
    m.is("outputs.connect", CellStatus::Supported);

    let modes = rig.backend().output_modes(first.id);
    assert!(
        modes.contains(&first.mode),
        "the mode an output is in is one it lists: {modes:?}"
    );
    assert!(
        rig.backend().output_modes(OutputId::new(u64::MAX)).is_empty(),
        "an output the backend does not drive lists no mode"
    );
    m.is("outputs.modes", CellStatus::Supported);

    // A mode change: accepted where the backend can re-mode, refused with the shared answer where the host or the
    // hardware decides. A mode with no pixels is refused everywhere, though not necessarily with the same words.
    let smaller = DisplayMode::new(800, 600, first.mode.refresh_mhz);
    let remode = rig
        .backend()
        .configure_output(first.id, smaller, first.scale, Scale::IDENTITY);
    m.is("outputs.remode", status_of(remode));
    assert!(
        rig.backend()
            .configure_output(first.id, DisplayMode::new(0, 600, 60_000), first.scale, Scale::IDENTITY)
            .is_err(),
        "a mode with no pixels is refused"
    );
    let _ = rig.pump(probe);
    let now = rig.first();
    let scale = rig
        .backend()
        .configure_output(first.id, now.mode, Scale::integral(2), Scale::IDENTITY);
    m.is("outputs.scale", status_of(scale));
    // Put back what a backend that accepted changed, so the scenarios after this one see the output they expect.
    let _ = rig
        .backend()
        .configure_output(first.id, first.mode, first.scale, Scale::IDENTITY);
    let _ = rig.pump(probe);

    // Composing larger than the display and averaging down is the one change every backend can make.
    let back = rig.first();
    let supersample = rig
        .backend()
        .test_output(first.id, back.mode, back.scale, Scale::integral(2))
        .and_then(|()| {
            rig.backend()
                .configure_output(first.id, back.mode, back.scale, Scale::integral(2))
        });
    m.is("outputs.supersample", status_of(supersample));
    let _ = rig.pump(probe);
    rig.backend()
        .configure_output(first.id, back.mode, back.scale, Scale::IDENTITY)
        .expect("a backend that supersamples can stop");
    let _ = rig.pump(probe);
}

/// The scene every backend draws: a red rectangle on black, with points on both sides of each edge.
const SHAPE: Rect = Rect::new(10, 10, 100, 60);

/// The probes and what they must read. `(x, y, red?)`.
const PROBES: [(i32, i32, bool); 8] = [
    (50, 40, true),
    (10, 10, true),
    (109, 69, true),
    (9, 9, false),
    (110, 70, false),
    (109, 70, false),
    (9, 40, false),
    (200, 200, false),
];

/// Draws the scene and checks the readback against what the pixels must be, within `tolerance` per channel.
#[track_caller]
fn scene_matches_reference(rig: &mut dyn Rig, probe: &mut Probe, tolerance: i32) {
    let output = rig.first();
    *probe = Probe::drawing_shape(Solid::new(SHAPE, RED));
    rig.present(probe, output.id);
    let image = rig
        .backend()
        .capture(output.id, false)
        .expect("a backend with a renderer reads its frame back");
    for (x, y, red) in PROBES {
        let expected = if red { RED_PIXEL } else { BLACK_PIXEL };
        let seen = image.colour_at(x, y).unwrap_or_else(|| panic!("no pixel at {x},{y}"));
        let worst = seen
            .iter()
            .zip(expected)
            .map(|(got, want)| (i32::from(*got) - i32::from(want)).abs())
            .max()
            .unwrap_or(0);
        assert!(
            worst <= tolerance,
            "pixel {x},{y} is {seen:?}, the reference is {expected:?} (tolerance {tolerance})"
        );
    }
}

/// A frame is drawn, read back, and the same scene gives the same pixels.
///
/// "Presented" means what the backend does with a drawn frame: nested hands a buffer to the host and
/// [`Rig::present`] waits until it did, headless has nowhere to scan out to and answers [`Present::Nothing`]. Either
/// way the frame exists, and the readback is where it is proven.
fn rendering(rig: &mut dyn Rig, m: &mut ColumnCheck, probe: &mut Probe) {
    let output = rig.first();
    scene_matches_reference(rig, probe, 0);
    m.is("render.present", CellStatus::Supported);
    m.is("render.pixels_equal", CellStatus::Supported);
    m.is("render.software", CellStatus::Supported);
    let readback = rig.backend().capture(output.id, false);
    assert_eq!(
        readback.as_ref().map(gnoblin_render_frame::image::Image::size),
        Some(gnoblin_types::Size::new(output.mode.width, output.mode.height)),
        "the readback is the output's mode"
    );
    m.is("render.readback", CellStatus::Supported);
}

/// One GPU renderer's frame matches the reference within the stated tolerance, and its GPU answers are the ones
/// headless gives for the same renderer.
fn gpu_renderer(rig: &mut dyn Rig, m: &mut ColumnCheck, probe: &mut Probe, kind: RendererKind) {
    scene_matches_reference(rig, probe, GPU_TOLERANCE);
    let output = rig.first();
    let backend = rig.backend();
    let draws_views = backend.gpu_views();
    if kind == RendererKind::Gles {
        assert!(draws_views, "GLES draws a View on the GPU");
        m.is("gpu.views", CellStatus::Supported);
    }
    let reference = Headless::with_config(HeadlessConfig {
        renderer: kind,
        ..HeadlessConfig::default()
    })
    .expect("a headless backend with the renderer the other one made");
    let reference_output = Backend::<Probe>::outputs(&reference)[0].id;
    assert_eq!(
        Backend::<Probe>::gpu_views(&reference),
        draws_views,
        "whether a View is drawn on the GPU"
    );
    assert_eq!(
        Backend::<Probe>::effect_kinds(&reference),
        backend.effect_kinds(),
        "the effects the renderer draws"
    );
    let footprint = backend.footprint(output.id);
    assert_ne!(
        footprint,
        gnoblin_render_frame::footprint::Footprint::default(),
        "a GPU renderer holds memory it reports"
    );
    assert_ne!(
        Backend::<Probe>::footprint(&reference, reference_output),
        gnoblin_render_frame::footprint::Footprint::default(),
        "and so does the reference"
    );
    m.is("gpu.footprint", CellStatus::Supported);
    let mut reference = reference;
    let offers = |backend: &mut dyn Backend<Probe>| backend.dmabuf().map(|support| support.formats.iter().count());
    assert_eq!(
        offers(backend),
        offers(&mut reference),
        "the dmabuf formats clients are offered are the same code's answer"
    );
    m.is("gpu.dmabuf", CellStatus::Supported);
}

/// Gamma and power are controls of a display: virtual on headless, refused with the shared answer on a window.
fn controls(rig: &mut dyn Rig, m: &mut ColumnCheck) {
    let output = rig.first();
    let controls = rig.backend().output_controls();
    let identity: Vec<u16> = (0..256u32)
        .map(|step| u16::try_from(step * 257).unwrap_or(u16::MAX))
        .collect();
    let ramp = GammaRamp {
        red: &identity,
        green: &identity,
        blue: &identity,
    };
    let gamma = status_of(rig.backend().set_gamma(output.id, ramp));
    m.is("control.gamma", gamma);
    assert_eq!(
        controls.gamma,
        gamma == CellStatus::Supported,
        "the controls the backend reports are the ones it has"
    );
    assert_eq!(
        rig.backend().gamma_size(output.id).is_some(),
        gamma == CellStatus::Supported,
        "a refused table has no size to state"
    );
    assert_eq!(status_of(rig.backend().restore_gamma(output.id)), gamma);
    let power = status_of(rig.backend().set_power(output.id, false));
    m.is("control.power", power);
    assert_eq!(controls.power, power == CellStatus::Supported);
    if power == CellStatus::Supported {
        rig.backend()
            .set_power(output.id, true)
            .expect("a backend that switches off can switch on");
    }
}

/// Pointer, scroll, keys and touch arrive with the payload they were produced with.
fn input(rig: &mut dyn Rig, m: &mut ColumnCheck, probe: &mut Probe) {
    let output = rig.first();
    let inside = Point::new(120, 90);
    delivered(
        rig,
        probe,
        &InputEvent::PointerMotion {
            device: DeviceId::VIRTUAL,
            output: output.id,
            position: inside,
        },
    );
    delivered(
        rig,
        probe,
        &InputEvent::PointerButton {
            device: DeviceId::VIRTUAL,
            button: BTN_LEFT,
            pressed: true,
        },
    );
    m.is("input.pointer", CellStatus::Supported);
    delivered(
        rig,
        probe,
        &InputEvent::Key {
            device: DeviceId::VIRTUAL,
            keycode: 30,
            pressed: true,
        },
    );
    m.is("input.keyboard", CellStatus::Supported);
    delivered(
        rig,
        probe,
        &InputEvent::PointerAxis {
            device: DeviceId::VIRTUAL,
            horizontal: 0,
            vertical: 120,
        },
    );
    m.is("input.axis", CellStatus::Supported);
    for (phase, at) in [
        (TouchPhase::Down, Point::new(40, 40)),
        (TouchPhase::Motion, Point::new(44, 44)),
        (TouchPhase::Up, Point::new(44, 44)),
    ] {
        delivered(
            rig,
            probe,
            &InputEvent::Touch {
                device: DeviceId::VIRTUAL,
                output: output.id,
                slot: 0,
                position: at,
                phase,
            },
        );
    }
    m.is("input.touch", CellStatus::Supported);
}

/// The document's `input` section, given to the backend's devices through the trait: applied (`supported`) or refused with
/// the shared typed answer (`refused`), never silently dropped.
#[track_caller]
fn input_settings(rig: &mut dyn Rig) -> CellStatus {
    let section = gnoblin_config::Input {
        mouse: Some(gnoblin_config::Mouse {
            speed: Some(gnoblin_config::Inherit::Value(0.5)),
            ..gnoblin_config::Mouse::default()
        }),
        ..gnoblin_config::Input::default()
    };
    match rig.backend().set_input_settings(&section) {
        Ok(()) => CellStatus::Supported,
        Err(gnoblin_backend::InputSettingsRefusal::HostOwnsDevices) => CellStatus::Refused,
        Err(other) => panic!("a refusal outside the shared vocabulary: {other:?}"),
    }
}

#[test]
fn the_status_table_holds_for_headless() {
    let table = table();
    let mut m = ColumnCheck::new(&table, "headless");
    let mut rig = HeadlessRig::new(HeadlessConfig::default()).expect("the software renderer needs no GPU");
    let mut probe = Probe::drawing(RED);
    outputs(&mut rig, &mut m, &mut probe);
    rendering(&mut rig, &mut m, &mut probe);
    controls(&mut rig, &mut m);
    input(&mut rig, &mut m, &mut probe);

    // The renderers: GLES and Vulkan where the machine can create them.
    for (id, kind, config) in [
        ("render.gles", RendererKind::Gles, HeadlessConfig::gles()),
        ("render.vulkan", RendererKind::Vulkan, HeadlessConfig::vulkan()),
    ] {
        match HeadlessRig::new(config) {
            Some(mut gpu) => {
                let mut probe = Probe::drawing(RED);
                gpu_renderer(&mut gpu, &mut m, &mut probe, kind);
                m.is(id, CellStatus::Supported);
            }
            None => m.skipped(id, "this machine cannot create the renderer"),
        }
    }

    // Hotplug and the rest of what only a virtual backend can be driven to do.
    let mut probe = Probe::drawing(RED);
    let added = rig.connect_output(&mut probe);
    assert_eq!(rig.backend().outputs().len(), 2);
    rig.disconnect_output(&mut probe, added);
    assert_eq!(rig.backend().outputs().len(), 1);
    m.is("outputs.hotplug", CellStatus::Supported);

    // Relative motion, devices and gestures arrive from the queue like any other event.
    delivered(
        &mut rig,
        &mut probe,
        &InputEvent::PointerMotionRelative {
            device: DeviceId::VIRTUAL,
            dx: 5,
            dy: -3,
        },
    );
    m.is("input.relative", CellStatus::Supported);
    let device = DeviceInfo::new(DeviceId::new(7), "conformance keyboard", DeviceKind::Keyboard);
    delivered(&mut rig, &mut probe, &InputEvent::DeviceAdded { device });
    delivered(
        &mut rig,
        &mut probe,
        &InputEvent::DeviceRemoved { id: DeviceId::new(7) },
    );
    m.is("input.devices", CellStatus::Supported);
    // The document's input section reaches a virtual device the session has, and the one that arrives after it.
    let mouse = DeviceId::new(11);
    delivered(
        &mut rig,
        &mut probe,
        &InputEvent::DeviceAdded {
            device: DeviceInfo::new(mouse, "conformance mouse", DeviceKind::Mouse),
        },
    );
    let applied = input_settings(&mut rig);
    m.is("input.settings", applied);
    assert_eq!(
        rig.handle.device_settings(mouse),
        Some(vec![gnoblin_backend::input_settings::Setting::Speed(0.5)]),
        "the section reached the virtual mouse"
    );
    let later = DeviceId::new(12);
    delivered(
        &mut rig,
        &mut probe,
        &InputEvent::DeviceAdded {
            device: DeviceInfo::new(later, "conformance mouse 2", DeviceKind::Mouse),
        },
    );
    assert_eq!(
        rig.handle.device_settings(later),
        Some(vec![gnoblin_backend::input_settings::Setting::Speed(0.5)]),
        "a device that arrives later is configured from the section the backend holds"
    );
    delivered(&mut rig, &mut probe, &InputEvent::DeviceRemoved { id: mouse });
    assert_eq!(rig.handle.device_settings(mouse), None, "a removed device is forgotten");
    let sample = gnoblin_backend::RawTouchpadSample {
        device: DeviceId::new(7),
        phase: gnoblin_backend::RawTouchpadPhase::Begin,
        kind: gnoblin_backend::RawTouchpadKind::Swipe,
        fingers: 3,
        dx: 0.0,
        dy: 0.0,
        scale: 1.0,
        angle: 0.0,
    };
    delivered(&mut rig, &mut probe, &InputEvent::Touchpad(sample));
    m.is("input.touchpad", CellStatus::Supported);
    delivered(&mut rig, &mut probe, &InputEvent::PointerLeave);

    // A drawing tablet: the synthetic stylus produces the stream a device would, and every event of it arrives, in order.
    let output = rig.first().id;
    let mut pen = Stylus::new(DeviceId::new(9), output, TabletTool::pen(7));
    let stroke = pen.stroke(&[(20.0, 20.0), (30.0, 25.5), (40.0, 30.0)]);
    assert!(
        stroke.len() >= 6,
        "a stroke has a proximity, a tip, motion and the way out: {stroke:?}"
    );
    for event in &stroke {
        delivered(&mut rig, &mut probe, event);
    }
    m.is("input.tablet_tool", CellStatus::Supported);
    for event in [
        TabletEvent::PadButton {
            device: DeviceId::new(10),
            button: 1,
            pressed: true,
        },
        TabletEvent::PadRing {
            device: DeviceId::new(10),
            ring: 0,
            angle: Some(45.0),
        },
        TabletEvent::PadStrip {
            device: DeviceId::new(10),
            strip: 0,
            position: None,
        },
    ] {
        delivered(&mut rig, &mut probe, &event.into());
    }
    m.is("input.tablet_pad", CellStatus::Supported);

    // The session ending and the device being lost are each reported once.
    rig.handle.end_session();
    let events = rig.pump(&mut probe);
    assert_eq!(
        events
            .iter()
            .filter(|event| **event == BackendEvent::SessionEnded)
            .count(),
        1,
        "{events:?}"
    );
    m.is("lifecycle.session_end", CellStatus::Supported);
    let mut lost = HeadlessRig::new(HeadlessConfig::default()).expect("a second backend");
    lost.handle.lose_device();
    let events = lost.pump(&mut probe);
    assert_eq!(
        events
            .iter()
            .filter(|event| **event == BackendEvent::Disconnected)
            .count(),
        1,
        "{events:?}"
    );
    assert!(lost.pump(&mut probe).is_empty(), "a lost device is reported once");
    m.is("lifecycle.backend_lost", CellStatus::Supported);
    drop(lost);
    m.is("lifecycle.shutdown", CellStatus::Supported);

    // By nature: there is no host, no window, no host seat.
    for id in [
        "outputs.host_scale",
        "input.focus_loss",
        "input.shortcut_inhibit",
        "input.pointer_constraints",
        "bridge.clipboard",
        "bridge.window_identity",
        "bridge.dmabuf_present",
    ] {
        m.is(id, CellStatus::NotApplicable);
    }
    m.skip_unseen(
        &["gpu.views", "gpu.footprint", "gpu.dmabuf"],
        "this machine cannot create a GPU renderer",
    );
    m.finish();
}

#[test]
fn the_status_table_holds_for_nested() {
    let session = Session::start();
    session.run(|host| {
        let table = table();
        let mut m = ColumnCheck::new(&table, "nested");
        let mut probe = Probe::drawing(RED);
        let mut rig = NestedRig::connect(host, RendererKind::Software).expect("the software renderer needs no GPU");
        let _ = rig.pump(&mut probe);
        // The host's seat has a pointer and a keyboard, and the session's seat lists them: devices are announced.
        draw(&mut rig.nested, &mut probe);
        host.wait_for(|observation| observation.windows().any(|window| window.mapped));
        let _ = rig.pump(&mut probe);
        assert!(
            probe
                .inputs
                .iter()
                .any(|event| matches!(event, InputEvent::DeviceAdded { .. })),
            "the host's devices join the session's seat: {:?}",
            probe.inputs
        );
        m.is("input.devices", CellStatus::Supported);
        // The host's seat is the host's to configure: the section is refused, with the shared typed answer.
        m.is("input.settings", input_settings(&mut rig));

        outputs(&mut rig, &mut m, &mut probe);
        rendering(&mut rig, &mut m, &mut probe);
        controls(&mut rig, &mut m);
        input(&mut rig, &mut m, &mut probe);

        for (id, kind) in [
            ("render.gles", RendererKind::Gles),
            ("render.vulkan", RendererKind::Vulkan),
        ] {
            match NestedRig::connect(host, kind) {
                Some(mut gpu) => {
                    let mut probe = Probe::drawing(RED);
                    gpu_renderer(&mut gpu, &mut m, &mut probe, kind);
                    m.is(id, CellStatus::Supported);
                    // The second connection is a second window; close it so the rest see the host as it was.
                    drop(gpu);
                    host.wait_for(|observation| observation.windows().filter(|window| window.mapped).count() <= 1);
                }
                None => m.skipped(id, "this machine cannot create the renderer"),
            }
        }

        // Hotplug is a window opening and closing on the host.
        let added = rig.connect_output(&mut probe);
        assert_eq!(rig.backend().outputs().len(), 2);
        rig.disconnect_output(&mut probe, added);
        assert_eq!(rig.backend().outputs().len(), 1);
        m.is("outputs.hotplug", CellStatus::Supported);

        // Proven by the tests of nested.rs, which drive the host's own seat, scale, focus and window list.
        m.covered(
            "outputs.host_scale",
            CellStatus::Supported,
            "a_host_scale_change_reaches_the_nested_output",
            NESTED_TESTS,
        );
        m.covered(
            "input.focus_loss",
            CellStatus::Supported,
            "a_key_held_when_the_host_moves_the_keyboard_focus_is_released",
            NESTED_TESTS,
        );
        m.covered(
            "input.shortcut_inhibit",
            CellStatus::Supported,
            "the_nested_window_asks_the_host_to_keep_its_shortcuts_while_it_has_the_keyboard",
            NESTED_TESTS,
        );
        m.covered(
            "bridge.window_identity",
            CellStatus::Supported,
            "the_host_shows_the_nested_window_with_its_title_app_id_and_icon",
            NESTED_TESTS,
        );
        m.covered(
            "lifecycle.session_end",
            CellStatus::Supported,
            "closing_the_last_window_ends_the_session_and_closing_one_of_two_does_not",
            NESTED_TESTS,
        );
        m.covered(
            "lifecycle.backend_lost",
            CellStatus::Supported,
            "killing_the_host_reports_a_disconnect_without_panicking",
            NESTED_TESTS,
        );
        // Declared: the repository's own host sends no gesture to a client, so the gesture path is unit-tested in the
        // backend.
        m.declared("input.touchpad", "unit tested");
        m.covered(
            "input.tablet_tool",
            CellStatus::Supported,
            "a_hosts_pen_arrives_as_tablet_events",
            NESTED_TESTS,
        );
        m.covered(
            "input.tablet_pad",
            CellStatus::Supported,
            "a_hosts_pad_arrives_as_pad_events",
            NESTED_TESTS,
        );
        m.covered(
            "bridge.dmabuf_present",
            CellStatus::Supported,
            "a_frame_is_presented_through_the_hosts_dmabuf",
            NESTED_DMABUF_TESTS,
        );
        m.covered(
            "outputs.host_scale",
            CellStatus::Supported,
            "a_host_that_states_only_an_integer_scale_is_followed_to_it",
            NESTED_TESTS,
        );
        m.covered(
            "outputs.host_scale",
            CellStatus::Supported,
            "a_host_fractional_scale_is_followed_exactly",
            NESTED_TESTS,
        );
        m.covered(
            "input.relative",
            CellStatus::Supported,
            "a_lock_in_the_session_locks_the_hosts_pointer_and_its_relative_motion_arrives",
            NESTED_TESTS,
        );
        m.covered(
            "input.pointer_constraints",
            CellStatus::Supported,
            "a_lock_in_the_session_locks_the_hosts_pointer_and_its_relative_motion_arrives",
            NESTED_TESTS,
        );
        m.covered(
            "bridge.clipboard",
            CellStatus::Supported,
            "a_copy_on_the_host_is_pasted_in_the_session",
            SELECTION_BRIDGE_TESTS,
        );
        m.covered(
            "bridge.clipboard",
            CellStatus::Supported,
            "a_copy_in_the_session_is_pasted_on_the_host",
            SELECTION_BRIDGE_TESTS,
        );

        m.skip_unseen(
            &["gpu.views", "gpu.footprint", "gpu.dmabuf"],
            "this machine cannot create a GPU renderer",
        );
        // Dropping the backend closes its windows: nothing of the session is left on the host.
        drop(rig);
        host.wait_for(|observation| observation.windows().next().is_none());
        m.is("lifecycle.shutdown", CellStatus::Supported);
        m.finish();
    });
}

#[test]
fn the_user_page_says_what_the_design_table_says() {
    let design = table();
    let page = StatusTable::parse(USER_PAGE).expect("docs/backends.md carries a parseable status table");
    assert_eq!(
        page.rows.len(),
        design.rows.len(),
        "the user page lists the same capabilities as the design table"
    );
    for row in &design.rows {
        let words = page
            .rows
            .iter()
            .find(|candidate| candidate.capability == row.capability)
            .unwrap_or_else(|| panic!("docs/backends.md has no row for `{}`", row.capability));
        for backend in gnoblin_testkit::backend_status::BACKENDS {
            assert_eq!(
                words.cell(backend).status,
                row.cell(backend).status,
                "docs/backends.md and the design table disagree about `{}` on {backend}",
                row.id
            );
        }
    }
}
