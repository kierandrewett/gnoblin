//! A client's or a taskbar's request of a toplevel, against a session with and without a shell.
//!
//! With a shell holding the tree the window manager does not act on a request: it becomes an event on the window's
//! node and nothing changes until the shell answers by writing the node (`maximised`, `fullscreen`, `minimised`,
//! `width`, `height`) or calling `close`. The compositor turns those writes into the client's configure. With no
//! shell the window manager decides, as it always has. Each test asserts on both ends: what the shell was sent, and
//! what the client was configured to.

mod session;

use std::thread;
use std::time::{Duration, Instant};

use gnoblin_render::transaction::TreeValue;
use gnoblin_wayland::WindowChange;
use session::shell::Shell;
use session::{Client, Session, TIMEOUT};
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_manager_v1;

/// The `xdg_toplevel.state` values a configure carries.
const MAXIMIZED: u32 = 1;
const FULLSCREEN: u32 = 2;

/// The state values `zwlr_foreign_toplevel_handle_v1.state` carries.
const FOREIGN_MINIMISED: u8 = 1;

/// Lets the client hear everything the session has sent for a while, so a change that should not happen has time to
/// show itself.
fn settle(client: &mut Client) {
    for _ in 0..20 {
        client.roundtrip();
        thread::sleep(Duration::from_millis(5));
    }
}

/// Binds the taskbar's manager and waits for it to know `count` windows.
fn taskbar_with(handle: &session::Handle, count: usize) -> Client {
    let mut taskbar = Client::connect(handle.privileged_socket());
    let _manager: zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1 = taskbar
        .globals
        .bind(&taskbar.qh, 1..=3, ())
        .expect("the privileged connection sees zwlr_foreign_toplevel_manager_v1");
    taskbar.wait_until(&format!("{count} toplevels"), |state| {
        state.toplevels.len() == count && state.toplevels.values().all(|toplevel| toplevel.dones >= 1)
    });
    taskbar
}

/// The handle id of the window titled `title`.
fn handle_of(taskbar: &Client, title: &str) -> u32 {
    taskbar
        .state
        .toplevels
        .iter()
        .find(|(_, toplevel)| toplevel.title == title)
        .map(|(id, _)| *id)
        .unwrap_or_else(|| panic!("no toplevel titled {title}: {:#?}", taskbar.state.toplevels))
}

#[test]
fn a_client_maximise_is_a_request_and_the_shells_answer_is_the_configure() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.gnoblin.Max", "Max");
        let node = shell.wait_window_node();
        settle(&mut client);
        let (width, height) = (client.state.width, client.state.height);

        window.toplevel.set_maximized();
        client.roundtrip();
        shell.wait_request(node, "maximize", 1);
        settle(&mut client);
        assert!(
            !client.state.states.contains(&MAXIMIZED),
            "nothing maximises the window until the shell answers: {:?}",
            client.state.states
        );
        assert_eq!((client.state.width, client.state.height), (width, height));

        shell.write(
            node,
            &[
                ("maximised", TreeValue::Bool(true)),
                ("width", TreeValue::Double(640.0)),
                ("height", TreeValue::Double(480.0)),
            ],
        );
        client.wait_until("the maximised configure", |state| {
            state.states.contains(&MAXIMIZED) && state.width == 640 && state.height == 480
        });

        window.toplevel.unset_maximized();
        client.roundtrip();
        shell.wait_request(node, "unmaximize", 1);
        shell.write(
            node,
            &[
                ("maximised", TreeValue::Bool(false)),
                ("width", TreeValue::Double(200.0)),
                ("height", TreeValue::Double(150.0)),
            ],
        );
        client.wait_until("the restored configure", |state| {
            !state.states.contains(&MAXIMIZED) && state.width == 200 && state.height == 150
        });
    });
}

#[test]
fn a_client_fullscreen_is_a_request_and_the_shells_answer_is_the_configure() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.gnoblin.Full", "Full");
        let node = shell.wait_window_node();

        window.toplevel.set_fullscreen(None);
        client.roundtrip();
        shell.wait_request(node, "fullscreen", 1);
        settle(&mut client);
        assert!(
            !client.state.states.contains(&FULLSCREEN),
            "nothing fullscreens the window until the shell answers: {:?}",
            client.state.states
        );

        shell.write(
            node,
            &[
                ("fullscreen", TreeValue::Bool(true)),
                ("width", TreeValue::Double(1024.0)),
                ("height", TreeValue::Double(768.0)),
            ],
        );
        client.wait_until("the fullscreen configure", |state| {
            state.states.contains(&FULLSCREEN) && state.width == 1024 && state.height == 768
        });

        window.toplevel.unset_fullscreen();
        client.roundtrip();
        shell.wait_request(node, "unfullscreen", 1);
        shell.write(node, &[("fullscreen", TreeValue::Bool(false))]);
        client.wait_until("the configure that leaves fullscreen", |state| {
            !state.states.contains(&FULLSCREEN)
        });
    });
}

#[test]
fn a_shell_that_refuses_leaves_the_window_as_it_was() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.gnoblin.Refused", "Refused");
        let node = shell.wait_window_node();
        settle(&mut client);
        let before = (client.state.width, client.state.height, client.state.states.clone());

        window.toplevel.set_maximized();
        window.toplevel.set_fullscreen(None);
        window.toplevel.set_minimized();
        client.roundtrip();
        shell.wait_request(node, "maximize", 1);
        shell.wait_request(node, "fullscreen", 1);
        shell.wait_request(node, "minimize", 1);
        settle(&mut client);

        assert_eq!(
            (client.state.width, client.state.height, client.state.states.clone()),
            before,
            "the shell answered nothing, so no configure changed the window"
        );
    });
}

#[test]
fn a_client_minimise_is_a_request_and_the_shell_hides_the_window() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.gnoblin.Hide", "Hide");
        let node = shell.wait_window_node();
        let mut taskbar = taskbar_with(session, 1);

        window.toplevel.set_minimized();
        client.roundtrip();
        shell.wait_request(node, "minimize", 1);
        settle(&mut taskbar);
        assert!(
            !taskbar
                .state
                .toplevels
                .values()
                .any(|toplevel| toplevel.states.contains(&FOREIGN_MINIMISED)),
            "the window is not minimised until the shell says so"
        );

        shell.write(node, &[("minimised", TreeValue::Bool(true))]);
        taskbar.wait_until("the minimised state", |state| {
            state
                .toplevels
                .values()
                .any(|toplevel| toplevel.states.contains(&FOREIGN_MINIMISED) && toplevel.outputs.is_empty())
        });

        shell.write(node, &[("minimised", TreeValue::Bool(false))]);
        taskbar.wait_until("the restored state", |state| {
            state
                .toplevels
                .values()
                .any(|toplevel| !toplevel.states.contains(&FOREIGN_MINIMISED) && !toplevel.outputs.is_empty())
        });
    });
}

#[test]
fn every_taskbar_request_is_a_request_to_the_shell_and_close_waits_for_the_shell() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let _window = client.map_window("org.gnoblin.Bar", "Bar");
        let node = shell.wait_window_node();
        let mut taskbar = taskbar_with(session, 1);
        let id = handle_of(&taskbar, "Bar");

        {
            let handle = &taskbar.state.handles[&id];
            handle.activate(&taskbar.seat);
            handle.set_maximized();
            handle.unset_maximized();
            handle.set_minimized();
            handle.unset_minimized();
            handle.set_fullscreen(None);
            handle.unset_fullscreen();
            handle.close();
        }
        taskbar.roundtrip();

        shell.wait_request(node, "close", 1);
        assert_eq!(
            shell.requests(node),
            [
                "activate",
                "maximize",
                "unmaximize",
                "minimize",
                "unminimize",
                "fullscreen",
                "unfullscreen",
                "close"
            ],
            "each taskbar request reached the shell, in the order it was made"
        );
        settle(&mut client);
        assert!(
            !client.state.closed,
            "the client is not asked to close until the shell calls `close`"
        );

        shell.call(node, "close");
        let deadline = Instant::now() + TIMEOUT;
        while !client.state.closed {
            client.roundtrip();
            assert!(
                Instant::now() < deadline,
                "the shell's `close` never reached the client"
            );
            thread::sleep(Duration::from_millis(5));
        }
    });
}

#[test]
fn with_no_shell_a_client_fullscreen_and_minimise_are_applied_by_the_window_manager() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.gnoblin.Direct", "Direct");
        session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));

        window.toplevel.set_fullscreen(None);
        client.wait_until("the fullscreen configure", |state| state.states.contains(&FULLSCREEN));
        assert_eq!(
            (client.state.width, client.state.height),
            session.screen(),
            "a fullscreen window covers the output"
        );
        window.toplevel.unset_fullscreen();
        client.wait_until("the configure that leaves fullscreen", |state| {
            !state.states.contains(&FULLSCREEN)
        });

        window.toplevel.set_minimized();
        client.roundtrip();
        session.wait_for(|observation| observation.only().is_some_and(|window| !window.visible));
    });
}

#[test]
fn a_shell_that_lets_go_of_the_tree_gives_requests_back_to_the_window_manager() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.gnoblin.Returned", "Returned");
        let node = shell.wait_window_node();

        window.toplevel.set_fullscreen(None);
        client.roundtrip();
        shell.wait_request(node, "fullscreen", 1);
        settle(&mut client);
        assert!(
            !client.state.states.contains(&FULLSCREEN),
            "the shell holds the request"
        );
        window.toplevel.unset_fullscreen();
        client.roundtrip();
        shell.wait_request(node, "unfullscreen", 1);

        shell.detach();
        // Give the frame that reads the detach time to run before the request that follows it.
        thread::sleep(Duration::from_millis(100));
        window.toplevel.set_fullscreen(None);
        client.wait_until("the window manager's fullscreen configure", |state| {
            state.states.contains(&FULLSCREEN)
        });
    });
}

/// The size a toolkit's window draws at before anyone asks it for another.
const OWN_SIZE: (i32, i32) = (800, 600);

/// The window a toolkit client makes: its size limits first, then a buffer at its own size with the window geometry
/// set to it, which is what Ghostty sent in `gnoblin-2jv2`'s trace.
fn toolkit_window(client: &mut Client, title: &str) -> session::Window {
    let window = client.create_window("org.gnoblin.Toolkit", title);
    window.toplevel.set_min_size(139, 126);
    window.toplevel.set_max_size(0, 0);
    client.draw_configured(&window, OWN_SIZE);
    window
}

/// Runs the session for a while with a client that answers every configure after `since` (an index into its log) by
/// drawing at its size, as a toolkit does, so a configure the compositor sends late has the time to arrive and is
/// answered like the first.
fn settle_drawing(client: &mut Client, window: &session::Window, since: usize) {
    let mut answered = since;
    for _ in 0..40 {
        client.roundtrip();
        // What the client knows when it decides to draw: a configure the commit itself provokes is a new one.
        let known = client.state.configure_log.len();
        if known > answered {
            answered = known;
            client.draw_configured(window, OWN_SIZE);
        }
        thread::sleep(Duration::from_millis(5));
    }
}

/// Fails unless the client was configured to `size`, maximised or not as `maximised` says, by the configures it got
/// after `since` (an index into its log), and was never configured to anything else after that: a later configure with
/// another size or state is the window snapping back.
#[track_caller]
fn assert_configured_for_good(client: &Client, since: usize, maximised: bool, size: (i32, i32)) {
    let log = &client.state.configure_log[since..];
    let arrived = log
        .iter()
        .position(|configure| {
            (configure.width, configure.height) == size && configure.states.contains(&MAXIMIZED) == maximised
        })
        .unwrap_or_else(|| panic!("the client was never configured to {size:?} (maximised {maximised}): {log:#?}"));
    for later in &log[arrived..] {
        assert_eq!(
            ((later.width, later.height), later.states.contains(&MAXIMIZED)),
            (size, maximised),
            "the window was configured again after it reached {size:?} (maximised {maximised}): {:#?}",
            &log[arrived..]
        );
    }
}

/// The window manager maximises on a control request, as `gnoblinctl window maximize` makes, and un-maximises on the
/// next. Neither configure is followed by one that takes the window back to the size it had: `gnoblin-2jv2` saw a
/// maximised toolkit window configured to its old size 40 ms after the client had drawn the maximised one.
#[test]
fn a_window_maximised_by_a_control_request_is_not_configured_back_to_its_old_size() {
    let session = Session::start_allowing_focus();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let window = toolkit_window(&mut client, "Max");
        let id = session
            .wait_for(|observation| observation.titled("Max").is_some_and(|window| window.mapped))
            .titled("Max")
            .expect("the window")
            .id;

        let asked = client.state.configure_log.len();
        assert!(matches!(
            session.control.window_control(id, WindowChange::Maximised(true)),
            Some(Ok(()))
        ));
        client.wait_until("the maximised configure", |state| state.states.contains(&MAXIMIZED));
        settle_drawing(&mut client, &window, asked);
        assert_configured_for_good(&client, asked, true, session.screen());

        let asked = client.state.configure_log.len();
        assert!(matches!(
            session.control.window_control(id, WindowChange::Maximised(false)),
            Some(Ok(()))
        ));
        client.wait_until("the configure that leaves maximise", |state| {
            !state.states.contains(&MAXIMIZED)
        });
        settle_drawing(&mut client, &window, asked);
        assert_configured_for_good(&client, asked, false, OWN_SIZE);
    });
}

/// A toolkit that asks to be maximised itself (`xdg_toplevel.set_maximized`, as a titlebar double click does) is
/// maximised once and stays at the work area, and asking to leave maximise restores the size it had.
#[test]
fn a_window_that_asks_to_be_maximised_is_not_configured_back_to_its_old_size() {
    let session = Session::start_allowing_focus();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let window = toolkit_window(&mut client, "Ask");
        session.wait_for(|observation| observation.titled("Ask").is_some_and(|window| window.mapped));

        let asked = client.state.configure_log.len();
        window.toplevel.set_maximized();
        client.wait_until("the maximised configure", |state| state.states.contains(&MAXIMIZED));
        settle_drawing(&mut client, &window, asked);
        assert_configured_for_good(&client, asked, true, session.screen());

        let asked = client.state.configure_log.len();
        window.toplevel.unset_maximized();
        client.wait_until("the configure that leaves maximise", |state| {
            !state.states.contains(&MAXIMIZED)
        });
        settle_drawing(&mut client, &window, asked);
        assert_configured_for_good(&client, asked, false, OWN_SIZE);
    });
}

/// With a shell holding the tree the same control request is the shell's to answer. It answers as the stock shell does,
/// with the flag and the work area's size in one write, and the window is configured once, to that size.
#[test]
fn a_window_the_shell_maximises_is_not_configured_back_to_its_old_size() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let window = toolkit_window(&mut client, "Shelled");
        let node = shell.wait_window_node();
        let id = session
            .wait_for(|observation| observation.titled("Shelled").is_some_and(|window| window.mapped))
            .titled("Shelled")
            .expect("the window")
            .id;
        let (width, height) = session.screen();

        let asked = client.state.configure_log.len();
        assert!(matches!(
            session.control.window_control(id, WindowChange::Maximised(true)),
            Some(Ok(()))
        ));
        shell.wait_request(node, "maximize", 1);
        shell.write(
            node,
            &[
                ("maximised", TreeValue::Bool(true)),
                ("width", TreeValue::Double(f64::from(width))),
                ("height", TreeValue::Double(f64::from(height))),
            ],
        );
        client.wait_until("the maximised configure", |state| state.states.contains(&MAXIMIZED));
        settle_drawing(&mut client, &window, asked);
        assert_configured_for_good(&client, asked, true, (width, height));

        let asked = client.state.configure_log.len();
        assert!(matches!(
            session.control.window_control(id, WindowChange::Maximised(false)),
            Some(Ok(()))
        ));
        shell.wait_request(node, "unmaximize", 1);
        shell.write(
            node,
            &[
                ("maximised", TreeValue::Bool(false)),
                ("width", TreeValue::Double(f64::from(OWN_SIZE.0))),
                ("height", TreeValue::Double(f64::from(OWN_SIZE.1))),
            ],
        );
        client.wait_until("the configure that leaves maximise", |state| {
            !state.states.contains(&MAXIMIZED)
        });
        settle_drawing(&mut client, &window, asked);
        assert_configured_for_good(&client, asked, false, OWN_SIZE);
    });
}

/// The frame's corner of the only window the session manages: what a surface-local point is shifted by.
fn frame_corner(session: &session::Handle) -> (i32, i32) {
    let geometry = session
        .wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
        .only()
        .expect("one window")
        .geometry;
    (geometry.x, geometry.y)
}

#[test]
fn a_window_menu_request_with_an_issued_serial_reaches_the_shell_at_its_anchor_and_is_not_published_twice() {
    // The failure mode: the menu is asked for, a shell holds the tree, and either nobody hears it (no menu ever
    // opens) or both the shell and an extension subscribed to `menu_requested` open one, stacked.
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.gnoblin.Menu", "Menu");
        let node = shell.wait_window_node();
        let serial = session.click_for_serial(&mut client);
        let (x, y) = frame_corner(session);

        window.toplevel.show_window_menu(&client.seat, serial, 30, 40);
        client.roundtrip();
        shell.wait_request(node, "menu", 1);

        assert_eq!(
            shell.menu_anchors(node),
            [(x + 30, y + 40)],
            "the anchor is the surface point in global coordinates"
        );
        assert!(
            session.windows().menu_requests().is_empty(),
            "a shell that holds the tree draws the menu, so no `menu_requested` goes to a second subscriber: {:?}",
            session.windows().menu_requests()
        );
    });
}

#[test]
fn a_window_menu_request_with_a_serial_nobody_issued_reaches_nobody() {
    // The failure mode: a client opens menus on its own, and a menu takes the keyboard, so it steals it. The serial
    // check holds whether or not a shell is attached.
    let (shell, hooks) = Shell::new();
    let session = Session::start_shelled(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.gnoblin.Menu", "Menu");
        let node = shell.wait_window_node();
        let issued = session.click_for_serial(&mut client);

        // One more than the last serial the client was handed was never handed to it.
        window
            .toplevel
            .show_window_menu(&client.seat, issued.wrapping_add(1_000_003), 30, 40);
        window.toplevel.show_window_menu(&client.seat, 0, 30, 40);
        client.roundtrip();
        settle(&mut client);

        assert!(
            shell.menu_anchors(node).is_empty(),
            "an unissued serial opens no menu: {:?}",
            shell.menu_anchors(node)
        );
        assert!(session.windows().menu_requests().is_empty());
    });
}

#[test]
fn a_window_menu_request_with_no_shell_is_published_for_a_subscriber_as_before() {
    let session = Session::start();
    session.run(move |session| {
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.gnoblin.Menu", "Menu");
        let serial = session.click_for_serial(&mut client);
        let (x, y) = frame_corner(session);

        window.toplevel.show_window_menu(&client.seat, serial, 30, 40);
        client.roundtrip();
        let observation = session.wait_for(|observation| !observation.menu_requests().is_empty());
        let request = &observation.menu_requests()[0];
        assert_eq!((request.anchor().x(), request.anchor().y()), (x + 30, y + 40));
    });
}
