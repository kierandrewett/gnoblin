//! The DRM column of the backend status table, measured against a fake kernel.
//!
//! The hardware backend cannot start without a seat, a GPU and a display, so what a machine without them can prove is
//! every decision the backend makes above the ioctl: which connectors become outputs, what a hotplug changes, what a
//! mode or scale change asks the kernel for and in which order, what a gamma table must look like, how libinput's
//! events become the seat's, and when a frame is drawn and flipped. The kernel is [`crate::modeset::tests::Fake`],
//! the same recorder the mode-change tests use; the device side is [`Mapper`], [`Devices`] and [`Presenter`], which
//! are pure.
//!
//! Each row of `design/backend-parity.md`'s status table is either measured here or declared: a declared row is one
//! only hardware can prove, and its table cell must say so (`unproven on hardware`) or name the reason it is refused
//! (`GLES only`, `scan-out only`). The same table is measured for headless and nested in
//! `gnoblin-wayland/tests/backend_conformance.rs`, so one document is held to all three.

use gnoblin_backend::{BackendError, InputEvent, TabletAxes, TabletEvent, TabletTool, TouchPhase};
use gnoblin_testkit::backend_status::{CellStatus, ColumnCheck, StatusTable};
use gnoblin_types::{DeviceId, OutputId, Point, Scale, Size};

use crate::frame::{Presented, Scanout, present};
use crate::gamma::GammaRamp;
use crate::libinput::map::{DeviceEvent, Mapper, Screen, TabletDeviceEvent};
use crate::modeset::tests::{FOUR_K, FULL_HD, Fake, UNLISTED, ask, connector, lit, presenting};
use crate::modeset::{self, ModesetError, ModesetKind};
use crate::plan::{self, Hotplug, Op, OutputState, Power, Seen};
use crate::present::Presenter;
use crate::udev::{DeviceChange, DeviceOverrideSource, Devices, DrmNode, DrmNodeKind};
use crate::{modeset_error, output_of};

/// The design document, so the table the test holds the code to is the one that is committed.
const DESIGN: &str = include_str!("../../../design/backend-parity.md");

fn table() -> StatusTable {
    StatusTable::parse(DESIGN).expect("design/backend-parity.md carries a parseable status table")
}

/// The connector a fake kernel reports: connected, two modes, 256 gamma entries.
fn output() -> gnoblin_backend::Output {
    output_of(&connector(true)).expect("a connected connector with modes is an output")
}

#[test]
fn outputs_come_from_connected_connectors_with_modes() {
    let output = output();
    assert_eq!(
        output.mode, FULL_HD,
        "the first listed mode is the one the output starts in"
    );
    assert_eq!(output.connector, "DP-1");
    let mut no_modes = connector(true);
    no_modes.modes.clear();
    assert!(
        output_of(&no_modes).is_none(),
        "a connector that offers no mode cannot be driven"
    );
}

#[test]
fn a_hotplug_is_the_difference_between_what_is_driven_and_what_the_kernel_reports() {
    let known = vec![connector(true)];
    let gone = [Seen {
        name: "DP-1".to_owned(),
        connected: false,
        modes: Vec::new(),
    }];
    assert_eq!(plan::hotplug(&known, &gone), [Hotplug::Unplugged("DP-1".to_owned())]);
    let back = vec![connector(false)];
    let seen = [Seen {
        name: "DP-1".to_owned(),
        connected: true,
        modes: vec![FULL_HD],
    }];
    assert_eq!(plan::hotplug(&back, &seen), [Hotplug::Plugged("DP-1".to_owned())]);
}

#[test]
fn a_mode_the_kernel_lists_is_committed_and_one_it_does_not_is_the_vocabulary_headless_uses() {
    let id = OutputId::new(1);
    let mut fake = Fake::default();
    let (mut buffers, mut presenter) = presenting(&fake);
    let outcome = modeset::configure(
        &mut fake,
        &connector(true),
        &lit(),
        ask(FOUR_K, Scale::integral(1)),
        &mut buffers,
        &mut presenter,
    )
    .expect("a listed mode is accepted when real mode changes are on");
    assert_eq!(outcome.kind, ModesetKind::Mode);

    let mut fake = Fake::default();
    let (mut buffers, mut presenter) = presenting(&fake);
    let refused = modeset::configure(
        &mut fake,
        &connector(true),
        &lit(),
        ask(UNLISTED, Scale::integral(1)),
        &mut buffers,
        &mut presenter,
    )
    .expect_err("a mode the connector does not list is refused");
    assert_eq!(
        modeset_error(id, &refused),
        BackendError::Unsupported(id),
        "the same typed refusal a virtual output gives for a mode it does not have"
    );

    let mut off = Fake::without_modeset();
    let (mut buffers, mut presenter) = presenting(&off);
    let disabled = modeset::configure(
        &mut off,
        &connector(true),
        &lit(),
        ask(FOUR_K, Scale::integral(1)),
        &mut buffers,
        &mut presenter,
    )
    .expect_err("real mode changes are off unless the process opts in");
    assert_eq!(disabled, ModesetError::Disabled);
    assert!(matches!(
        modeset_error(id, &disabled),
        BackendError::ModesetDisabled(_, hint) if hint.contains("GNOBLIN_DRM_MODESET=1")
    ));
}

#[test]
fn a_scale_change_commits_nothing_and_a_supersample_asks_the_kernel_only_for_the_mode() {
    let mut fake = Fake::default();
    let (mut buffers, mut presenter) = presenting(&fake);
    let scaled = modeset::configure(
        &mut fake,
        &connector(true),
        &lit(),
        ask(FULL_HD, Scale::integral(2)),
        &mut buffers,
        &mut presenter,
    )
    .expect("a scale needs no kernel");
    assert_eq!(scaled.kind, ModesetKind::Scale);

    let mut fake = Fake::default();
    let (mut buffers, mut presenter) = presenting(&fake);
    let mut wanted = ask(FULL_HD, Scale::integral(1));
    wanted.supersample = Scale::integral(2);
    let supersampled = modeset::configure(
        &mut fake,
        &connector(true),
        &lit(),
        wanted,
        &mut buffers,
        &mut presenter,
    )
    .expect("a supersample factor within the GPU's limit is composed");
    assert_eq!(supersampled.state.supersample, Scale::integral(2));
}

/// Records the frame path's calls: a draw into a slot, then the flip of that slot.
#[derive(Default)]
struct FlipRecorder(Vec<String>);

impl Scanout for FlipRecorder {
    type Frame = u32;

    fn draw(&mut self, slot: usize, frame: &u32) -> Result<(), String> {
        self.0.push(format!("draw {slot} {frame}"));
        Ok(())
    }

    fn flip(&mut self, slot: usize) -> Result<(), String> {
        self.0.push(format!("flip {slot}"));
        Ok(())
    }
}

#[test]
fn a_frame_is_drawn_then_flipped_and_only_one_flip_is_in_flight() {
    let mut presenter = Presenter::new();
    let mut recorder = FlipRecorder::default();
    assert_eq!(
        present(&mut presenter, &mut recorder, &7),
        Presented::Queued { slot: 0 }
    );
    presenter.mark_dirty();
    assert_eq!(present(&mut presenter, &mut recorder, &8), Presented::Waiting);
    assert_eq!(recorder.0, ["draw 0 7", "flip 0"]);
}

#[test]
fn a_gamma_table_is_checked_against_the_crtc_before_the_kernel_sees_it() {
    let ramp = GammaRamp::identity(256);
    assert!(ramp.matches(256).is_ok());
    assert!(
        ramp.matches(1024).is_err(),
        "a table of the wrong length never reaches the ioctl"
    );
    assert!(
        GammaRamp::from_channels(&[0; 8], &[0; 4], &[0; 8]).is_err(),
        "the channels must agree"
    );
}

#[test]
fn switching_an_output_off_is_a_power_operation_and_keeps_the_mode() {
    let on = OutputState::lit(FULL_HD, Scale::integral(1), gnoblin_types::Orientation::Normal);
    let off = OutputState::dark(FULL_HD, Scale::integral(1), gnoblin_types::Orientation::Normal);
    let ops = plan::ops(&connector(true), Some(&on), &off).expect("a plan");
    assert_eq!(ops, [Op::Power(Power::Off)]);
}

/// The events a device produces, as the seat receives them, for every kind of input the mapper owns.
#[test]
fn libinput_events_become_every_input_event_a_seat_applies() {
    let mut mapper = Mapper::new();
    mapper.set_screens(vec![Screen {
        id: OutputId::new(1),
        logical: Size::new(1920, 1080),
    }]);
    let device = DeviceId::new(2);
    let events: Vec<InputEvent> = [
        DeviceEvent::Motion { dx: 5.0, dy: -3.0 },
        DeviceEvent::Absolute { x: 0.5, y: 0.5 },
        DeviceEvent::Button {
            code: 0x110,
            pressed: true,
        },
        DeviceEvent::Key {
            code: 30,
            pressed: true,
        },
        DeviceEvent::Scroll {
            horizontal: 0.0,
            vertical: 1.0,
        },
        DeviceEvent::Touch {
            slot: 0,
            x: 0.25,
            y: 0.25,
            phase: TouchPhase::Down,
        },
    ]
    .into_iter()
    .flat_map(|event| mapper.event(device, event))
    .collect();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, InputEvent::PointerMotionRelative { dx: 5, dy: -3, .. }))
    );
    assert!(
        events.iter().any(
            |event| matches!(event, InputEvent::PointerMotion { position, .. } if *position == Point::new(960, 540))
        )
    );
    assert!(events.iter().any(|event| matches!(
        event,
        InputEvent::PointerButton {
            button: 0x110,
            pressed: true,
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        InputEvent::Key {
            keycode: 30,
            pressed: true,
            ..
        }
    )));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, InputEvent::PointerAxis { vertical: 120, .. }))
    );
    assert!(events.iter().any(|event| matches!(
        event,
        InputEvent::Touch {
            phase: TouchPhase::Down,
            ..
        }
    )));
}

#[test]
fn a_tablet_stroke_reaches_the_seat_through_the_mapper() {
    let mut mapper = Mapper::new();
    mapper.set_screens(vec![Screen {
        id: OutputId::new(1),
        logical: Size::new(1920, 1080),
    }]);
    let tablet = DeviceId::new(4);
    let pen = TabletTool::pen(9);
    let mut events = Vec::new();
    for event in [
        TabletDeviceEvent::ToolProximity {
            tool: pen,
            inside: true,
            x: 0.5,
            y: 0.5,
            axes: TabletAxes::default(),
        },
        TabletDeviceEvent::ToolTip { tool: pen, down: true },
        TabletDeviceEvent::ToolAxis {
            tool: pen,
            x: 0.6,
            y: 0.5,
            axes: TabletAxes {
                pressure: Some(0.4),
                tilt: Some((5.0, 0.0)),
                ..TabletAxes::default()
            },
        },
        TabletDeviceEvent::ToolButton {
            tool: pen,
            code: 0x14b,
            pressed: true,
        },
        TabletDeviceEvent::ToolTip { tool: pen, down: false },
        TabletDeviceEvent::ToolProximity {
            tool: pen,
            inside: false,
            x: 0.6,
            y: 0.5,
            axes: TabletAxes::default(),
        },
    ] {
        events.extend(mapper.event(tablet, DeviceEvent::Tablet(event)));
    }
    let kinds: Vec<&str> = events
        .iter()
        .map(|event| match event {
            InputEvent::Tablet(TabletEvent::ToolIn { .. }) => "in",
            InputEvent::Tablet(TabletEvent::ToolTip { down: true, .. }) => "down",
            InputEvent::Tablet(TabletEvent::ToolMotion { .. }) => "motion",
            InputEvent::Tablet(TabletEvent::ToolButton { .. }) => "button",
            InputEvent::Tablet(TabletEvent::ToolTip { down: false, .. }) => "up",
            InputEvent::Tablet(TabletEvent::ToolOut { .. }) => "out",
            other => panic!("not a tablet event: {other:?}"),
        })
        .collect();
    assert_eq!(kinds, ["in", "down", "motion", "button", "up", "out"]);
    assert!(
        events
            .iter()
            .all(|event| matches!(event, InputEvent::Tablet(event) if event.device() == tablet)),
        "every event names the tablet it came from"
    );
}

#[test]
fn a_seat_pause_releases_what_the_session_held() {
    let mut mapper = Mapper::new();
    mapper.set_screens(vec![Screen {
        id: OutputId::new(1),
        logical: Size::new(1920, 1080),
    }]);
    let device = DeviceId::new(2);
    let _ = mapper.event(
        device,
        DeviceEvent::Button {
            code: 0x110,
            pressed: true,
        },
    );
    let _ = mapper.event(
        device,
        DeviceEvent::Touch {
            slot: 1,
            x: 0.1,
            y: 0.1,
            phase: TouchPhase::Down,
        },
    );
    let released = mapper.cancel();
    assert!(
        released
            .iter()
            .any(|event| matches!(event, InputEvent::PointerButton { pressed: false, .. }))
    );
    assert!(released.iter().any(|event| matches!(
        event,
        InputEvent::Touch {
            phase: TouchPhase::Cancel,
            ..
        }
    )));
}

#[test]
fn devices_are_numbered_once_and_the_gpu_removal_is_reported_as_the_primary_lost() {
    let mut devices = crate::libinput::devices::Devices::new();
    let info = gnoblin_backend::DeviceInfo::new(DeviceId::VIRTUAL, "keyboard", gnoblin_backend::DeviceKind::Keyboard);
    let first = devices
        .announce("event3", info.clone())
        .expect("a new device is announced");
    assert!(
        devices.announce("event3", info).is_none(),
        "libinput announces a device once"
    );
    assert_eq!(devices.remove("event3"), Some(first.id));

    let node = |path: &str| DrmNode {
        path: path.into(),
        kind: DrmNodeKind::Card,
        boot_vga: false,
        driver: None,
    };
    let mut gpus = Devices::new(vec![node("/dev/dri/card0")], &DeviceOverrideSource::default()).expect("a GPU");
    assert_eq!(
        gpus.removed(std::path::Path::new("/dev/dri/card0")),
        DeviceChange::PrimaryLost
    );
}

/// A libinput device that records the calls made on it, standing in for one the kernel has.
#[derive(Clone, Default)]
struct FakeLibinput(std::rc::Rc<std::cell::RefCell<Vec<crate::libinput::configure::DeviceCall>>>);

impl crate::libinput::configure::Execute for FakeLibinput {
    fn call(
        &mut self,
        call: crate::libinput::configure::DeviceCall,
    ) -> Result<(), gnoblin_backend::input_settings::SettingRefusal> {
        self.0.borrow_mut().push(call);
        Ok(())
    }
}

#[test]
fn a_document_configures_each_device_through_libinput_calls() {
    use crate::libinput::configure::{Configurator, DeviceCall, Profile};
    use gnoblin_backend::input_settings::DeviceBook;
    use gnoblin_backend::{DeviceInfo, DeviceKind};
    use gnoblin_config::{AccelProfile, Inherit, Input, Mouse, Touchpad};

    // Devices are numbered the way the backend numbers them, and configured when they are announced.
    let mut numbering = crate::libinput::devices::Devices::new();
    let mut book: DeviceBook<Configurator<FakeLibinput>> = DeviceBook::new();
    let mut calls = Vec::new();
    for (sysname, kind) in [("event3", DeviceKind::Touchpad), ("event4", DeviceKind::Mouse)] {
        let draft = DeviceInfo::new(DeviceId::VIRTUAL, sysname, kind);
        let info = numbering.announce(sysname, draft).expect("a new device is announced");
        let fake = FakeLibinput::default();
        calls.push(fake.0.clone());
        assert!(book.add(&info, Configurator::new(fake)).is_empty());
    }
    // The section arrives after both devices: a reload, and the first one of the session.
    book.set_input(Input {
        touchpad: Some(Touchpad {
            tap_to_click: Some(Inherit::Value(true)),
            ..Touchpad::default()
        }),
        mouse: Some(Mouse {
            accel_profile: Some(Inherit::Value(AccelProfile::Flat)),
            ..Mouse::default()
        }),
        ..Input::default()
    });
    assert_eq!(
        *calls[0].borrow(),
        [DeviceCall::Tap(true)],
        "the touchpad reads the touchpad section"
    );
    assert_eq!(
        *calls[1].borrow(),
        [DeviceCall::AccelProfile(Profile::Flat)],
        "and the mouse reads the mouse section"
    );
    // An unplugged device is forgotten, so a later reload makes no call on it.
    let gone = numbering.remove("event3").expect("a known device");
    assert!(book.remove(gone).is_some());
    book.set_input(Input::default());
    assert_eq!(calls[0].borrow().len(), 1, "no call reaches a device that went away");
    assert_eq!(
        calls[1].borrow().len(),
        2,
        "the mouse's flat profile is put back to its default"
    );
}

/// Every row of the table, for the DRM column: measured by the tests above through the same code the backend runs,
/// or declared with the words the table must carry.
#[test]
fn the_status_table_holds_for_the_hardware_backend() {
    let table = table();
    let mut measured = ColumnCheck::new(&table, "drm");
    // ColumnCheck on the fake kernel by the tests of this module.
    let supported_on_fake = [
        "outputs.connect",
        "outputs.hotplug",
        "outputs.modes",
        "outputs.remode",
        "outputs.scale",
        "outputs.supersample",
        "render.present",
        "input.pointer",
        "input.relative",
        "input.axis",
        "input.keyboard",
        "input.touch",
        "input.devices",
        "input.tablet_tool",
        "input.focus_loss",
        "control.gamma",
        "control.power",
        "lifecycle.backend_lost",
    ];
    for id in supported_on_fake {
        measured.is(id, CellStatus::Supported);
        assert!(
            table.row(id).cell("drm").note.contains("fake kernel"),
            "`{id}` is proven on the fake kernel, and the table says so: `{}`",
            table.row(id).cell("drm").note
        );
    }
    // What only hardware, or the code's own shape, decides.
    measured.declared("render.gles", "unproven on hardware");
    measured.declared("gpu.dmabuf", "unproven on hardware");
    measured.declared("gpu.views", "unproven on hardware");
    measured.declared("input.touchpad", "unproven on hardware");
    measured.declared("lifecycle.shutdown", "unproven on hardware");
    // Refused for a reason in the code: the scan-out path is GLES and keeps no readable frame.
    measured.declared("render.software", "GLES only");
    measured.declared("render.vulkan", "GLES only");
    measured.declared("render.readback", "scan-out only");
    // Refused for a reason in the code: the toolkit's libinput source drops a pad's events, so a pad is not announced.
    measured.declared("input.tablet_pad", "drops pad events");
    // Not applicable by nature: there is no host and no window.
    for id in [
        "outputs.host_scale",
        "render.pixels_equal",
        "input.shortcut_inhibit",
        "input.pointer_constraints",
        "bridge.clipboard",
        "bridge.window_identity",
        "bridge.dmabuf_present",
        "lifecycle.session_end",
    ] {
        measured.is(id, CellStatus::NotApplicable);
    }
    measured.declared("gpu.footprint", "unproven on hardware");
    // Measured above against fake libinput devices (`a_document_configures_each_device_through_libinput_calls`):
    // the calls are proven, and what a real device answers is not.
    measured.is("input.settings", CellStatus::Supported);
    assert!(
        table
            .row("input.settings")
            .cell("drm")
            .note
            .contains("fake libinput devices"),
        "`input.settings` is proven on fake devices, and the table says so"
    );
    assert!(
        table
            .row("input.settings")
            .cell("drm")
            .note
            .contains("unproven on hardware"),
        "and says what is not proven"
    );
    measured.finish();
}
