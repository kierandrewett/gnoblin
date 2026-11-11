//! What a client window under the pointer sends the shell, and where a window the shell holds is on the outputs.
//!
//! While a shell holds the tree the compositor does not focus or raise a window on a press: the press is an edge on
//! the window's node and the shell's focus and raise policy follows it. The same ray still reaches the window's own
//! client as `wl_pointer` events. A window the shell places is on an output as a window the compositor places is, so
//! its client hears `wl_surface.enter` and, when the shell hides it, `wl_surface.leave`.

mod session;

use gnoblin_backend::InputEvent;
use gnoblin_render::transaction::TreeValue;
use gnoblin_types::{NodeId, Point};
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
    session
        .control
        .inject_input(InputEvent::PointerMotion { output, position: at });
}

/// Presses or releases the left button where the pointer is.
fn button(session: &Handle, pressed: bool) {
    session.control.inject_input(InputEvent::PointerButton {
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
