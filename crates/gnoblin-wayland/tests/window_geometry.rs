//! A window's node is its window geometry, whatever the buffer its client draws around it.
//!
//! A client that draws a shadow around its window (every GTK and libadwaita client), or a title bar above its main
//! surface (foot), commits a buffer that is larger than the window and says which part of it is the window with
//! `xdg_surface.set_window_geometry`. The window the shell reads from the tree is that rectangle: the size it publishes
//! is the size an `xdg_toplevel.configure` asks for, so asking for it again gives the same window back, and the point
//! the pointer is over is found in the surface's own coordinates, so the client's input region still decides what it
//! reaches. The composer's own cases (position, drawing, popups, Views) are `src/render/surface_tree_tests.rs`, and the
//! pixels a real client draws are `tests/surface_tree.rs`.

mod session;

use std::time::{Duration, Instant};

use gnoblin_backend::InputEvent;
use gnoblin_render::transaction::TreeValue;
use gnoblin_types::{DeviceId, NodeId, Point};
use session::shell::Shell;
use session::{Client, Handle, Session, Window};

/// Where the shell puts the window: its node's rectangle is what the pointer is found in.
const PLACED: (f64, f64, f64, f64) = (100.0, 100.0, 200.0, 150.0);

/// How long a test waits for the compositor.
const PATIENCE: Duration = Duration::from_secs(10);

/// Moves the pointer to a global position, as a device would.
fn motion(session: &Handle, at: Point) {
    let output = session.windows().outputs().first().copied().expect("an output");
    session.control.inject_input(InputEvent::PointerMotion {
        device: DeviceId::VIRTUAL,
        output,
        position: at,
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

/// A window geometry: x, y, width and height in the surface's coordinates.
type Geometry = (i32, i32, i32, i32);

/// A number the compositor last published for `key` on `node`.
fn number(shell: &Shell, node: NodeId, key: &str) -> f64 {
    match shell.published(node, key) {
        Some(TreeValue::Double(value)) => value,
        other => panic!("{key} is published as {other:?}, not as a number"),
    }
}

/// Maps a toplevel the way a client that draws outside its window geometry does: the window geometry
/// `(x, y, width, height)`, in the surface's coordinates, is set with a buffer of `buffer`, and `input` is the surface's
/// input region (`None` leaves the default, the whole surface).
fn map_csd(
    client: &mut Client,
    title: &str,
    geometry: Geometry,
    buffer: (i32, i32),
    input: Option<Geometry>,
) -> Window {
    let window = client.create_window("org.gnoblin.Geometry", title);
    let pixels = client.buffer(buffer.0, buffer.1);
    window
        .xdg_surface
        .set_window_geometry(geometry.0, geometry.1, geometry.2, geometry.3);
    if let Some((x, y, width, height)) = input {
        let region = client.compositor.create_region(&client.qh, ());
        region.add(x, y, width, height);
        window.surface.set_input_region(Some(&region));
    }
    window.surface.attach(Some(&pixels), 0, 0);
    window.surface.damage(0, 0, buffer.0, buffer.1);
    window.surface.commit();
    client.roundtrip();
    window
}

/// Moves the pointer to `at` until the client says it entered a surface, which happens once the compositor has put the
/// window where the shell wrote it.
#[track_caller]
fn enter_at(session: &Handle, client: &mut Client, at: Point, entered: u32) {
    let deadline = Instant::now() + PATIENCE;
    while client.state.pointer_enters < entered {
        assert!(
            Instant::now() < deadline,
            "the pointer never entered the client's surface"
        );
        motion(session, at);
        client.roundtrip();
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_node_a_shell_reads_is_the_window_geometry_for_every_origin_and_buffer() {
    // The geometry the client set and the buffer it committed: at the origin, with a bar above the surface (a negative
    // origin, as foot sets it), with a shadow margin on every side, and a geometry smaller than its buffer.
    let cases: [(Geometry, (i32, i32)); 4] = [
        ((0, 0, 200, 150), (200, 150)),
        ((0, -26, 200, 176), (200, 150)),
        ((12, 12, 200, 150), (224, 174)),
        ((0, 0, 120, 90), (200, 150)),
    ];
    for (geometry, buffer) in cases {
        let (shell, hooks) = Shell::new();
        let session = Session::start_shelled(hooks);
        session.run(move |session| {
            shell.attach();
            let mut client = Client::connect(session.socket());
            let _window = map_csd(&mut client, "Geometry", geometry, buffer, None);
            let node = shell.wait_window_node();
            shell.wait_published(node, "width", &TreeValue::Double(f64::from(geometry.2)));
            shell.wait_published(node, "height", &TreeValue::Double(f64::from(geometry.3)));
        });
    }
}

#[test]
fn a_client_that_never_sets_a_geometry_is_its_buffer() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let window = client.create_window("org.gnoblin.Geometry", "Unset");
        client.attach_buffer(&window.surface, 220, 130);
        let node = shell.wait_window_node();
        shell.wait_published(node, "width", &TreeValue::Double(220.0));
        shell.wait_published(node, "height", &TreeValue::Double(130.0));
    });
}

#[test]
fn a_geometry_committed_later_is_published_again() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let window = map_csd(&mut client, "Later", (12, 12, 200, 150), (224, 174), None);
        let node = shell.wait_window_node();
        shell.wait_published(node, "width", &TreeValue::Double(200.0));

        let pixels = client.buffer(244, 184);
        window.xdg_surface.set_window_geometry(12, 12, 220, 160);
        window.surface.attach(Some(&pixels), 0, 0);
        window.surface.damage(0, 0, 244, 184);
        window.surface.commit();
        client.roundtrip();
        shell.wait_published(node, "width", &TreeValue::Double(220.0));
        shell.wait_published(node, "height", &TreeValue::Double(160.0));
    });
}

#[test]
fn asking_for_the_published_size_five_times_loses_no_bar() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        // Foot: a 26 pixel bar above the 174 pixel main surface, inside the window geometry of 200.
        let window = map_csd(&mut client, "Bar", (0, -26, 300, 200), (300, 174), None);
        let node = shell.wait_window_node();
        shell.wait_published(node, "height", &TreeValue::Double(200.0));
        for trip in 1..=5 {
            let (width, height) = (number(&shell, node, "width"), number(&shell, node, "height"));
            assert_eq!(
                (width, height),
                (300.0, 200.0),
                "trip {trip}: the tree publishes the window geometry"
            );
            // The shell asks for the size the tree holds, and the compositor sends it as the toplevel's size.
            shell.write(
                node,
                &[
                    ("width", TreeValue::Double(width)),
                    ("height", TreeValue::Double(height)),
                ],
            );
            client.roundtrip();
            let (asked_width, asked_height) = (client.state.width, client.state.height);
            assert!(
                asked_width > 0 && asked_height > 0,
                "trip {trip}: the compositor configured the window to a size"
            );
            assert_eq!(
                (asked_width, asked_height),
                (300, 200),
                "trip {trip}: the configure asks for the window geometry's size, the bar included"
            );
            // The client answers as one that draws its own bar: that geometry, and a main surface a bar shorter.
            let pixels = client.buffer(asked_width, asked_height - 26);
            window
                .xdg_surface
                .set_window_geometry(0, -26, asked_width, asked_height);
            window.surface.attach(Some(&pixels), 0, 0);
            window.surface.damage(0, 0, asked_width, asked_height - 26);
            window.surface.commit();
            client.roundtrip();
            // What the commit made of the node is published within a frame or two, and is the same window.
            let until = Instant::now() + Duration::from_millis(300);
            while Instant::now() < until {
                assert_eq!(
                    (number(&shell, node, "width"), number(&shell, node, "height")),
                    (300.0, 200.0),
                    "trip {trip}: the commit did not shrink the window"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        for configure in client
            .state
            .configure_log
            .iter()
            .filter(|configure| configure.width > 0)
        {
            assert_eq!(
                (configure.width, configure.height),
                (300, 200),
                "every configure that named a size asked for the geometry: {:?}",
                client.state.configure_log
            );
        }
    });
}

#[test]
fn the_input_region_decides_what_the_pointer_reaches_in_the_surfaces_own_coordinates() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let (_keyboard, pointer) = client.seat_devices();
        assert!(pointer.is_some(), "the seat offers a pointer");
        // A 12 pixel shadow margin on every side, and an input region that is the window geometry alone.
        let _window = map_csd(
            &mut client,
            "Region",
            (12, 12, 200, 150),
            (224, 174),
            Some((12, 12, 200, 150)),
        );
        let node = shell.wait_window_node();
        place(&shell, node);

        // 5 pixels inside the placed geometry is 17 inside the surface, which the region holds.
        enter_at(session, &mut client, Point { x: 105, y: 105 }, 1);
        assert_eq!(
            client.state.pointer_at,
            Some((17.0, 17.0)),
            "the point is in the surface's coordinates: the geometry's offset more than the point in the window"
        );

        // 6 pixels up and left of the geometry is in the shadow, which the client's region leaves out.
        let deadline = Instant::now() + PATIENCE;
        while client.state.pointer_leaves < 1 {
            assert!(
                Instant::now() < deadline,
                "the pointer never left the surface in the shadow"
            );
            motion(session, Point { x: 94, y: 94 });
            client.roundtrip();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            client.state.pointer_enters, 1,
            "the shadow is not the window and the client's input region excludes it"
        );
    });
}

#[test]
fn a_surface_that_takes_input_over_its_shadow_is_reached_there() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let (_keyboard, pointer) = client.seat_devices();
        assert!(pointer.is_some(), "the seat offers a pointer");
        // The default input region is the whole surface, as a client that gives its shadow a resize border has it.
        let _window = map_csd(&mut client, "Border", (12, 12, 200, 150), (224, 174), None);
        let node = shell.wait_window_node();
        place(&shell, node);

        enter_at(session, &mut client, Point { x: 94, y: 94 }, 1);
        assert_eq!(
            client.state.pointer_at,
            Some((6.0, 6.0)),
            "a point 6 pixels up and left of the geometry is 6 pixels into the surface"
        );
    });
}
