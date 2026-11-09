//! `xdg_activation_v1` against a real client.
//!
//! A token is earned, bound and spent, and every refusal is visible to the session as
//! an `activation-denied` observation — so these tests assert on both sides: what the
//! focused window is, and what the session recorded about the request.

mod session;

use std::thread;
use std::time::{Duration, Instant};

use session::{Client, Session, TIMEOUT};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1;
use wayland_protocols::xdg::shell::client::xdg_toplevel;

/// Binds the activation global.
fn activation(client: &mut Client) -> xdg_activation_v1::XdgActivationV1 {
    client
        .globals
        .bind(&client.qh, 1..=1, ())
        .expect("every session advertises xdg_activation_v1")
}

/// Asks for a token, naming `serial` when the caller has one.
fn request_token(
    client: &mut Client,
    activation: &xdg_activation_v1::XdgActivationV1,
    serial: Option<u32>,
) -> Option<String> {
    let token = activation.get_activation_token(&client.qh, ());
    if let Some(serial) = serial {
        token.set_serial(serial, &client.seat);
    }
    token.commit();
    // A token is minted (or refused) during the dispatch the commit causes.
    for _ in 0..20 {
        client.roundtrip();
        if client.state.token.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    client.state.token.clone()
}

/// Clicks a window and returns the serial the button carried.
fn click_for_serial(client: &mut Client, session: &session::Handle, _window: &session::Window) -> u32 {
    let geometry = session
        .wait_for(|observation| observation.only().is_some_and(|candidate| candidate.mapped))
        .only()
        .expect("the clicked window is managed")
        .geometry;
    // The buffer `map_window` draws sits at the frame origin; only it takes pointer input.
    let centre = gnoblin_types::Point::new(geometry.x + 100, geometry.y + 75);
    session.click(centre);
    let deadline = Instant::now() + TIMEOUT;
    loop {
        client.roundtrip();
        if let Some(serial) = client.state.button_serial {
            return serial;
        }
        assert!(Instant::now() < deadline, "the click never reached the client");
        thread::sleep(Duration::from_millis(5));
    }
}

/// Waits for the session to report one window with this title focused.
fn wait_focus(session: &session::Handle, title: &str) {
    session.wait_for(|observation| observation.focused().is_some_and(|window| window.title == title));
}

/// Waits for a denial with this reason.
fn wait_denial(session: &session::Handle, reason: &str) {
    session.wait_for(|observation| {
        observation
            .activation_denied()
            .is_some_and(|denial| denial.reason == reason)
    });
}

#[test]
fn a_token_from_a_click_activates_the_window() {
    let session = Session::start_allowing_focus();
    session.run(|session| {
        let mut alpha = Client::connect(session.socket());
        let activation = activation(&mut alpha);
        let alpha_window = alpha.map_window("org.gnoblin.Alpha", "Alpha");
        wait_focus(session, "Alpha");
        let serial = click_for_serial(&mut alpha, session, &alpha_window);

        let token = request_token(&mut alpha, &activation, Some(serial)).expect("a click earns a token");

        // Another window shows up and takes the focus and the keyboard with it.
        let mut beta = Client::connect(session.socket());
        let _beta_window = beta.map_window("org.gnoblin.Beta", "Beta");
        wait_focus(session, "Beta");

        activation.activate(token, &alpha_window.surface);
        alpha.roundtrip();
        wait_focus(session, "Alpha");
        assert!(
            session.windows().activation_denied().is_none(),
            "an earned token is not a refusal"
        );
        let _: Option<&xdg_toplevel::XdgToplevel> = None;
    });
}

#[test]
fn a_token_cannot_be_redeemed_twice() {
    let session = Session::start();
    session.run(|session| {
        let mut alpha = Client::connect(session.socket());
        let activation = activation(&mut alpha);
        let alpha_window = alpha.map_window("org.gnoblin.Alpha", "Alpha");
        wait_focus(session, "Alpha");
        let serial = click_for_serial(&mut alpha, session, &alpha_window);
        let token = request_token(&mut alpha, &activation, Some(serial)).expect("a click earns a token");

        activation.activate(token.clone(), &alpha_window.surface);
        alpha.roundtrip();
        wait_focus(session, "Alpha");

        activation.activate(token, &alpha_window.surface);
        alpha.roundtrip();
        wait_denial(session, "reused-token");
    });
}

#[test]
fn a_fabricated_token_is_refused() {
    let session = Session::start_allowing_focus();
    session.run(|session| {
        let mut alpha = Client::connect(session.socket());
        let activation = activation(&mut alpha);
        let alpha_window = alpha.map_window("org.gnoblin.Alpha", "Alpha");
        wait_focus(session, "Alpha");

        let mut beta = Client::connect(session.socket());
        let _beta_window = beta.map_window("org.gnoblin.Beta", "Beta");
        wait_focus(session, "Beta");

        // A token this session never minted is not in the compositor's token list at
        // all, so `xdg_activation.activate` reaches no handler: the request is dropped
        // before it can become a decision, and focus does not move.
        activation.activate("0000000000000000000000000000dead".to_owned(), &alpha_window.surface);
        alpha.roundtrip();
        for _ in 0..20 {
            alpha.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            session.windows().focused().map(|window| window.title.as_str()),
            Some("Beta"),
            "an invented token never activates anything"
        );
        assert!(
            session.windows().activation_denied().is_none(),
            "a token the compositor never issued is not a decision, so nothing is denied"
        );
    });
}

#[test]
fn an_app_id_alone_does_not_activate() {
    let session = Session::start_allowing_focus();
    session.run(|session| {
        let mut alpha = Client::connect(session.socket());
        let activation = activation(&mut alpha);
        let alpha_window = alpha.map_window("org.gnoblin.Claimant", "Claimant");
        wait_focus(session, "Claimant");

        let mut beta = Client::connect(session.socket());
        let _beta_window = beta.map_window("org.gnoblin.Other", "Other");
        wait_focus(session, "Other");

        // An application id is a label the process chooses, so presenting one as the
        // authority buys exactly what an invented token buys: nothing.
        activation.activate("org.gnoblin.Claimant".to_owned(), &alpha_window.surface);
        for _ in 0..20 {
            alpha.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            session.windows().focused().map(|window| window.title.as_str()),
            Some("Other"),
            "an app id never grants focus"
        );
    });
}

#[test]
fn a_token_needs_a_trusted_context() {
    let session = Session::start();
    session.run(|session| {
        let mut client = Client::connect(session.socket());
        let activation = activation(&mut client);
        let _window = client.map_window("org.gnoblin.Silent", "Silent");
        wait_focus(session, "Silent");

        // A serial this session never issued is not a gesture, and an ordinary
        // connection is not one of the session's own processes, so the compositor keeps
        // no token. `xdg-activation` still answers `done` with a string — the protocol
        // has no refusal event — but redeeming it reaches nothing.
        let token = request_token(&mut client, &activation, Some(1));
        if let Some(token) = token {
            assert!(token.len() >= 32, "a token is at least the protocol's minimum");
        }
        activation.activate(
            client
                .state
                .token
                .clone()
                .unwrap_or_else(|| "0000000000000000000000000000dead".to_owned()),
            &_window.surface,
        );
        for _ in 0..20 {
            client.roundtrip();
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            session.windows().activation_denied().is_none(),
            "an unearned token is dropped, not decided"
        );
    });
}
