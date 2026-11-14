//! Where a pointer goes when it is over a View: the tri-state `input` of a node over a client's picture.
//!
//! A shell makes a `view` of a client's window node and the pointer over it is the View's. With the default
//! `input_mode` of `forward` the client gets it, in its own surface coordinates, wherever the View is placed, scaled
//! and turned, and the shell is sent nothing for the View. With `capture` the shell is sent the View's edges and the
//! client nothing. With `input` unset (`none`) the View is a miss and the pointer reaches what lies under it.

mod session;

use gnoblin_backend::InputEvent;
use gnoblin_render::transaction::TreeValue;
use gnoblin_types::{DeviceId, NodeId, Point};
use gnoblin_wayland::TreeInput;
use session::shell::Shell;
use session::{BTN_LEFT, Client, Handle, Session};

/// The shell's View of the window.
const VIEW: NodeId = NodeId::new(0x7001);

/// Where the shell puts the window the View shows, and a place nowhere near the View.
const PLACED: (f64, f64) = (100.0, 100.0);
/// The size every test window draws at (`Client::map_window`'s buffer).
const WINDOW: (f64, f64) = (200.0, 150.0);

fn motion(session: &Handle, at: Point) {
    let output = session.windows().outputs().first().copied().expect("an output");
    session.control.inject_input(InputEvent::PointerMotion {
        device: DeviceId::VIRTUAL,
        output,
        position: at,
    });
}

fn button(session: &Handle, pressed: bool) {
    session.control.inject_input(InputEvent::PointerButton {
        device: DeviceId::VIRTUAL,
        button: BTN_LEFT,
        pressed,
    });
}

fn place(shell: &Shell, node: NodeId, at: (f64, f64)) {
    shell.write(
        node,
        &[
            ("x", TreeValue::Double(at.0)),
            ("y", TreeValue::Double(at.1)),
            ("width", TreeValue::Double(WINDOW.0)),
            ("height", TreeValue::Double(WINDOW.1)),
        ],
    );
}

/// A View of `source`, `width` by `height`, with `values` written after it is made.
fn view_of(shell: &Shell, source: NodeId, size: (f64, f64), values: &[(&str, TreeValue)]) {
    let mut all = vec![
        ("source", TreeValue::Node(Some(source))),
        ("width", TreeValue::Double(size.0)),
        ("height", TreeValue::Double(size.1)),
    ];
    all.extend(values.iter().cloned());
    shell.create("view", VIEW, None, &all);
}

fn near(actual: (f64, f64), expected: (f64, f64), what: &str) {
    assert!(
        (actual.0 - expected.0).abs() < 0.02 && (actual.1 - expected.1).abs() < 0.02,
        "{what}: the client's pointer is at {actual:?}, expected {expected:?}"
    );
}

#[test]
fn a_forwarding_view_gives_its_client_the_press_in_surface_coordinates_and_the_shell_nothing() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let (_keyboard, pointer) = client.seat_devices();
        assert!(pointer.is_some(), "the seat offers a pointer");
        let _window = client.map_window("org.gnoblin.Forward", "Forward");
        let node = shell.wait_window_node();
        place(&shell, node, PLACED);
        view_of(
            &shell,
            node,
            WINDOW,
            &[("x", TreeValue::Double(500.0)), ("y", TreeValue::Double(300.0))],
        );

        // The View is the default, `forward`: a point 50 right and 40 down in it is the same point of the window.
        motion(session, Point { x: 550, y: 340 });
        client.wait_until("the pointer to enter the client through the View", |state| {
            state.pointer_enters >= 1
        });
        near(client.state.pointer_at.expect("an enter"), (50.0, 40.0), "enter");
        button(session, true);
        button(session, false);
        client.wait_until("the press and release through the View", |state| {
            state.pointer_buttons.len() >= 2
        });
        assert_eq!(client.state.pointer_buttons, [(BTN_LEFT, true), (BTN_LEFT, false)]);
        assert!(
            shell.inputs(VIEW).is_empty(),
            "the shell is sent nothing for a forwarding View: {:?}",
            shell.inputs(VIEW)
        );
    });
}

#[test]
fn a_capturing_view_sends_the_shell_its_edges_and_its_client_nothing_until_it_forwards_again() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let (_keyboard, pointer) = client.seat_devices();
        assert!(pointer.is_some());
        let _window = client.map_window("org.gnoblin.Capture", "Capture");
        let node = shell.wait_window_node();
        place(&shell, node, PLACED);
        view_of(
            &shell,
            node,
            WINDOW,
            &[
                ("x", TreeValue::Double(500.0)),
                ("y", TreeValue::Double(300.0)),
                ("input_mode", TreeValue::Text("capture".to_owned())),
            ],
        );

        motion(session, Point { x: 550, y: 340 });
        shell.wait_input(VIEW, "enter", 1);
        button(session, true);
        shell.wait_input(VIEW, "press", 1);
        button(session, false);
        shell.wait_input(VIEW, "release", 1);
        client.roundtrip();
        assert_eq!(
            shell.inputs(VIEW)[1],
            TreeInput::Press {
                x: 50.0,
                y: 40.0,
                button: BTN_LEFT
            },
            "the press is in the View's own plane"
        );
        assert_eq!(client.state.pointer_enters, 0, "the client's pointer never entered");
        assert!(client.state.pointer_buttons.is_empty(), "the client got no button");

        // Back to `forward`: the next press at the same place is the client's, and the View's edges stop.
        shell.write(VIEW, &[("input_mode", TreeValue::Text("forward".to_owned()))]);
        motion(session, Point { x: 551, y: 341 });
        client.wait_until("the pointer to enter the client", |state| state.pointer_enters >= 1);
        button(session, true);
        button(session, false);
        client.wait_until("the client's press", |state| state.pointer_buttons.len() >= 2);
        client.roundtrip();
        let presses = shell
            .inputs(VIEW)
            .iter()
            .filter(|input| input.name() == "press")
            .count();
        assert_eq!(presses, 1, "the shell hears no press of the View once it forwards");
    });
}

#[test]
fn a_view_without_input_lets_the_pointer_reach_the_window_under_it() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut shown = Client::connect(session.socket());
        let (_keyboard, pointer) = shown.seat_devices();
        assert!(pointer.is_some());
        let _shown_window = shown.map_window("org.gnoblin.Shown", "Shown");
        let mut below = Client::connect(session.socket());
        let (_keyboard, pointer) = below.seat_devices();
        assert!(pointer.is_some());
        let _below_window = below.map_window("org.gnoblin.Below", "Below");
        let nodes = shell.wait_window_nodes(2);
        place(&shell, nodes[0], PLACED);
        place(&shell, nodes[1], (500.0, 300.0));
        // The View covers the second window and shows the first.
        view_of(
            &shell,
            nodes[0],
            WINDOW,
            &[("x", TreeValue::Double(500.0)), ("y", TreeValue::Double(300.0))],
        );

        motion(session, Point { x: 550, y: 340 });
        shown.wait_until("the shown client to get the pointer through the View", |state| {
            state.pointer_enters >= 1
        });
        assert_eq!(below.state.pointer_enters, 0);

        shell.write(VIEW, &[("input", TreeValue::Bool(false))]);
        motion(session, Point { x: 551, y: 341 });
        below.wait_until("the window under the View to get the pointer", |state| {
            state.pointer_enters >= 1
        });
        button(session, true);
        button(session, false);
        below.wait_until("the press on the window under the View", |state| {
            state.pointer_buttons.len() >= 2
        });
        shown.roundtrip();
        assert!(shown.state.pointer_buttons.is_empty(), "the shown client got no press");
    });
}

/// A View of the client, 400 by 300, with the 200 by 150 window in its top left quarter and margins right and below
/// (what a View much larger than its client shows), scaled by a half, turned a quarter about its centre and placed at
/// (600, 400). A point of the View's plane (lx, ly) lands on the output at
/// `offset + centre + R * S * (l - centre)`, with `centre = (200, 150)`, `S` a half and `R` a quarter turn, which is
/// `(800 - (ly - 150) / 2, 550 + (lx - 200) / 2)`: the picture's points go the other way, so the client gets the point
/// of its surface the picture shows there.
#[test]
fn a_transformed_view_forwards_the_inverse_transform_of_the_ray_and_its_margins_reach_no_client() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let (_keyboard, pointer) = client.seat_devices();
        assert!(pointer.is_some());
        let _window = client.map_window("org.gnoblin.Transformed", "Transformed");
        let node = shell.wait_window_node();
        place(&shell, node, PLACED);
        view_of(
            &shell,
            node,
            (400.0, 300.0),
            &[
                ("x", TreeValue::Double(600.0)),
                ("y", TreeValue::Double(400.0)),
                ("scale", TreeValue::ListDouble(vec![0.5, 0.5, 1.0])),
                ("rotation", TreeValue::ListDouble(vec![0.0, 0.0, 90.0])),
            ],
        );
        let on_output = |lx: f64, ly: f64| (800.0 - (ly - 150.0) / 2.0, 550.0 + (lx - 200.0) / 2.0);

        // The corner of the surface, the far corner, a point inside, and one that is neither square nor central.
        let mut presses = 0;
        for (surface, what) in [
            ((0.0, 0.0), "the top left corner"),
            ((198.0, 148.0), "the far corner"),
            ((100.0, 70.0), "the middle"),
            ((30.0, 120.0), "a point off the diagonal"),
        ] {
            let (x, y) = on_output(surface.0, surface.1);
            assert!(
                x.fract() == 0.0 && y.fract() == 0.0,
                "{what} is a whole pixel of the output"
            );
            motion(
                session,
                Point {
                    x: x as i32,
                    y: y as i32,
                },
            );
            client.wait_until(what, |state| {
                state
                    .pointer_at
                    .is_some_and(|at| (at.0 - surface.0).abs() < 0.02 && (at.1 - surface.1).abs() < 0.02)
            });
            near(client.state.pointer_at.expect("a point"), surface, what);
            button(session, true);
            button(session, false);
            presses += 1;
            client.wait_until(&format!("the press at {what}"), |state| {
                state.pointer_buttons.len() >= presses * 2
            });
        }
        assert!(
            client
                .state
                .pointer_buttons
                .iter()
                .all(|(button, _)| *button == BTN_LEFT),
            "{:?}",
            client.state.pointer_buttons
        );

        // The margins: a point of the View right of the window, and one below it, are on the View but not on its
        // picture of the client, so the pointer leaves the client and a press reaches nothing.
        let leaves = client.state.pointer_leaves;
        for (lx, ly) in [(300.0, 100.0), (100.0, 250.0)] {
            let (x, y) = on_output(lx, ly);
            motion(
                session,
                Point {
                    x: x as i32,
                    y: y as i32,
                },
            );
            button(session, true);
            button(session, false);
        }
        client.wait_until("the pointer to leave the client in the margin", |state| {
            state.pointer_leaves > leaves
        });
        client.roundtrip();
        assert_eq!(
            client.state.pointer_buttons.len(),
            presses * 2,
            "no press reached the client from a margin of the View"
        );
        assert!(
            shell.inputs(VIEW).is_empty(),
            "the shell is sent nothing for a forwarding View"
        );
    });
}
