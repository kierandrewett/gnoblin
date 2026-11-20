//! Drawing tablets, against real clients: the tool a hand holds over one and the pad beside it.
//!
//! A real `wayland-client` connection binds `zwp_tablet_manager_v2`, a tablet seat, and records every event its tablet,
//! tool and pad objects are told. The events are injected as the contract's [`TabletEvent`]s, which is where every
//! backend's tablet input ends, so what is proven here is the half every backend shares: what the compositor serves
//! from them. The nested translation and the libinput mapper are proven on their own sides of the contract, and
//! `backend_conformance.rs` runs the same stroke through each.
//!
//! The tests are the failure-mode list of `src/protocols/zwp_tablet_v2.rs`, in its order.

use gnoblin_wayland_testkit::session;

use std::thread;
use std::time::Duration;

use gnoblin_backend::{
    DeviceCaps, DeviceInfo, DeviceKind, InputEvent, PadLayout, Stylus, TabletAxes, TabletEvent, TabletTool,
};
use gnoblin_types::{DeviceId, OutputId};
use session::{Client, Handle, Session};
use wayland_client::protocol::wl_pointer;

/// The tablet the tests plug in.
const TABLET: DeviceId = DeviceId::new(7);
/// The pad the tests plug in.
const PAD: DeviceId = DeviceId::new(8);

// ---- the session -----------------------------------------------------------------------------------------------------

fn tablet_device() -> InputEvent {
    let mut info = DeviceInfo::new(TABLET, "Test tablet", DeviceKind::Mouse).with_caps(DeviceCaps::TABLET_TOOL);
    info.vendor = 0x56a;
    info.product = 0x37;
    InputEvent::DeviceAdded { device: info }
}

fn pad_device() -> InputEvent {
    InputEvent::DeviceAdded {
        device: DeviceInfo::new(PAD, "Test pad", DeviceKind::Mouse)
            .with_caps(DeviceCaps::NONE)
            .with_pad(PadLayout {
                buttons: 4,
                rings: 1,
                strips: 1,
            }),
    }
}

fn output_of(session: &Handle) -> OutputId {
    session.windows().outputs().first().copied().expect("an output")
}

fn inject(session: &Handle, events: impl IntoIterator<Item = InputEvent>) {
    for event in events {
        session.control.inject_input(event);
    }
}

// ---- tools -----------------------------------------------------------------------------------------------------------

#[test]
fn the_session_serves_the_tablet_manager() {
    let session = Session::start();
    session.run(|session| {
        let client = Client::connect(session.socket());
        assert!(client.has_global("zwp_tablet_manager_v2"));
    });
}

#[test]
fn a_tablet_is_announced_with_its_name_and_ids() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let _seat = client.bind_tablets();
        inject(session, [tablet_device()]);
        client.wait_log("the tablet is announced", |log| {
            log.iter().any(|line| line == "tablet_added")
        });
        client.wait_log("it has a name and ids", |log| {
            log.iter().any(|line| line == "tablet_name Test tablet")
                && log.iter().any(|line| line == "tablet_id 56a 37")
        });
    });
}

#[test]
fn tool_events_reach_the_client_under_the_tool() {
    let session = Session::start_allowing_focus();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let _seat = client.bind_tablets();
        let _window = client.map_window("org.gnoblin.Pen", "Pen");
        let geometry = session
            .wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
            .only()
            .expect("the window")
            .geometry;
        // A second client with the tablet bound and no window under the pen.
        let mut bystander = Client::connect(session.socket());
        let _bystander_seat = bystander.bind_tablets();
        inject(session, [tablet_device()]);
        client.wait_log("the tablet is announced", |log| {
            log.iter().any(|line| line == "tablet_added")
        });

        let (x, y) = (f64::from(geometry.x) + 40.0, f64::from(geometry.y) + 30.0);
        let mut pen = Stylus::new(TABLET, output_of(session), TabletTool::pen(77));
        inject(
            session,
            pen.enter(
                x,
                y,
                TabletAxes {
                    distance: Some(0.25),
                    ..TabletAxes::default()
                },
            ),
        );
        client.wait_log("the pen came into proximity", |log| {
            log.iter().any(|line| line == "proximity_in")
        });
        assert!(
            client.logged("tool_type Pen") == 1,
            "the tool says what it is: {:?}",
            client.state.tablet_log
        );
        assert!(
            client.logged("tool_serial 0 77") == 1,
            "the tool states its hardware serial: {:?}",
            client.state.tablet_log
        );
        let entered = client.numbers_of("motion", 0).expect("proximity carries a motion");

        inject(session, pen.tip(true));
        inject(
            session,
            pen.move_to(
                x + 5.5,
                y + 2.25,
                TabletAxes {
                    pressure: Some(0.5),
                    tilt: Some((10.0, -5.0)),
                    ..TabletAxes::default()
                },
            ),
        );
        inject(session, pen.button(0x14b, true));
        inject(session, pen.button(0x14b, false));
        inject(session, pen.leave());
        client.wait_log("the stroke ended", |log| log.iter().any(|line| line == "proximity_out"));

        let log = client.state.tablet_log.clone();
        let position_of = |wanted: &str| log.iter().position(|line| line == wanted || line.starts_with(wanted));
        assert!(
            position_of("down") < position_of("motion 45"),
            "the tip came down first: {log:#?}"
        );
        assert!(
            log.iter().any(|line| line == "pressure 32767"),
            "half pressure is half of the 16-bit range: {log:#?}"
        );
        assert!(
            log.iter().any(|line| line == "tilt 10.0 -5.0"),
            "the tilt arrives in degrees: {log:#?}"
        );
        assert!(
            log.iter()
                .any(|line| line.starts_with("button 331 ") && line.ends_with("Pressed")),
            "the barrel button went down: {log:#?}"
        );
        assert!(
            log.iter()
                .any(|line| line.starts_with("button 331 ") && line.ends_with("Released")),
            "and up: {log:#?}"
        );
        let moved = client.numbers_of("motion", 1).expect("the pen moved");
        assert!(
            (moved[0] - entered[0] - 5.5).abs() < 0.01 && (moved[1] - entered[1] - 2.25).abs() < 0.01,
            "the pen moved by what the tablet said, in the window's own coordinates: {entered:?} then {moved:?}"
        );
        assert!(
            log.iter().position(|line| line == "up") < log.iter().position(|line| line == "proximity_out"),
            "the tip lifted before the pen left: {log:#?}"
        );

        bystander.roundtrip();
        assert_eq!(
            bystander.logged("proximity_in"),
            0,
            "a client with no window under the pen sees nothing of it: {:?}",
            bystander.state.tablet_log
        );
        assert_eq!(bystander.logged("motion"), 0);
    });
}

#[test]
fn a_pen_moves_no_pointer() {
    let session = Session::start_allowing_focus();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let _seat = client.bind_tablets();
        let _window = client.map_window("org.gnoblin.Pen", "Pen");
        let geometry = session
            .wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
            .only()
            .expect("the window")
            .geometry;
        inject(session, [tablet_device()]);
        let mut pen = Stylus::new(TABLET, output_of(session), TabletTool::pen(1));
        inject(
            session,
            pen.stroke(&[
                (f64::from(geometry.x) + 20.0, f64::from(geometry.y) + 20.0),
                (f64::from(geometry.x) + 40.0, f64::from(geometry.y) + 40.0),
            ]),
        );
        client.wait_log("the stroke ended", |log| log.iter().any(|line| line == "proximity_out"));
        let pointer: Option<wl_pointer::WlPointer> = client.state.pointer.clone();
        assert!(pointer.is_some(), "the client has a pointer");
        assert_eq!(
            client.state.pointer_buttons.len(),
            0,
            "a pen press is not a pointer button"
        );
        assert_eq!(client.state.pointer_at, None, "and a pen moves no pointer");
    });
}

#[test]
fn a_removed_tablet_ends_its_tools() {
    let session = Session::start_allowing_focus();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let _seat = client.bind_tablets();
        let _window = client.map_window("org.gnoblin.Pen", "Pen");
        let geometry = session
            .wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
            .only()
            .expect("the window")
            .geometry;
        inject(session, [tablet_device()]);
        let mut pen = Stylus::new(TABLET, output_of(session), TabletTool::pen(9));
        inject(
            session,
            pen.enter(
                f64::from(geometry.x) + 30.0,
                f64::from(geometry.y) + 30.0,
                TabletAxes::default(),
            ),
        );
        inject(session, pen.tip(true));
        client.wait_log("the pen is down", |log| log.iter().any(|line| line == "down"));
        inject(session, [InputEvent::DeviceRemoved { id: TABLET }]);
        client.wait_log("the tablet is gone", |log| {
            log.iter().any(|line| line == "tablet_removed")
        });
        let log = client.state.tablet_log.clone();
        let out = log
            .iter()
            .position(|line| line == "proximity_out")
            .expect("the pen left proximity");
        let up = log.iter().position(|line| line == "up").expect("the tip lifted");
        let removed = log
            .iter()
            .position(|line| line == "tablet_removed")
            .expect("the tablet left");
        assert!(
            up < out && out < removed,
            "tip, then proximity, then the tablet: {log:#?}"
        );
    });
}

// ---- pads ------------------------------------------------------------------------------------------------------------

#[test]
fn a_client_that_binds_late_sees_the_pad() {
    let session = Session::start();
    session.run(|session| {
        inject(session, [pad_device()]);
        thread::sleep(Duration::from_millis(100));
        let mut client = Client::connect(session.socket());
        let _seat = client.bind_tablets();
        client.wait_log("the pad is announced to a client that asked after it joined", |log| {
            log.iter().any(|line| line == "pad_done")
        });
        let log = &client.state.tablet_log;
        assert!(log.iter().any(|line| line == "pad_added"), "{log:#?}");
        assert!(
            log.iter().any(|line| line == "pad_buttons 4"),
            "the pad says how many buttons it has: {log:#?}"
        );
        assert!(log.iter().any(|line| line == "pad_has_ring"), "and its ring: {log:#?}");
        assert!(
            log.iter().any(|line| line == "pad_has_strip"),
            "and its strip: {log:#?}"
        );
    });
}

#[test]
fn a_pad_joined_after_the_client_bound_is_announced() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let _seat = client.bind_tablets();
        inject(session, [pad_device()]);
        client.wait_log("the pad is announced", |log| log.iter().any(|line| line == "pad_done"));
    });
}

#[test]
fn a_pad_reaches_the_client_with_the_keyboard() {
    let session = Session::start_allowing_focus();
    session.run(|session| {
        let mut first = Client::connect(session.socket());
        let _first_seat = first.bind_tablets();
        let _first_window = first.map_window("org.gnoblin.First", "First");
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "First"));
        let mut second = Client::connect(session.socket());
        let _second_seat = second.bind_tablets();
        let _second_window = second.map_window("org.gnoblin.Second", "Second");
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Second"));
        inject(session, [pad_device()]);
        first.wait_log("the first client has the pad", |log| {
            log.iter().any(|line| line == "pad_done")
        });
        second.wait_log("the second client has the pad", |log| {
            log.iter().any(|line| line == "pad_done")
        });

        inject(
            session,
            [
                TabletEvent::PadButton {
                    device: PAD,
                    button: 2,
                    pressed: true,
                }
                .into(),
                TabletEvent::PadRing {
                    device: PAD,
                    ring: 0,
                    angle: Some(90.0),
                }
                .into(),
                TabletEvent::PadRing {
                    device: PAD,
                    ring: 0,
                    angle: None,
                }
                .into(),
                TabletEvent::PadStrip {
                    device: PAD,
                    strip: 0,
                    position: Some(1.0),
                }
                .into(),
                TabletEvent::PadButton {
                    device: PAD,
                    button: 2,
                    pressed: false,
                }
                .into(),
            ],
        );
        second.wait_log("the focused client got the pad's events", |log| {
            log.iter().any(|line| line == "pad_button 2 Released")
        });
        let log = second.state.tablet_log.clone();
        let find = |wanted: &str| log.iter().position(|line| line == wanted);
        assert!(
            find("pad_enter") < find("pad_button 2 Pressed"),
            "the pad enters before it presses: {log:#?}"
        );
        assert!(log.iter().any(|line| line == "pad_ring 90.0"), "{log:#?}");
        assert!(
            log.iter().any(|line| line == "pad_ring_stop"),
            "a lifted finger is a stop: {log:#?}"
        );
        assert!(
            log.iter().any(|line| line == "pad_strip 65535"),
            "a strip is a fraction of 16 bits: {log:#?}"
        );
        first.roundtrip();
        assert_eq!(
            first.logged("pad_button "),
            0,
            "a client that is not typing cannot read the pad: {:?}",
            first.state.tablet_log
        );
    });
}

#[test]
fn a_pad_of_a_client_that_left_is_forgotten() {
    let session = Session::start_allowing_focus();
    session.run(|session| {
        {
            let mut gone = Client::connect(session.socket());
            let _seat = gone.bind_tablets();
            let _window = gone.map_window("org.gnoblin.Gone", "Gone");
            session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Gone"));
            inject(session, [pad_device()]);
            gone.wait_log("the pad is announced", |log| log.iter().any(|line| line == "pad_done"));
        }
        session.wait_for(|observation| observation.windows().next().is_none());
        // The client is gone; a pad event and a removal go on without it, and a new client sees the pad.
        inject(
            session,
            [TabletEvent::PadButton {
                device: PAD,
                button: 1,
                pressed: true,
            }
            .into()],
        );
        let mut next = Client::connect(session.socket());
        let _seat = next.bind_tablets();
        next.wait_log("the pad is announced to the next client", |log| {
            log.iter().any(|line| line == "pad_done")
        });
        inject(session, [InputEvent::DeviceRemoved { id: PAD }]);
        next.wait_log("the pad leaves", |log| log.iter().any(|line| line == "pad_removed"));
    });
}
