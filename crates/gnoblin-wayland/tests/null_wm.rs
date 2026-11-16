//! A session with no window-manager provider: the core on its own.
//!
//! `NullWm` is what the compositor runs when nothing registered a provider. It places a window at the origin of the
//! first output at the size the client drew, keeps the order windows arrived in, and has no focus policy. It still
//! applies a request nobody else decides, and a holder attached to the tree decides in its place, as it does over
//! the stock provider.

mod session;

use std::thread;
use std::time::Duration;

use gnoblin_types::{Point, Rect};
use gnoblin_wayland::{SessionSnapshot, TreeHooks};
use session::shell::Shell;
use session::{Client, Handle, Session};

/// The `xdg_toplevel.state` value of a fullscreen configure.
const FULLSCREEN: u32 = 2;

/// What `Client::map_window` draws: a buffer of this size.
const DRAWN: Rect = Rect::new(0, 0, 200, 150);

/// Lets the client hear everything the session has sent for a while, so a change that should not happen has time to
/// show itself.
fn settle(client: &mut Client) {
    for _ in 0..20 {
        client.roundtrip();
        thread::sleep(Duration::from_millis(5));
    }
}

/// Maps two windows from one client and waits until the session shows both drawn.
fn two_windows(session: &Handle) -> (Client, SessionSnapshot) {
    let mut client = Client::connect(session.socket());
    client.map_window("org.gnoblin.First", "First");
    client.map_window("org.gnoblin.Second", "Second");
    let snapshot = session.wait_for(|snapshot| snapshot.len() == 2 && snapshot.windows().all(|window| window.mapped));
    (client, snapshot)
}

#[test]
fn a_session_with_no_provider_places_every_window_at_the_origin_at_the_size_it_drew() {
    let session = Session::start_core_only(TreeHooks::default());
    session.run(|session| {
        let (_client, snapshot) = two_windows(session);
        for window in snapshot.windows() {
            assert_eq!(window.geometry, DRAWN, "{} is at the origin", window.title);
            assert!(window.visible, "{} is drawn", window.title);
        }
    });
}

#[test]
fn the_stock_provider_cascades_the_same_two_windows() {
    // The control for the test above: the placement it asserts is NullWm's, not the headless backend's.
    let session = Session::start();
    session.run(|session| {
        let (_client, snapshot) = two_windows(session);
        let origins: Vec<(i32, i32)> = snapshot
            .windows()
            .map(|window| (window.geometry.x, window.geometry.y))
            .collect();
        assert_ne!(
            origins[0], origins[1],
            "the second window cascades off the first: {origins:?}"
        );
    });
}

#[test]
fn a_session_with_no_provider_has_no_focus_policy() {
    let session = Session::start_core_only(TreeHooks::default());
    session.run(|session| {
        let (mut client, snapshot) = two_windows(session);
        assert_eq!(snapshot.focus(), None, "a new window takes no focus");

        session.click(Point::new(10, 10));
        settle(&mut client);
        assert_eq!(session.windows().focus(), None, "a click focuses nothing either");
    });
}

#[test]
fn a_holder_decides_in_a_session_with_no_provider_and_the_provider_decides_again_on_detach() {
    let (shell, hooks) = Shell::new();
    let session = Session::start_core_only(hooks);
    session.run(move |session| {
        shell.attach();
        let mut client = Client::connect(session.socket());
        let window = client.map_window("org.gnoblin.Held", "Held");
        let node = shell.wait_window_node();

        window.toplevel.set_fullscreen(None);
        client.roundtrip();
        shell.wait_request(node, "fullscreen", 1);
        settle(&mut client);
        assert!(
            !client.state.states.contains(&FULLSCREEN),
            "the holder was asked and has not answered, so nothing was configured"
        );

        shell.detach();
        // Give the frame that reads the detach time to run before the request that follows it.
        thread::sleep(Duration::from_millis(100));
        window.toplevel.set_fullscreen(None);
        client.wait_until("the provider's fullscreen configure", |state| {
            state.states.contains(&FULLSCREEN)
        });
        assert_eq!(
            (client.state.width, client.state.height),
            session.screen(),
            "NullWm covers the output for a fullscreen request nobody else decides"
        );
    });
}
