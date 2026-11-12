//! `zwlr_layer_shell_v1` against a real client.
//!
//! Each test connects a real `wayland-client` — a bar on the privileged socket — and
//! asserts on what the client was configured with and on what the session published,
//! because a bar's contract is its configure size and the work area it leaves behind.
//! The failure modes are the ones at the top of `protocols::zwlr_layer_shell_v1`.
//!
//! The tests after `a_disconnected_output_reserves_nothing` pin what deleting the layer registry
//! (`design/cutover-survey.md` 16.1, lane 8C) must not change. The registry recorded every layer surface under a
//! name and read none of it back, so a surface's layer, its zone, its keyboard and its popups are the protocol's
//! alone. Each of these passes on the tree before the registry went and on the tree after it.

mod session;

use std::thread;
use std::time::{Duration, Instant};

use session::{Client, LayerWindow, PopupPlace, Session, TIMEOUT};
use wayland_protocols::xdg::shell::client::xdg_positioner;
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

/// The anchors of a bar that spans the top of an output.
fn across_the_top() -> zwlr_layer_surface_v1::Anchor {
    zwlr_layer_surface_v1::Anchor::Top | zwlr_layer_surface_v1::Anchor::Left | zwlr_layer_surface_v1::Anchor::Right
}

/// Maps a bar across the top of the 1024 wide output the way a shell does: its attributes and first commit, the
/// configure the compositor answers with, the acknowledgement, and a buffer of the configured size.
fn map_bar(
    client: &mut Client,
    namespace: &str,
    layer: zwlr_layer_shell_v1::Layer,
    height: u32,
    zone: i32,
) -> LayerWindow {
    let shell = shell(client);
    let mut bar = client.layer_surface(&shell, namespace, layer);
    commit_bar(client, &bar, (0, height), across_the_top(), zone);
    wait_configure(client, (1024, height));
    bar.ack(client);
    client.attach_buffer(&bar.surface, 1024, i32::try_from(height).expect("a small height"));
    bar
}

/// Maps a 40 high prompt across the top of the output that asks for the keyboard exclusively.
fn map_prompt(client: &mut Client, namespace: &str, layer: zwlr_layer_shell_v1::Layer) -> LayerWindow {
    let shell = shell(client);
    let mut prompt = client.layer_surface(&shell, namespace, layer);
    prompt.layer_surface.set_size(0, 40);
    prompt.layer_surface.set_anchor(across_the_top());
    prompt
        .layer_surface
        .set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive);
    prompt.surface.commit();
    client.roundtrip();
    wait_configure(client, (1024, 40));
    prompt.ack(client);
    client.attach_buffer(&prompt.surface, 1024, 40);
    prompt
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
        // The client sees its error before the server has finished tearing the refused surface
        // down: the layer is removed from `layer_destroyed`, which runs on the server thread once
        // the disconnect is processed. So wait for the entry to go rather than reading it once,
        // which raced that teardown. A surface that is never removed times out with its state.
        session.wait_for(|observation| observation.layers().is_empty());

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

#[test]
fn a_surface_that_changes_layer_is_published_in_the_one_it_moved_to() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::connect(session.privileged_socket());
        let bar = map_bar(
            &mut client,
            "org.gnoblin.TestBar",
            zwlr_layer_shell_v1::Layer::Bottom,
            30,
            30,
        );
        let observation = session.wait_for(|observation| {
            observation
                .layer("org.gnoblin.TestBar")
                .is_some_and(|layer| layer.mapped)
        });
        let layer = observation.layer("org.gnoblin.TestBar").expect("the bar is published");
        assert_eq!(layer.layer, "bottom", "the layer it asked for");

        // The shell raises its bar: the next commit carries the new layer and nothing else about the surface moves.
        bar.layer_surface.set_layer(zwlr_layer_shell_v1::Layer::Overlay);
        bar.surface.commit();
        client.roundtrip();
        let observation = session.wait_for(|observation| {
            observation
                .layer("org.gnoblin.TestBar")
                .is_some_and(|layer| layer.layer == "overlay")
        });
        let layer = observation
            .layer("org.gnoblin.TestBar")
            .expect("the bar is still published");
        assert_eq!(
            observation.layers().len(),
            1,
            "the surface changed layer, it did not gain a second entry"
        );
        assert_eq!(
            layer.rect,
            gnoblin_types::Rect::new(0, 0, 1024, 30),
            "the strip it occupies"
        );
        assert_eq!(layer.exclusive, 30, "the zone it reserves");
        assert!(layer.mapped, "it is still shown");
    });
}

#[test]
fn two_bars_in_one_namespace_each_keep_their_own_strip() {
    let session = Session::start();
    session.run(|session| {
        let mut app = Client::connect(session.socket());
        let window = app.map_window("org.gnoblin.Window", "Window");
        window.toplevel.set_maximized();
        app.roundtrip();
        session.wait_for(|observation| observation.only().is_some_and(|window| window.geometry.height == 768));

        // A namespace is the client's name for its surface. Two clients may use the same one, and the
        // compositor keeps both: each reserves its own strip.
        let mut first_client = Client::connect(session.privileged_socket());
        let mut second_client = Client::connect(session.privileged_socket());
        let first = map_bar(
            &mut first_client,
            "org.gnoblin.TestBar",
            zwlr_layer_shell_v1::Layer::Top,
            30,
            30,
        );
        let second = map_bar(
            &mut second_client,
            "org.gnoblin.TestBar",
            zwlr_layer_shell_v1::Layer::Top,
            20,
            20,
        );
        let observation = session.wait_for(|observation| {
            observation.layers().len() == 2 && observation.only().is_some_and(|window| window.geometry.height == 718)
        });
        assert!(
            observation
                .layers()
                .iter()
                .all(|layer| layer.namespace == "org.gnoblin.TestBar" && layer.mapped),
            "both surfaces are published under the namespace they named"
        );
        let heights: Vec<u32> = observation.layers().iter().map(|layer| layer.rect.height).collect();
        assert_eq!(heights, [30, 20], "in the order they were created");

        // The first exits: its strip comes back and the second's stays.
        first.layer_surface.destroy();
        first.surface.destroy();
        first_client.roundtrip();
        session.wait_for(|observation| {
            observation.layers().len() == 1 && observation.only().is_some_and(|window| window.geometry.height == 748)
        });

        second.layer_surface.destroy();
        second.surface.destroy();
        second_client.roundtrip();
        session.wait_for(|observation| {
            observation.layers().is_empty() && observation.only().is_some_and(|window| window.geometry.height == 768)
        });
    });
}

#[test]
fn a_top_layer_surface_holds_the_keyboard_from_a_window_that_may_take_focus() {
    // The default session refuses a new window focus, which would make this test pass for the wrong reason.
    let session = Session::start_allowing_focus();
    session.run(|session| {
        let mut prompt_client = Client::connect(session.privileged_socket());
        let _prompt = map_prompt(
            &mut prompt_client,
            "org.gnoblin.Prompt",
            zwlr_layer_shell_v1::Layer::Top,
        );
        prompt_client.wait_until("the keyboard", |state| state.keyboard_enters >= 1);

        let mut app = Client::connect(session.socket());
        let _window = app.map_window("org.gnoblin.Window", "Window");
        session.wait_for(|observation| observation.only().is_some_and(|window| window.mapped));
        prompt_client.roundtrip();
        assert_eq!(
            prompt_client.state.keyboard_enters, 1,
            "the exclusive surface kept the keyboard"
        );
        assert_eq!(
            app.state.keyboard_enters, 0,
            "the window that may take focus never had it"
        );
    });
}

#[test]
fn a_bottom_layer_surface_does_not_hold_the_keyboard_from_a_window() {
    // The protocol grants exclusive focus only above the shell layer, so a bottom surface's request is not one.
    let session = Session::start_allowing_focus();
    session.run(|session| {
        let mut wallpaper_client = Client::connect(session.privileged_socket());
        let _wallpaper = map_prompt(
            &mut wallpaper_client,
            "org.gnoblin.Wallpaper",
            zwlr_layer_shell_v1::Layer::Bottom,
        );

        let mut app = Client::connect(session.socket());
        let _window = app.map_window("org.gnoblin.Window", "Window");
        app.wait_until("the keyboard", |state| state.keyboard_enters >= 1);
        wallpaper_client.roundtrip();
        assert_eq!(
            wallpaper_client.state.keyboard_enters, 0,
            "the bottom surface never held the keyboard"
        );
    });
}

#[test]
fn a_popup_of_a_bar_is_placed_against_the_bar() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::connect(session.privileged_socket());
        let bar = map_bar(
            &mut client,
            "org.gnoblin.TestBar",
            zwlr_layer_shell_v1::Layer::Top,
            30,
            30,
        );
        session.wait_for(|observation| {
            observation
                .layer("org.gnoblin.TestBar")
                .is_some_and(|layer| layer.mapped)
        });

        // A menu 100 wide and 20 high whose corner is at (500, 5) in the bar: the compositor tells the client
        // where the positioner put it, measured from the bar's own origin.
        let popup = client.layer_popup(&bar, &PopupPlace::at(500, 5, (100, 20)));
        assert_eq!(client.state.popup_placed, Some((500, 5, 100, 20)));
        assert!(!client.state.popup_done, "a popup of a live bar is not dismissed");
        popup.popup.destroy();
        popup.xdg_surface.destroy();
        popup.surface.destroy();
        client.roundtrip();
        assert!(
            session
                .windows()
                .layer("org.gnoblin.TestBar")
                .is_some_and(|layer| layer.mapped),
            "closing the menu leaves the bar where it was"
        );
    });
}

#[test]
fn a_popup_of_a_bar_that_is_repositioned_stays_inside_the_bar() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::connect(session.privileged_socket());
        let bar = map_bar(
            &mut client,
            "org.gnoblin.TestBar",
            zwlr_layer_shell_v1::Layer::Top,
            30,
            30,
        );
        session.wait_for(|observation| {
            observation
                .layer("org.gnoblin.TestBar")
                .is_some_and(|layer| layer.mapped)
        });
        let popup = client.layer_popup(&bar, &PopupPlace::at(500, 5, (100, 20)));

        // The menu is asked to sit lower, with room to slide: a popup whose root is a layer surface is kept
        // inside that surface's own rectangle, not inside the whole monitor, so it slides up to the bar's
        // bottom edge instead of hanging 10 below it.
        let lower = PopupPlace {
            constraint_adjustment: xdg_positioner::ConstraintAdjustment::SlideY,
            ..PopupPlace::at(500, 20, (100, 20))
        };
        let positioner = client.positioner(&lower);
        popup.popup.reposition(&positioner, 7);
        positioner.destroy();
        client.wait_until("the repositioned popup", |state| state.popup_repositioned == Some(7));
        assert_eq!(client.state.popup_placed, Some((500, 10, 100, 20)));
    });
}
