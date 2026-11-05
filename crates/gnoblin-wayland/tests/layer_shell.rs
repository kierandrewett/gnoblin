//! `zwlr_layer_shell_v1` against a real client.
//!
//! Each test connects a real `wayland-client` — a bar on the privileged socket — and
//! asserts on what the client was configured with and on what the session published,
//! because a bar's contract is its configure size and the work area it leaves behind.
//! The failure modes are the ones at the top of `protocols::layer_shell`.

mod session;

use std::thread;
use std::time::{Duration, Instant};

use session::{Client, LayerWindow, Session, TIMEOUT};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

/// Binds the layer shell, which only the privileged socket advertises.
fn shell(client: &mut Client) -> zwlr_layer_shell_v1::ZwlrLayerShellV1 {
    client
        .globals
        .bind(&client.qh, 1..=5, ())
        .expect("the privileged connection sees zwlr_layer_shell_v1")
}

/// Commits a bar the way a shell does: attributes first, then the first commit.
fn commit_bar(
    client: &mut Client,
    bar: &LayerWindow,
    size: (u32, u32),
    anchor: zwlr_layer_surface_v1::Anchor,
    zone: i32,
) {
    bar.layer_surface.set_size(size.0, size.1);
    bar.layer_surface.set_anchor(anchor);
    bar.layer_surface.set_exclusive_zone(zone);
    bar.layer_surface
        .set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::None);
    bar.surface.commit();
    client.roundtrip();
}

/// Waits for the client's last configure to carry a size.
fn wait_configure(client: &mut Client, size: (u32, u32)) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        client.roundtrip();
        if client.state.layer_size == Some(size) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the bar was never configured {}x{}; last {:?}",
            size.0,
            size.1,
            client.state.layer_size
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_bar_is_configured_at_its_anchor_and_size() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::connect(session.privileged_socket());
        let shell = shell(&mut client);
        let mut bar = client.layer_surface(&shell, "org.gnoblin.TestBar", zwlr_layer_shell_v1::Layer::Top);
        commit_bar(
            &mut client,
            &bar,
            (0, 30),
            zwlr_layer_surface_v1::Anchor::Top
                | zwlr_layer_surface_v1::Anchor::Left
                | zwlr_layer_surface_v1::Anchor::Right,
            30,
        );
        // Anchored to both horizontal edges with a zero width: the compositor decides
        // the width, which is the output's.
        wait_configure(&mut client, (1024, 30));
        bar.ack(&mut client);
        client.attach_buffer(&bar.surface, 1024, 30);
        bar.layer_surface.set_exclusive_zone(30);

        let observation = session.wait_for(|observation| {
            observation
                .layer("org.gnoblin.TestBar")
                .is_some_and(|layer| layer.mapped)
        });
        let layer = observation.layer("org.gnoblin.TestBar").expect("the bar is published");
        assert_eq!(layer.rect, gnoblin_types::Rect::new(0, 0, 1024, 30), "the bar's strip");
        assert_eq!(layer.layer, "top", "the layer it asked for");
        assert_eq!(layer.exclusive, 30, "the zone it reserves");
    });
}

#[test]
fn a_bar_shrinks_the_work_area_a_maximised_window_gets() {
    let session = Session::start();
    session.run(|session| {
        let mut shell_client = Client::connect(session.privileged_socket());
        let mut app = Client::connect(session.socket());

        let window = app.map_window("org.gnoblin.Window", "Window");
        window.toplevel.set_maximized();
        app.roundtrip();
        let deadline = Instant::now() + TIMEOUT;
        while app.state.height != 768 {
            app.roundtrip();
            assert!(
                Instant::now() < deadline,
                "the window was never maximised: {:?}",
                (app.state.width, app.state.height)
            );
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(app.state.width, 1024, "a maximised window is the work area's width");

        let shell = shell(&mut shell_client);
        let mut bar = shell_client.layer_surface(&shell, "org.gnoblin.TestBar", zwlr_layer_shell_v1::Layer::Top);
        commit_bar(
            &mut shell_client,
            &bar,
            (0, 30),
            zwlr_layer_surface_v1::Anchor::Top
                | zwlr_layer_surface_v1::Anchor::Left
                | zwlr_layer_surface_v1::Anchor::Right,
            30,
        );
        wait_configure(&mut shell_client, (1024, 30));
        bar.ack(&mut shell_client);
        shell_client.attach_buffer(&bar.surface, 1024, 30);

        // The reservation reaches the window manager, which re-applies the maximised
        // geometry: the window's height loses the bar's strip.
        let deadline = Instant::now() + TIMEOUT;
        while app.state.height != 738 {
            app.roundtrip();
            assert!(
                Instant::now() < deadline,
                "the work area did not shrink; the window is {:?}",
                (app.state.width, app.state.height)
            );
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(app.state.width, 1024);
        let observation = session.windows();
        let managed = observation.only().expect("one window");
        assert_eq!(
            managed.geometry.height, 738,
            "the window manager's work area is the same"
        );
    });
}

#[test]
fn a_bar_that_disappears_gives_its_strip_back() {
    let session = Session::start();
    session.run(|session| {
        let mut shell_client = Client::connect(session.privileged_socket());
        let mut app = Client::connect(session.socket());
        let window = app.map_window("org.gnoblin.Window", "Window");
        window.toplevel.set_maximized();
        app.roundtrip();
        session.wait_for(|observation| observation.only().is_some_and(|window| window.geometry.height == 768));

        let shell = shell(&mut shell_client);
        let mut bar = shell_client.layer_surface(&shell, "org.gnoblin.TestBar", zwlr_layer_shell_v1::Layer::Top);
        commit_bar(
            &mut shell_client,
            &bar,
            (0, 30),
            zwlr_layer_surface_v1::Anchor::Top
                | zwlr_layer_surface_v1::Anchor::Left
                | zwlr_layer_surface_v1::Anchor::Right,
            30,
        );
        wait_configure(&mut shell_client, (1024, 30));
        bar.ack(&mut shell_client);
        shell_client.attach_buffer(&bar.surface, 1024, 30);
        session.wait_for(|observation| observation.only().is_some_and(|window| window.geometry.height == 738));

        // The bar exits without saying anything about its zone.
        bar.layer_surface.destroy();
        bar.surface.destroy();
        shell_client.roundtrip();

        let observation = session.wait_for(|observation| {
            observation.only().is_some_and(|window| window.geometry.height == 768) && observation.layers().is_empty()
        });
        assert!(
            observation.layer("org.gnoblin.TestBar").is_none(),
            "the bar left the published list"
        );
    });
}

#[test]
fn a_zero_size_surface_with_no_anchors_is_a_client_error() {
    let session = Session::start();
    session.run(|session| {
        let mut offender = Client::connect(session.privileged_socket());
        let shell = shell(&mut offender);
        let bar = offender.layer_surface(&shell, "org.gnoblin.NoAnchors", zwlr_layer_shell_v1::Layer::Top);
        bar.layer_surface.set_size(0, 0);
        bar.layer_surface.set_anchor(zwlr_layer_surface_v1::Anchor::empty());
        bar.surface.commit();

        // The protocol has no answer for "cover everything without anchors", so the
        // commit is a protocol error on that connection and only that connection.
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match offender.queue.roundtrip(&mut offender.state) {
                Err(_) => break,
                Ok(_) => {
                    assert!(Instant::now() < deadline, "the offending client was never disconnected");
                    thread::sleep(Duration::from_millis(5));
                }
            }
        }
        assert!(
            session.windows().layers().is_empty(),
            "the refused surface never reached the session"
        );

        // The session and every other client keep working.
        let mut app = Client::connect(session.socket());
        let _window = app.map_window("org.gnoblin.Window", "Window");
        session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
    });
}

#[test]
fn an_exclusive_surface_keeps_the_keyboard() {
    let session = Session::start();
    session.run(|session| {
        let mut bar_client = Client::connect(session.privileged_socket());
        let shell = shell(&mut bar_client);
        let mut bar = bar_client.layer_surface(&shell, "org.gnoblin.Prompt", zwlr_layer_shell_v1::Layer::Overlay);
        bar.layer_surface.set_size(0, 40);
        bar.layer_surface.set_anchor(
            zwlr_layer_surface_v1::Anchor::Top
                | zwlr_layer_surface_v1::Anchor::Left
                | zwlr_layer_surface_v1::Anchor::Right,
        );
        bar.layer_surface
            .set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive);
        bar.surface.commit();
        bar_client.roundtrip();
        wait_configure(&mut bar_client, (1024, 40));
        bar.ack(&mut bar_client);
        bar_client.attach_buffer(&bar.surface, 1024, 40);
        bar_client.wait_until("the keyboard", |state| state.keyboard_enters >= 1);

        // A window maps afterwards and does not take the keyboard from it.
        let mut app = Client::connect(session.socket());
        let _window = app.map_window("org.gnoblin.Window", "Window");
        session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        bar_client.roundtrip();
        assert_eq!(
            bar_client.state.keyboard_enters, 1,
            "the exclusive surface kept the keyboard"
        );
        assert_eq!(app.state.keyboard_enters, 0, "the new window never had it");
    });
}

#[test]
fn a_disconnected_output_reserves_nothing() {
    let (session, outputs) = Session::start_detached();
    session.run(move |session| {
        let mut shell_client = Client::connect(session.privileged_socket());
        let shell = shell(&mut shell_client);
        let mut bar = shell_client.layer_surface(&shell, "org.gnoblin.TestBar", zwlr_layer_shell_v1::Layer::Top);
        commit_bar(
            &mut shell_client,
            &bar,
            (0, 30),
            zwlr_layer_surface_v1::Anchor::Top
                | zwlr_layer_surface_v1::Anchor::Left
                | zwlr_layer_surface_v1::Anchor::Right,
            30,
        );
        wait_configure(&mut shell_client, (1024, 30));
        bar.ack(&mut shell_client);
        shell_client.attach_buffer(&bar.surface, 1024, 30);
        session.wait_for(|observation| !observation.layers().is_empty());

        let output = session.windows().outputs().first().copied().expect("an output");
        outputs.disconnect_output(output);
        session.wait_for(|observation| observation.outputs().is_empty());

        // The output comes back, and its work area is whole again: the old bar's strip
        // did not survive the output it belonged to.
        outputs
            .connect_output(
                gnoblin_backend::headless::DEFAULT_CONNECTOR,
                gnoblin_backend::headless::DEFAULT_MODE,
                gnoblin_types::Scale::IDENTITY,
            )
            .expect("a 1024x768 output");
        session.wait_for(|observation| observation.outputs().len() == 1);

        let mut app = Client::connect(session.socket());
        let window = app.map_window("org.gnoblin.Window", "Window");
        window.toplevel.set_maximized();
        app.roundtrip();
        let deadline = Instant::now() + TIMEOUT;
        while (app.state.width, app.state.height) != (1024, 768) {
            app.roundtrip();
            assert!(
                Instant::now() < deadline,
                "the new output kept a stale reservation: {:?}",
                (app.state.width, app.state.height)
            );
            thread::sleep(Duration::from_millis(5));
        }
    });
}
