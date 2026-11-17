//! A shell that holds the tree places, restacks and hides the popups and subsurfaces of a window (`placement_override`,
//! `stack` and `visible` on a `surface` node). Every test runs a real client over a real socket and a stand-in shell
//! that writes the nodes the compositor announced, and reads what the client was told (an `xdg_popup.configure`) and
//! which of its surfaces the pointer reached.
//!
//! The failure modes, each a test below:
//!
//! 1. A popup the shell placed is configured at the place the shell chose, after its positioner's own adjustment, and the
//!    place it settled on is written back to the node.
//! 2. A popup goes back where the protocol put it when the shell stops placing it, and when the shell lets go.
//! 3. A popup or subsurface the shell moved, restacked or hid takes the pointer where it is drawn.
//! 4. A hidden popup that holds a grab is dismissed.
//! 5. A locked session ignores the shell's placement, and gives it back at the unlock.

mod stock;

mod session;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use gnoblin_backend::InputEvent;
use gnoblin_render::transaction::TreeValue;
use gnoblin_types::{DeviceId, NodeId, Point};
use session::shell::Shell;
use session::{Client, Handle, Popup, PopupPlace, Session, TIMEOUT, Window};
use wayland_client::protocol::{wl_subcompositor, wl_subsurface, wl_surface};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_surface_v1, ext_session_lock_v1,
};
use wayland_protocols::xdg::shell::client::xdg_positioner;

wayland_client::delegate_noop!(session::App: ignore wl_subcompositor::WlSubcompositor);
wayland_client::delegate_noop!(session::App: ignore wl_subsurface::WlSubsurface);
wayland_client::delegate_noop!(session::App: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1);

/// Whether a lock object told its client `locked`.
type Locked = Arc<AtomicBool>;

impl Dispatch<ext_session_lock_v1::ExtSessionLockV1, Locked> for session::App {
    fn event(
        _: &mut Self,
        _: &ext_session_lock_v1::ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        locked: &Locked,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_v1::Event::Locked = event {
            locked.store(true, Ordering::SeqCst);
        }
    }
}

impl Dispatch<ext_session_lock_surface_v1::ExtSessionLockSurfaceV1, ()> for session::App {
    fn event(
        _: &mut Self,
        surface: &ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure { serial, .. } = event {
            surface.ack_configure(serial);
        }
    }
}

fn number(value: f64) -> TreeValue {
    TreeValue::Double(value)
}

/// Where the window of every test is: its node is written there, and its buffer is 200 by 150.
const WINDOW_AT: (i32, i32) = (100, 100);

/// Maps a window and puts its node at [`WINDOW_AT`], as a shell places a window it was told of.
fn place_window(client: &mut Client, shell: &Shell) -> (Window, NodeId) {
    let window = client.map_window("org.example.Menu", "Menu");
    let node = shell.wait_window_node();
    shell.write(
        node,
        &[
            ("x", number(f64::from(WINDOW_AT.0))),
            ("y", number(f64::from(WINDOW_AT.1))),
        ],
    );
    (window, node)
}

/// Creates a popup of `window` the way a toolkit does, takes its first configure and maps it. `grab` is the serial the
/// popup asks to be grabbed with.
fn window_popup(client: &mut Client, window: &Window, place: &PopupPlace, grab: Option<u32>) -> Popup {
    client.state.popup_placed = None;
    let positioner = client.positioner(place);
    let surface = client.compositor.create_surface(&client.qh, ());
    let xdg_surface = client.wm_base.get_xdg_surface(&surface, &client.qh, ());
    let popup = xdg_surface.get_popup(Some(&window.xdg_surface), &positioner, &client.qh, ());
    positioner.destroy();
    if let Some(serial) = grab {
        popup.grab(&client.seat, serial);
    }
    surface.commit();
    client.wait_until("the popup's first configure", |state| state.popup_placed.is_some());
    client.attach_buffer(&surface, place.size.0, place.size.1);
    Popup {
        surface,
        xdg_surface,
        popup,
    }
}

/// Moves the pointer to a global position and waits for the client to be told it is over `surface` at `local`. The motion
/// is sent again on each try, so a window that is still moving (the window manager's own placement tween) is settled on.
#[track_caller]
fn point_at(
    session: &Handle,
    client: &mut Client,
    at: (i32, i32),
    surface: &wl_surface::WlSurface,
    local: (f64, f64),
    what: &str,
) {
    let output = session.windows().outputs().first().copied().expect("an output");
    let deadline = Instant::now() + TIMEOUT;
    loop {
        session.control.inject_input(InputEvent::PointerMotion {
            device: DeviceId::VIRTUAL,
            output,
            position: Point::new(at.0, at.1),
        });
        client.roundtrip();
        if client.state.pointer_surface.as_ref() == Some(surface) && client.state.pointer_at == Some(local) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: the pointer at {at:?} was never reported over the expected surface at {local:?}; it is on {:?} at {:?}",
            client.state.pointer_surface,
            client.state.pointer_at
        );
        thread::sleep(Duration::from_millis(5));
    }
}

/// Moves the pointer to a global position and waits for the client to be told it is over no surface of its own.
#[track_caller]
fn point_at_nothing(session: &Handle, client: &mut Client, at: (i32, i32), what: &str) {
    let output = session.windows().outputs().first().copied().expect("an output");
    session.control.inject_input(InputEvent::PointerMotion {
        device: DeviceId::VIRTUAL,
        output,
        position: Point::new(at.0, at.1),
    });
    let deadline = Instant::now() + TIMEOUT;
    loop {
        client.roundtrip();
        if client.state.pointer_surface.is_none() {
            return;
        }
        assert!(Instant::now() < deadline, "{what}: the pointer is still on a surface");
        thread::sleep(Duration::from_millis(5));
    }
}

/// Waits until the last popup configure the client got is `placed`.
#[track_caller]
fn wait_placed(client: &mut Client, placed: (i32, i32, i32, i32)) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        client.roundtrip();
        if client.state.popup_placed == Some(placed) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the popup was never configured at {placed:?}; last {:?}",
            client.state.popup_placed
        );
        thread::sleep(Duration::from_millis(5));
    }
}

/// Makes a subsurface of the window's own surface, `size` big, at `at`, drawn at once (desynchronized), and returns it with
/// the surface of its own.
fn subsurface_of(
    client: &mut Client,
    window: &Window,
    at: (i32, i32),
    size: (i32, i32),
) -> (wl_surface::WlSurface, wl_subsurface::WlSubsurface) {
    let subcompositor: wl_subcompositor::WlSubcompositor = client
        .globals
        .bind(&client.qh, 1..=1, ())
        .expect("the session advertises wl_subcompositor");
    let surface = client.compositor.create_surface(&client.qh, ());
    let subsurface = subcompositor.get_subsurface(&surface, &window.surface, &client.qh, ());
    subsurface.set_position(at.0, at.1);
    subsurface.set_desync();
    client.attach_buffer(&surface, size.0, size.1);
    // The position is the parent's state: it applies with the parent's next commit.
    client.attach_buffer(&window.surface, 200, 150);
    (surface, subsurface)
}

/// A popup that asks to be configured again (`xdg_positioner.set_reactive`) is the one the protocol lets the compositor
/// move after its first configure.
fn reactive(place: PopupPlace) -> PopupPlace {
    PopupPlace {
        reactive: true,
        ..place
    }
}

#[test]
fn a_reactive_popup_the_shell_placed_is_configured_there_and_takes_the_pointer_there() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let (window, _) = place_window(&mut client, &shell);
        client.seat_devices();
        let popup = window_popup(&mut client, &window, &reactive(PopupPlace::at(10, 10, (60, 40))), None);
        assert_eq!(
            client.state.popup_placed,
            Some((10, 10, 60, 40)),
            "the protocol's place"
        );
        let node = shell.wait_surface_node("popup");
        shell.wait_published(node, "x", &number(10.0));
        shell.wait_published(node, "y", &number(10.0));
        shell.wait_published(node, "width", &number(60.0));
        point_at(
            session,
            &mut client,
            (115, 115),
            &popup.surface,
            (5.0, 5.0),
            "the control: over the popup",
        );

        shell.write(
            node,
            &[
                ("placement_override", TreeValue::Bool(true)),
                ("x", number(50.0)),
                ("y", number(20.0)),
            ],
        );
        wait_placed(&mut client, (50, 20, 60, 40));
        // The popup is at (150, 120) to (210, 160) now: the pointer finds it there and the window where it was.
        point_at(
            session,
            &mut client,
            (160, 130),
            &popup.surface,
            (10.0, 10.0),
            "over the placed popup",
        );
        point_at(
            session,
            &mut client,
            (115, 115),
            &window.surface,
            (15.0, 15.0),
            "where it was is the window's",
        );
    });
}

#[test]
fn a_popup_that_is_not_reactive_is_moved_without_being_told_because_the_protocol_forbids_a_configure() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let (window, _) = place_window(&mut client, &shell);
        client.seat_devices();
        let popup = window_popup(&mut client, &window, &PopupPlace::at(10, 10, (60, 40)), None);
        let node = shell.wait_surface_node("popup");
        shell.wait_published(node, "x", &number(10.0));
        let configures = client.state.configures;

        shell.write(
            node,
            &[
                ("placement_override", TreeValue::Bool(true)),
                ("x", number(50.0)),
                ("y", number(20.0)),
            ],
        );
        // The popup is drawn and picked at (150, 120), and its client is not sent a configure it may not receive.
        point_at(
            session,
            &mut client,
            (160, 130),
            &popup.surface,
            (10.0, 10.0),
            "over the placed popup",
        );
        point_at(
            session,
            &mut client,
            (115, 115),
            &window.surface,
            (15.0, 15.0),
            "where it was is the window's",
        );
        client.roundtrip();
        assert_eq!(
            client.state.popup_placed,
            Some((10, 10, 60, 40)),
            "the client's configure is the protocol's"
        );
        assert_eq!(client.state.configures, configures, "no configure reached the client");
        shell.wait_published(node, "x", &number(50.0));

        // The shell stops placing it: it is where the protocol put it again.
        shell.write(node, &[("placement_override", TreeValue::Bool(false))]);
        shell.wait_published(node, "x", &number(10.0));
        point_at(
            session,
            &mut client,
            (115, 115),
            &popup.surface,
            (5.0, 5.0),
            "back at the protocol's place",
        );

        // The shell lets go of the tree: the same.
        shell.write(
            node,
            &[
                ("placement_override", TreeValue::Bool(true)),
                ("x", number(50.0)),
                ("y", number(20.0)),
            ],
        );
        point_at(
            session,
            &mut client,
            (160, 130),
            &popup.surface,
            (10.0, 10.0),
            "placed again",
        );
        shell.detach();
        shell.wait_published(node, "placement_override", &TreeValue::Bool(false));
        // The window manager has the window again, and puts it where it likes: the popup is 10 pixels in from its corner.
        let corner = session
            .wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
            .only()
            .expect("one window")
            .geometry;
        point_at(
            session,
            &mut client,
            (corner.x + 15, corner.y + 15),
            &popup.surface,
            (5.0, 5.0),
            "back after the detach",
        );
    });
}

#[test]
fn a_popup_the_shell_placed_is_adjusted_by_its_positioner_and_the_node_says_where_it_settled() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let (window, _) = place_window(&mut client, &shell);
        client.seat_devices();
        let slide = PopupPlace {
            constraint_adjustment: xdg_positioner::ConstraintAdjustment::SlideX,
            ..PopupPlace::at(10, 10, (60, 40))
        };
        let popup = window_popup(&mut client, &window, &reactive(slide), None);
        let node = shell.wait_surface_node("popup");
        shell.wait_published(node, "x", &number(10.0));

        // 1000 puts the popup at 1100 on the 1024 wide output: the client asked to be slid, so it ends against the edge,
        // 1024 - 100 - 60 = 864 from the window's origin, which is where it is configured and where the node says it is.
        shell.write(
            node,
            &[
                ("placement_override", TreeValue::Bool(true)),
                ("x", number(1000.0)),
                ("y", number(20.0)),
            ],
        );
        wait_placed(&mut client, (864, 20, 60, 40));
        shell.wait_published(node, "x", &number(864.0));
        point_at(
            session,
            &mut client,
            (970, 130),
            &popup.surface,
            (6.0, 10.0),
            "the pointer finds it where it settled",
        );

        // A popup that asked for no adjustment is put where the shell said, off the output or not.
        let plain = window_popup(&mut client, &window, &reactive(PopupPlace::at(10, 10, (60, 40))), None);
        let second = shell.wait_surface_nodes("popup", 2)[1];
        shell.wait_published(second, "x", &number(10.0));
        shell.write(
            second,
            &[
                ("placement_override", TreeValue::Bool(true)),
                ("x", number(1000.0)),
                ("y", number(20.0)),
            ],
        );
        wait_placed(&mut client, (1000, 20, 60, 40));
        drop(plain);
    });
}

#[test]
fn a_reactive_popup_goes_back_where_the_protocol_put_it_when_the_shell_stops_placing_it_and_when_it_lets_go() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let (window, _) = place_window(&mut client, &shell);
        let _popup = window_popup(&mut client, &window, &reactive(PopupPlace::at(10, 10, (60, 40))), None);
        let node = shell.wait_surface_node("popup");
        shell.wait_published(node, "x", &number(10.0));
        let place = [
            ("placement_override", TreeValue::Bool(true)),
            ("x", number(50.0)),
            ("y", number(20.0)),
            ("stack", TreeValue::Int(3)),
        ];

        shell.write(node, &place);
        wait_placed(&mut client, (50, 20, 60, 40));
        shell.write(node, &[("placement_override", TreeValue::Bool(false))]);
        wait_placed(&mut client, (10, 10, 60, 40));
        shell.wait_published(node, "x", &number(10.0));

        shell.write(node, &place);
        wait_placed(&mut client, (50, 20, 60, 40));
        shell.detach();
        wait_placed(&mut client, (10, 10, 60, 40));
        shell.wait_published(node, "placement_override", &TreeValue::Bool(false));
        shell.wait_published(node, "stack", &TreeValue::Int(0));
        shell.wait_published(node, "x", &number(10.0));
    });
}

#[test]
fn a_popup_the_shell_hid_takes_no_pointer_and_a_hidden_popup_that_holds_a_grab_is_dismissed() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let (window, _) = place_window(&mut client, &shell);
        client.seat_devices();
        let popup = window_popup(&mut client, &window, &PopupPlace::at(10, 10, (60, 40)), None);
        let node = shell.wait_surface_node("popup");
        shell.wait_published(node, "x", &number(10.0));
        point_at(
            session,
            &mut client,
            (115, 115),
            &popup.surface,
            (5.0, 5.0),
            "the control: over the popup",
        );

        shell.write(node, &[("visible", TreeValue::Bool(false))]);
        point_at(
            session,
            &mut client,
            (116, 116),
            &window.surface,
            (16.0, 16.0),
            "the window shows through",
        );
        assert!(
            !client.state.popup_done,
            "a popup without a grab is only hidden: its client still owns it"
        );
        shell.write(node, &[("visible", TreeValue::Bool(true))]);
        point_at(
            session,
            &mut client,
            (115, 115),
            &popup.surface,
            (5.0, 5.0),
            "and it is back",
        );
        popup.popup.destroy();
        client.roundtrip();

        // A popup that holds a grab is dismissed when it is hidden: a popup nobody can see must not keep the seat.
        session.click(Point::new(150, 150));
        let serial = {
            let deadline = Instant::now() + TIMEOUT;
            loop {
                client.roundtrip();
                if let Some(serial) = client.state.button_serial {
                    break serial;
                }
                assert!(Instant::now() < deadline, "the click never reached the client");
                thread::sleep(Duration::from_millis(5));
            }
        };
        let grabbed = window_popup(&mut client, &window, &PopupPlace::at(10, 10, (60, 40)), Some(serial));
        let node = shell.wait_surface_nodes("popup", 2)[1];
        shell.wait_published(node, "x", &number(10.0));
        assert!(!client.state.popup_done);
        shell.write(node, &[("visible", TreeValue::Bool(false))]);
        client.wait_until("the hidden popup's dismissal", |state| state.popup_done);
        drop(grabbed);
    });
}

#[test]
fn a_subsurface_the_shell_placed_takes_the_pointer_where_the_shell_put_it_and_its_client_is_not_told() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let (window, _) = place_window(&mut client, &shell);
        client.seat_devices();
        let (child, _subsurface) = subsurface_of(&mut client, &window, (10, 10), (20, 20));
        let node = shell.wait_surface_node("subsurface");
        shell.wait_published(node, "x", &number(10.0));
        shell.wait_published(node, "width", &number(20.0));
        point_at(
            session,
            &mut client,
            (115, 115),
            &child,
            (5.0, 5.0),
            "the control: over the subsurface",
        );

        shell.write(
            node,
            &[
                ("placement_override", TreeValue::Bool(true)),
                ("x", number(100.0)),
                ("y", number(60.0)),
            ],
        );
        // Drawn at (200, 160) to (220, 180) now. The client has no event for it, and its own position is unchanged.
        point_at(
            session,
            &mut client,
            (205, 165),
            &child,
            (5.0, 5.0),
            "over the moved subsurface",
        );
        point_at(
            session,
            &mut client,
            (115, 115),
            &window.surface,
            (15.0, 15.0),
            "where it was is the window's",
        );

        // The client moves it itself: the shell's place stands while it holds the override.
        _subsurface.set_position(30, 30);
        client.attach_buffer(&window.surface, 200, 150);
        point_at(
            session,
            &mut client,
            (205, 166),
            &child,
            (5.0, 6.0),
            "the shell's place wins",
        );
        point_at(
            session,
            &mut client,
            (135, 135),
            &window.surface,
            (35.0, 35.0),
            "and the client's is not where it is",
        );

        // Writing false shows it where the client put it, which is where it last asked.
        shell.write(node, &[("placement_override", TreeValue::Bool(false))]);
        point_at(
            session,
            &mut client,
            (135, 135),
            &child,
            (5.0, 5.0),
            "back at the client's place",
        );
    });
}

#[test]
fn a_subsurface_the_shell_restacked_takes_the_pointer_over_its_sibling_and_a_hidden_one_takes_none() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let (window, _) = place_window(&mut client, &shell);
        client.seat_devices();
        let (lower, _first) = subsurface_of(&mut client, &window, (10, 10), (30, 30));
        let (upper, _second) = subsurface_of(&mut client, &window, (10, 10), (30, 30));
        let nodes = shell.wait_surface_nodes("subsurface", 2);
        point_at(
            session,
            &mut client,
            (115, 115),
            &upper,
            (5.0, 5.0),
            "the control: the later subsurface is on top",
        );

        shell.write(
            nodes[0],
            &[
                ("placement_override", TreeValue::Bool(true)),
                ("stack", TreeValue::Int(1)),
            ],
        );
        point_at(
            session,
            &mut client,
            (116, 116),
            &lower,
            (6.0, 6.0),
            "stack 1 puts the first over the second",
        );

        shell.write(nodes[0], &[("visible", TreeValue::Bool(false))]);
        point_at(
            session,
            &mut client,
            (117, 117),
            &upper,
            (7.0, 7.0),
            "hidden, it takes nothing",
        );
        shell.write(nodes[1], &[("visible", TreeValue::Bool(false))]);
        point_at(
            session,
            &mut client,
            (118, 118),
            &window.surface,
            (18.0, 18.0),
            "with both hidden the window is under the pointer",
        );
    });
}

#[test]
fn a_subsurface_the_shell_moved_outside_its_window_takes_the_pointer_there() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let (window, _) = place_window(&mut client, &shell);
        client.seat_devices();
        let (child, _subsurface) = subsurface_of(&mut client, &window, (10, 10), (20, 20));
        let node = shell.wait_surface_node("subsurface");
        shell.wait_published(node, "x", &number(10.0));
        shell.write(
            node,
            &[
                ("placement_override", TreeValue::Bool(true)),
                ("x", number(300.0)),
                ("y", number(10.0)),
            ],
        );
        // The window spans (100, 100) to (300, 250); the subsurface is at (400, 110), past its right edge.
        point_at(
            session,
            &mut client,
            (405, 115),
            &child,
            (5.0, 5.0),
            "over the subsurface past the window's edge",
        );
        point_at_nothing(session, &mut client, (500, 500), "beside it there is nothing");
    });
}

#[test]
fn a_locked_session_ignores_the_shells_placement_and_gives_it_back_at_the_unlock() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let (window, _) = place_window(&mut client, &shell);
        let _popup = window_popup(&mut client, &window, &reactive(PopupPlace::at(10, 10, (60, 40))), None);
        let node = shell.wait_surface_node("popup");
        shell.wait_published(node, "x", &number(10.0));
        shell.write(
            node,
            &[
                ("placement_override", TreeValue::Bool(true)),
                ("x", number(50.0)),
                ("y", number(20.0)),
            ],
        );
        wait_placed(&mut client, (50, 20, 60, 40));

        // A locker takes the session: the overrides stop counting, the popup is configured where the protocol puts it, and
        // the node says so.
        let mut locker = Client::connect(session.socket());
        let manager: ext_session_lock_manager_v1::ExtSessionLockManagerV1 = locker
            .globals
            .bind(&locker.qh, 1..=1, ())
            .expect("the session advertises ext_session_lock_manager_v1");
        let locked: Locked = Arc::default();
        let lock = manager.lock(&locker.qh, Arc::clone(&locked));
        locker.roundtrip();
        wait_placed(&mut client, (10, 10, 60, 40));
        shell.wait_published(node, "x", &number(10.0));

        // The lock covers the output, and the owner unlocks. The shell's override is still on, and counts again: it writes
        // the place it wants once more, since the node held the protocol's while the session was locked.
        let surface = locker.compositor.create_surface(&locker.qh, ());
        let lock_surface = lock.get_lock_surface(&surface, &locker.output, &locker.qh, ());
        locker.roundtrip();
        locker.attach_buffer(&surface, 1024, 768);
        let deadline = Instant::now() + TIMEOUT;
        while !locked.load(Ordering::SeqCst) {
            locker.roundtrip();
            assert!(Instant::now() < deadline, "the session never reported itself locked");
            thread::sleep(Duration::from_millis(5));
        }
        lock.unlock_and_destroy();
        locker.roundtrip();
        drop(lock_surface);
        shell.write(node, &[("x", number(50.0)), ("y", number(20.0))]);
        wait_placed(&mut client, (50, 20, 60, 40));
    });
}
