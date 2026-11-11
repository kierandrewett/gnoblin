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
