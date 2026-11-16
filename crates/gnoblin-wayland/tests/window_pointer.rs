//! What a client window under the pointer sends the shell, and where a window the shell holds is on the outputs.
//!
//! While a shell holds the tree the compositor does not focus or raise a window on a press: the press is an edge on
//! the window's node and the shell's focus and raise policy follows it. The same ray still reaches the window's own
//! client as `wl_pointer` events. A window the shell places is on an output as a window the compositor places is, so
//! its client hears `wl_surface.enter` and, when the shell hides it, `wl_surface.leave`.

mod session;
mod stock;

use gnoblin_backend::InputEvent;
use gnoblin_render::transaction::TreeValue;
use gnoblin_types::{DeviceId, NodeId, Point};
use gnoblin_wayland::TreeInput;
use session::shell::Shell;
use session::{BTN_LEFT, Client, Handle, Session};

/// Where the shell puts the window: its node's rectangle is what the ray hits.
const PLACED: (f64, f64, f64, f64) = (100.0, 100.0, 200.0, 150.0);

/// A point inside the placed window and one outside it.
const INSIDE: Point = Point { x: 150, y: 140 };
const FURTHER_INSIDE: Point = Point { x: 160, y: 150 };
const OUTSIDE: Point = Point { x: 20, y: 20 };

/// Moves the pointer to a global position, as a device would.
fn motion(session: &Handle, at: Point) {
    let output = session.windows().outputs().first().copied().expect("an output");
    session.control.inject_input(InputEvent::PointerMotion {
        device: DeviceId::VIRTUAL,
        output,
        position: at,
    });
}

/// Presses or releases the left button where the pointer is.
fn button(session: &Handle, pressed: bool) {
    session.control.inject_input(InputEvent::PointerButton {
        device: DeviceId::VIRTUAL,
        button: BTN_LEFT,
        pressed,
    });
}

/// Puts the window's node at [`PLACED`], as the shell's placement does.
fn place(shell: &Shell, node: NodeId) {
    let (x, y, width, height) = PLACED;
    shell.write(
        node,
        &[
            ("x", TreeValue::Double(x)),
            ("y", TreeValue::Double(y)),
            ("width", TreeValue::Double(width)),
            ("height", TreeValue::Double(height)),
        ],
    );
}

#[test]
fn a_client_window_under_the_pointer_sends_the_shell_its_pointer_edges() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let (_keyboard, pointer) = client.seat_devices();
        assert!(pointer.is_some(), "the seat offers a pointer");
        let _window = client.map_window("org.gnoblin.Edges", "Edges");
        let node = shell.wait_window_node();
        place(&shell, node);

        // A ray that misses the window sends it nothing.
        motion(session, OUTSIDE);
        client.roundtrip();
        assert!(
            shell.inputs(node).is_empty(),
            "no edge for a ray that is off the window: {:?}",
            shell.inputs(node)
        );

        // Entering the window is one edge, with the point in the node's own space.
        motion(session, INSIDE);
        shell.wait_input(node, "enter", 1);
        client.wait_until("the pointer to enter the client's surface", |state| {
            state.pointer_enters >= 1
        });
        // Moving within the window is a motion, not a second enter.
        motion(session, FURTHER_INSIDE);
        shell.wait_input(node, "motion", 1);
        button(session, true);
        shell.wait_input(node, "press", 1);
        button(session, false);
        shell.wait_input(node, "release", 1);
        // Leaving it is a leave.
        motion(session, OUTSIDE);
        shell.wait_input(node, "leave", 1);
        client.wait_until("the pointer to leave the client's surface", |state| {
            state.pointer_leaves >= 1
        });

        let inputs = shell.inputs(node);
        let names: Vec<&str> = inputs.iter().map(|input| input.name()).collect();
        assert_eq!(names, ["enter", "motion", "press", "release", "leave"], "{inputs:?}");
        let (left, top) = (PLACED.0, PLACED.1);
        let local = |at: Point| (f64::from(at.x) - left, f64::from(at.y) - top);
        let (x, y) = local(INSIDE);
        assert_eq!(
            inputs[0],
            TreeInput::Enter { x, y },
            "an enter at the point, in the node's space"
        );
        let (x, y) = local(FURTHER_INSIDE);
        assert_eq!(inputs[1], TreeInput::Motion { x, y });
        assert_eq!(inputs[2], TreeInput::Press { x, y, button: BTN_LEFT });
        assert_eq!(inputs[3], TreeInput::Release { x, y, button: BTN_LEFT });
    });
}

#[test]
fn a_press_on_a_client_window_is_the_shells_to_answer_and_focuses_nothing_itself() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let _window = client.map_window("org.gnoblin.Press", "Press");
        let node = shell.wait_window_node();
        place(&shell, node);

        motion(session, INSIDE);
        button(session, true);
        button(session, false);
        shell.wait_input(node, "press", 1);
        shell.wait_input(node, "release", 1);
        client.roundtrip();
        assert!(
            session.windows().windows().all(|window| !window.focused),
            "the compositor leaves focus to the shell: {:#?}",
            session.windows()
        );
    });
}

#[test]
fn with_no_shell_holding_the_tree_a_press_focuses_the_window_and_the_shell_hears_nothing() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        let mut client = Client::connect(session.socket());
        let _window = client.map_window("org.gnoblin.Fallback", "Fallback");
        let geometry = session
            .wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
            .only()
            .expect("the window")
            .geometry;
        let at = Point {
            x: geometry.x + geometry.width as i32 / 2,
            y: geometry.y + geometry.height as i32 / 2,
        };
        motion(session, at);
        button(session, true);
        button(session, false);
        session.wait_for(|observation| observation.only().is_some_and(|window| window.focused));
        client.roundtrip();
        assert_eq!(shell.input_count(), 0, "a shell that holds nothing is sent no edge");
    });
}

#[test]
fn a_new_window_is_on_the_output_while_a_shell_holds_the_tree() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let _window = client.map_window("org.gnoblin.Entered", "Entered");
        let node = shell.wait_window_node();

        client.wait_until("wl_surface.enter for the new window", |state| state.surface_enters >= 1);
        assert_eq!(client.state.surface_leaves, 0, "a new window has not left an output");

        // A window the shell minimises leaves the outputs, and comes back on them when it is shown.
        shell.write(node, &[("minimised", TreeValue::Bool(true))]);
        client.wait_until("wl_surface.leave for the minimised window", |state| {
            state.surface_leaves >= 1
        });
        let entered = client.state.surface_enters;
        shell.write(node, &[("minimised", TreeValue::Bool(false))]);
        client.wait_until("wl_surface.enter when the window is shown again", |state| {
            state.surface_enters > entered
        });
        session.wait_for(|observation| observation.only().is_some_and(|window| window.visible));
    });
}

/// A canvas the shell puts under the window's node, as a frame's resize handle.
const HANDLE: NodeId = NodeId::new(0x7101);
/// A handle that is hidden from the pointer, as a frame's layers are.
const LAYER: NodeId = NodeId::new(0x7102);

#[test]
fn a_node_of_the_shell_outside_the_window_geometry_takes_the_pointer_and_the_client_is_not_sent_it() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let (_keyboard, pointer) = client.seat_devices();
        assert!(pointer.is_some(), "the seat offers a pointer");
        let _window = client.map_window("org.gnoblin.Handles", "Handles");
        let node = shell.wait_window_node();
        place(&shell, node);

        // A resize handle 6 pixels wide on the window's left, outside its geometry: a child of the window's node at a
        // negative x (a view with no source is a node like a canvas to the pointer). A layer over the whole window that
        // lets the pointer through, as a frame's overlay does.
        let canvas = |x: f64, y: f64, width: f64, height: f64| {
            [
                ("x", TreeValue::Double(x)),
                ("y", TreeValue::Double(y)),
                ("width", TreeValue::Double(width)),
                ("height", TreeValue::Double(height)),
            ]
        };
        shell.create("view", HANDLE, Some(node), &canvas(-6.0, 0.0, 6.0, PLACED.3));
        shell.create(
            "view",
            LAYER,
            Some(node),
            &canvas(-6.0, -6.0, PLACED.2 + 12.0, PLACED.3 + 12.0),
        );
        shell.write(LAYER, &[("input", TreeValue::Bool(false))]);

        let on_handle = Point {
            x: PLACED.0 as i32 - 3,
            y: PLACED.1 as i32 + 50,
        };
        motion(session, on_handle);
        shell.wait_input(HANDLE, "enter", 1);
        button(session, true);
        shell.wait_input(HANDLE, "press", 1);
        button(session, false);
        shell.wait_input(HANDLE, "release", 1);
        client.roundtrip();
        assert!(
            shell.inputs(node).is_empty(),
            "the window is not the target: {:?}",
            shell.inputs(node)
        );
        assert_eq!(
            client.state.pointer_enters, 0,
            "the client never saw the pointer: the handle is outside its geometry"
        );
        assert!(
            shell.inputs(LAYER).is_empty(),
            "a layer with no input lets the pointer through and is not hit"
        );

        // Inside the geometry the layer is passed through and the window is the target again.
        motion(session, INSIDE);
        shell.wait_input(node, "enter", 1);
        client.wait_until("the pointer to enter the client's surface", |state| {
            state.pointer_enters >= 1
        });
        assert!(shell.inputs(LAYER).is_empty());
    });
}

#[test]
fn a_press_on_a_shell_node_holds_it_until_the_button_is_released_wherever_the_pointer_goes() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let (_keyboard, pointer) = client.seat_devices();
        assert!(pointer.is_some(), "the seat offers a pointer");
        let _window = client.map_window("org.gnoblin.Grab", "Grab");
        let node = shell.wait_window_node();
        place(&shell, node);
        // The same handle: 6 pixels wide, on the window's left and outside its geometry.
        let (x, y) = (-6.0, 0.0);
        shell.create(
            "view",
            HANDLE,
            Some(node),
            &[
                ("x", TreeValue::Double(x)),
                ("y", TreeValue::Double(y)),
                ("width", TreeValue::Double(6.0)),
                ("height", TreeValue::Double(PLACED.3)),
            ],
        );
        let on_handle = Point {
            x: PLACED.0 as i32 - 3,
            y: PLACED.1 as i32 + 50,
        };
        let local = |at: Point| (f64::from(at.x) - (PLACED.0 + x), f64::from(at.y) - (PLACED.1 + y));

        motion(session, on_handle);
        shell.wait_input(HANDLE, "enter", 1);
        button(session, true);
        shell.wait_input(HANDLE, "press", 1);
        // The pointer leaves the handle, over the desktop and then over the window. The handle hears both moves, in its
        // own space, and neither the window nor its client hears anything while the button is down.
        motion(session, OUTSIDE);
        shell.wait_input(HANDLE, "motion", 1);
        motion(session, INSIDE);
        shell.wait_input(HANDLE, "motion", 2);
        client.roundtrip();
        assert!(
            shell.inputs(node).is_empty(),
            "the window hears nothing while the handle holds the pointer: {:?}",
            shell.inputs(node)
        );
        assert_eq!(client.state.pointer_enters, 0, "and neither does its client");
        button(session, false);
        shell.wait_input(HANDLE, "release", 1);
        // The release lets go, and the ray is given what it is over now: the window.
        shell.wait_input(HANDLE, "leave", 1);
        shell.wait_input(node, "enter", 1);
        client.wait_until("the pointer to enter the client's surface", |state| {
            state.pointer_enters >= 1
        });

        let inputs = shell.inputs(HANDLE);
        let (px, py) = local(on_handle);
        let (ox, oy) = local(OUTSIDE);
        let (ix, iy) = local(INSIDE);
        assert_eq!(
            inputs,
            [
                TreeInput::Enter { x: px, y: py },
                TreeInput::Press {
                    x: px,
                    y: py,
                    button: BTN_LEFT
                },
                TreeInput::Motion { x: ox, y: oy },
                TreeInput::Motion { x: ix, y: iy },
                TreeInput::Release {
                    x: ix,
                    y: iy,
                    button: BTN_LEFT
                },
                TreeInput::Leave,
            ],
            "the handle's edges, with the points in its own space"
        );
    });
}
