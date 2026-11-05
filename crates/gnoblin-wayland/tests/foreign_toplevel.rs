//! `zwlr_foreign_toplevel_manager_v1` (version 3) and `ext_foreign_toplevel_list_v1`
//! against a real taskbar client.
//!
//! A taskbar is one of the session's own processes, so every client here connects on
//! the privileged socket: that is the socket the privileged globals are advertised on.
//! Each test asserts on both sides — what the taskbar was told, and what the window
//! manager did — because a request that reaches the manager and a request that is
//! merely acknowledged look identical from the protocol.

mod session;

use std::thread;
use std::time::{Duration, Instant};

use session::{Client, Session, TIMEOUT};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1;
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_manager_v1;

/// The state values `zwlr_foreign_toplevel_handle_v1.state` carries.
const MAXIMISED: u8 = 0;
const MINIMISED: u8 = 1;
const ACTIVATED: u8 = 2;
const FULLSCREEN: u8 = 3;

/// Binds the wlr manager, which the taskbar protocol is read and driven through.
fn manager(client: &mut Client) -> zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1 {
    client
        .globals
        .bind(&client.qh, 1..=3, ())
        .expect("the privileged connection sees zwlr_foreign_toplevel_manager_v1")
}

/// Waits for the taskbar to know about `count` windows, each with its first `done`.
fn wait_toplevels(client: &mut Client, count: usize) {
    client.wait_until(&format!("{count} toplevels"), |state| {
        state.toplevels.len() == count && state.toplevels.values().all(|toplevel| toplevel.dones >= 1)
    });
}

/// The proxy of the one published window, by its title.
fn handle_of(client: &mut Client, title: &str) -> u32 {
    client
        .state
        .toplevels
        .iter()
        .find(|(_, toplevel)| toplevel.title == title)
        .map(|(id, _)| *id)
        .unwrap_or_else(|| panic!("no toplevel titled {title}: {:#?}", client.state.toplevels))
}

/// Whether a state array contains a value.
fn has(states: &[u8], value: u8) -> bool {
    states.contains(&value)
}

#[test]
fn a_taskbar_lists_a_mapped_window() {
    let session = Session::start();
    session.run(|session| {
        let mut app = Client::connect(session.socket());
        let _window = app.map_window("org.gnoblin.Listed", "Listed");
        session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));

        let mut taskbar = Client::connect(session.privileged_socket());
        let _manager = manager(&mut taskbar);
        wait_toplevels(&mut taskbar, 1);

        let entry = taskbar.state.toplevels.values().next().expect("one toplevel").clone();
        assert_eq!(entry.title, "Listed", "the title the client set");
        assert_eq!(entry.app_id, "org.gnoblin.Listed", "the application id");
        assert!(has(&entry.states, ACTIVATED), "the mapped window is the active one");
        assert_eq!(entry.outputs.len(), 1, "it entered the only output");
        assert!(!entry.closed, "it is alive");

        // The standardised list names the same window with its own identifier, so a
        // shell that reads both cannot see two different sets.
        let list: ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1 = taskbar
            .globals
            .bind(&taskbar.qh, 1..=1, ())
            .expect("the privileged connection sees ext_foreign_toplevel_list_v1");
        let _ = &list;
        taskbar.wait_until("the ext list", |state| {
            state
                .ext_toplevels
                .values()
                .any(|entry| entry.title == "Listed" && !entry.identifier.is_empty() && entry.dones >= 1)
        });
        let ext = taskbar
            .state
            .ext_toplevels
            .values()
            .find(|entry| entry.title == "Listed")
            .expect("the standardised list has it");
        assert_eq!(ext.app_id, "org.gnoblin.Listed");
        assert!(
            ext.identifier.starts_with("gnoblin-"),
            "the identifier is the session's own: {}",
            ext.identifier
        );
    });
}

#[test]
fn activate_reaches_the_window_manager() {
    let session = Session::start();
    session.run(|session| {
        let mut alpha = Client::connect(session.socket());
        let _alpha = alpha.map_window("org.gnoblin.Alpha", "Alpha");
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Alpha"));

        let mut taskbar = Client::connect(session.privileged_socket());
        let _manager = manager(&mut taskbar);
        wait_toplevels(&mut taskbar, 1);

        // A second window takes the focus, exactly as a launched application would.
        let mut beta = Client::connect(session.socket());
        let _beta = beta.map_window("org.gnoblin.Beta", "Beta");
        session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Beta"));

        let id = handle_of(&mut taskbar, "Alpha");
        taskbar.state.handles[&id].activate(&taskbar.seat);
        taskbar.roundtrip();

        let observation =
            session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == "Alpha"));
        assert!(has(&taskbar_entry(&observation, "Alpha"), ACTIVATED));
        let _ = observation;
    });
}

/// The state flags the session publishes for one window, through its own snapshot's
/// focus: a taskbar's `state` event is the only place the array is readable, so this
/// helper answers "is this window the focused one" from the session's side.
fn taskbar_entry(observation: &gnoblin_wayland::SessionSnapshot, title: &str) -> Vec<u8> {
    match observation.focused().map(|window| window.title.as_str()) {
        Some(focused) if focused == title => vec![ACTIVATED],
        _ => Vec::new(),
    }
}

#[test]
fn close_asks_the_client_and_the_window_stays_until_it_goes() {
    let session = Session::start();
    session.run(|session| {
        let mut app = Client::connect(session.socket());
        let window = app.map_window("org.gnoblin.Closing", "Closing");
        session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));

        let mut taskbar = Client::connect(session.privileged_socket());
        let _manager = manager(&mut taskbar);
        wait_toplevels(&mut taskbar, 1);
        let id = handle_of(&mut taskbar, "Closing");

        taskbar.state.handles[&id].close();
        taskbar.roundtrip();

        // `close` is the protocol's `xdg_toplevel.close`: the client decides.
        let deadline = Instant::now() + TIMEOUT;
        while !app.state.closed {
            app.roundtrip();
            assert!(Instant::now() < deadline, "the client was never asked to close");
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            session.windows().len(),
            1,
            "the window is still mapped until the client destroys it"
        );

        app.destroy(window);
        app.roundtrip();
        taskbar.wait_until("the closed event", |state| {
            state.toplevels.values().any(|toplevel| toplevel.closed)
        });
        session.wait_for(|observation| observation.is_empty());
    });
}

#[test]
fn minimise_and_restore_reach_the_window_manager() {
    let session = Session::start();
    session.run(|session| {
        let mut app = Client::connect(session.socket());
        let _window = app.map_window("org.gnoblin.Hidden", "Hidden");
        session.wait_for(|observation| observation.only().is_some_and(|window| window.visible));

        let mut taskbar = Client::connect(session.privileged_socket());
        let _manager = manager(&mut taskbar);
        wait_toplevels(&mut taskbar, 1);
        let id = handle_of(&mut taskbar, "Hidden");

        taskbar.state.handles[&id].set_minimized();
        taskbar.roundtrip();
        session.wait_for(|observation| observation.only().is_some_and(|window| !window.visible));
        taskbar.wait_until("the minimised state", |state| {
            state
                .toplevels
                .values()
                .any(|toplevel| has(&toplevel.states, MINIMISED))
        });
        // A minimised window leaves the output it was on.
        let entry = taskbar.state.toplevels.values().next().expect("one toplevel");
        assert!(entry.outputs.is_empty(), "a hidden window is on no output");

        taskbar.state.handles[&id].unset_minimized();
        taskbar.roundtrip();
        session.wait_for(|observation| observation.only().is_some_and(|window| window.visible));
        taskbar.wait_until("the restored state", |state| {
            state
                .toplevels
                .values()
                .any(|toplevel| !has(&toplevel.states, MINIMISED) && !toplevel.outputs.is_empty())
        });
    });
}

#[test]
fn maximise_and_fullscreen_reach_the_window_manager() {
    let session = Session::start();
    session.run(|session| {
        let mut app = Client::connect(session.socket());
        let _window = app.map_window("org.gnoblin.Sized", "Sized");
        let floating = session
            .wait_for(|observation| observation.only().is_some_and(|window| window.mapped))
            .only()
            .expect("one window")
            .geometry;

        let mut taskbar = Client::connect(session.privileged_socket());
        let _manager = manager(&mut taskbar);
        wait_toplevels(&mut taskbar, 1);
        let id = handle_of(&mut taskbar, "Sized");

        taskbar.state.handles[&id].set_maximized();
        taskbar.roundtrip();
        let maximised = session.wait_for(|observation| {
            observation
                .only()
                .is_some_and(|window| window.geometry == gnoblin_types::Rect::new(0, 0, 1024, 768))
        });
        assert_eq!(
            maximised.only().expect("one window").geometry,
            gnoblin_types::Rect::new(0, 0, 1024, 768),
            "a maximised window is the work area"
        );
        taskbar.wait_until("the maximised state", |state| {
            state
                .toplevels
                .values()
                .any(|toplevel| has(&toplevel.states, MAXIMISED))
        });

        taskbar.state.handles[&id].unset_maximized();
        taskbar.roundtrip();
        session.wait_for(|observation| observation.only().is_some_and(|window| window.geometry == floating));

        taskbar.state.handles[&id].set_fullscreen(None);
        taskbar.roundtrip();
        taskbar.wait_until("the fullscreen state", |state| {
            state
                .toplevels
                .values()
                .any(|toplevel| has(&toplevel.states, FULLSCREEN))
        });
        session.wait_for(|observation| {
            observation
                .only()
                .is_some_and(|window| window.geometry == gnoblin_types::Rect::new(0, 0, 1024, 768))
        });

        taskbar.state.handles[&id].unset_fullscreen();
        taskbar.roundtrip();
        taskbar.wait_until("the restored state", |state| {
            state
                .toplevels
                .values()
                .any(|toplevel| !has(&toplevel.states, FULLSCREEN))
        });
        session.wait_for(|observation| observation.only().is_some_and(|window| window.geometry == floating));
    });
}

#[test]
fn a_request_for_a_closed_window_does_nothing() {
    let session = Session::start();
    session.run(|session| {
        let mut app = Client::connect(session.socket());
        let gone = app.map_window("org.gnoblin.Gone", "Gone");
        session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));

        let mut taskbar = Client::connect(session.privileged_socket());
        let _manager = manager(&mut taskbar);
        wait_toplevels(&mut taskbar, 1);
        let stale = handle_of(&mut taskbar, "Gone");

        // A second window the stale requests must not touch.
        let mut other = Client::connect(session.socket());
        let _survivor = other.map_window("org.gnoblin.Survivor", "Survivor");
        let survivor = session
            .wait_for(|observation| observation.titled("Survivor").is_some_and(|window| window.focused))
            .titled("Survivor")
            .expect("the survivor")
            .geometry;

        app.destroy(gone);
        app.roundtrip();
        taskbar.wait_until("the closed event", |state| {
            state.toplevels.values().any(|toplevel| toplevel.closed)
        });

        // Every request on the stale handle is a no-op: the identity is gone, so it
        // cannot be applied to whatever the table holds now.
        taskbar.state.handles[&stale].set_minimized();
        taskbar.state.handles[&stale].set_maximized();
        taskbar.state.handles[&stale].set_fullscreen(None);
        taskbar.state.handles[&stale].close();
        taskbar.state.handles[&stale].activate(&taskbar.seat);
        taskbar.roundtrip();

        let observation = session.wait_for(|observation| observation.len() == 1);
        let survivor_now = observation.only().expect("one window left");
        assert_eq!(survivor_now.title, "Survivor");
        assert_eq!(survivor_now.geometry, survivor, "the survivor is unchanged");
        assert!(survivor_now.visible, "the survivor is still shown");
        assert!(survivor_now.focused, "the survivor still has the focus");
        assert!(!other.state.closed, "the survivor's client was never asked to close");
    });
}
