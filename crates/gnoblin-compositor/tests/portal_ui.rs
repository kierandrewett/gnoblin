//! The portal dialog relay over real IPC: a `portal` role asks for a consent dialog, an
//! `input` role routes keys into it, and the requester reads the settled outcome.
//!
//! Nothing here is a mock: the compositor is a running session on its own sockets and every
//! connection is a real `Wire` over a Unix stream.
//!
//! A request and a key travel on different connections, so nothing orders a key after its
//! request. `admit` is the barrier: a test admits a dialog before it keys it. Each test owns
//! its sockets and directory, so the tests run concurrently.

mod support;

use std::os::unix::net::UnixStream;
use std::thread;
use std::time::Duration;

use gnoblin_client::wire::{PortalRequest, VerifiedCaller, portal_cancel, portal_key, portal_ui, read_portal_outcome};
use gnoblin_model::Fields;
use gnoblin_proto::Envelope;
use support::{Session, Wire};

/// The consent interface a Screenshot request names; its dialog opens with Deny focused.
const SCREENSHOT: &str = "Screenshot";
/// An interface the compositor has no screen for; it must settle as `unsupported-ui`.
const WALLPAPER: &str = "Wallpaper";
/// The arm delay a consent dialog waits before Enter can release Allow.
const ARM_DELAY: Duration = Duration::from_millis(600);
/// How long a test waits for a reply that must arrive.
const REPLY_WAIT: Duration = Duration::from_secs(5);

fn consent(interface: &str) -> PortalRequest {
    PortalRequest {
        interface: interface.to_owned(),
        caller: VerifiedCaller::new(0, "org.example.Requester"),
        prompt: "Share the screen?".to_owned(),
        timeout_ms: 30_000,
        options: Fields::default(),
        parent_window: String::new(),
    }
}

/// Takes the named role's end the session handed out, once.
fn take(ends: &mut Vec<(String, UnixStream)>, role: &str) -> Wire {
    let index = ends
        .iter()
        .position(|(name, _)| name == role)
        .unwrap_or_else(|| panic!("the session handed out no end for {role}"));
    let (_, stream) = ends.remove(index);
    let wire = Wire::over(stream);
    wire.set_read_timeout(REPLY_WAIT);
    wire
}

/// The `(kind, detail)` a settled request replied with.
fn outcome(envelope: &Envelope) -> (String, String) {
    read_portal_outcome(envelope.payload().unwrap_or(&[])).expect("a portal outcome reply")
}

/// Receives the reply to one request, and checks it answers the request it was sent for.
fn receive_for(wire: &mut Wire, request_id: u64) -> Envelope {
    let envelope = wire.receive().expect("the reply arrives before the deadline");
    assert_eq!(envelope.request_id, request_id, "the reply answers the request sent");
    envelope
}

/// Sends one key to the dialog for `request_id` and waits for the input role's ack.
fn key(wire: &mut Wire, request_id: u64, name: &str) {
    wire.send(&portal_key(request_id, name, false));
    let _ack = wire.receive();
}

/// Admits `request_id` on the requester's connection, so a key sent next lands on its dialog.
///
/// A second `portal.ui` for the same id is refused `request-taken` only after the first is
/// held: one connection is read in order, so that refusal is the barrier.
fn admit(portal: &mut Wire, request_id: u64) {
    portal.send(&portal_ui(request_id, &consent(SCREENSHOT)));
    portal.send(&portal_ui(request_id, &consent(SCREENSHOT)));
    let refusal = receive_for(portal, request_id);
    assert_eq!(
        refusal.error_code(),
        Some("request-taken"),
        "the duplicate is refused because the first is held"
    );
}

#[test]
fn tab_then_enter_on_allow_is_accepted() {
    let (session, mut ends) = Session::start_privileged(&["portal", "input"]);
    let mut portal = take(&mut ends, "portal");
    let mut input = take(&mut ends, "input");

    admit(&mut portal, 1);
    thread::sleep(ARM_DELAY);
    key(&mut input, 1, "tab");
    key(&mut input, 1, "enter");

    let reply = receive_for(&mut portal, 1);
    assert_eq!(outcome(&reply), ("handled".to_owned(), "allowed".to_owned()));
    drop(session);
}

#[test]
fn enter_on_default_focus_is_cancelled_by_the_user() {
    let (session, mut ends) = Session::start_privileged(&["portal", "input"]);
    let mut portal = take(&mut ends, "portal");
    let mut input = take(&mut ends, "input");

    admit(&mut portal, 2);
    thread::sleep(ARM_DELAY);
    key(&mut input, 2, "enter");

    let reply = receive_for(&mut portal, 2);
    assert_eq!(outcome(&reply), ("cancelled".to_owned(), "user".to_owned()));
    drop(session);
}

#[test]
fn escape_cancels_at_any_time() {
    let (session, mut ends) = Session::start_privileged(&["portal", "input"]);
    let mut portal = take(&mut ends, "portal");
    let mut input = take(&mut ends, "input");

    admit(&mut portal, 3);
    key(&mut input, 3, "escape");

    let reply = receive_for(&mut portal, 3);
    assert_eq!(outcome(&reply), ("cancelled".to_owned(), "user".to_owned()));
    drop(session);
}

#[test]
fn portal_cancel_settles_the_request_for_its_requester() {
    let (session, mut ends) = Session::start_privileged(&["portal"]);
    let mut portal = take(&mut ends, "portal");

    portal.send(&portal_ui(4, &consent(SCREENSHOT)));
    portal.send(&portal_cancel(4));

    let reply = receive_for(&mut portal, 4);
    assert_eq!(outcome(&reply), ("cancelled".to_owned(), "user".to_owned()));
    drop(session);
}

#[test]
fn a_second_request_queues_and_is_served_after_the_first() {
    let (session, mut ends) = Session::start_privileged(&["portal", "input"]);
    let mut portal = take(&mut ends, "portal");
    let mut input = take(&mut ends, "input");

    admit(&mut portal, 5);
    // The second request queues behind the first. It is admitted before the key that settles
    // the first, because a queued request has no reply to wait on.
    admit(&mut portal, 6);
    thread::sleep(ARM_DELAY);
    key(&mut input, 5, "escape");

    let first = receive_for(&mut portal, 5);
    assert_eq!(outcome(&first), ("cancelled".to_owned(), "user".to_owned()));

    key(&mut input, 6, "escape");
    let second = receive_for(&mut portal, 6);
    assert_eq!(outcome(&second), ("cancelled".to_owned(), "user".to_owned()));
    drop(session);
}

#[test]
fn requester_disconnect_tears_the_dialog_down_and_late_keys_do_nothing() {
    let (session, mut ends) = Session::start_privileged(&["portal", "portal", "input"]);
    let mut first = take(&mut ends, "portal");
    let mut second = take(&mut ends, "portal");
    let mut input = take(&mut ends, "input");

    admit(&mut first, 7);
    drop(first);

    // The departed requester's dialog is torn down, so a key for it must not reach a
    // reply to anyone. A fresh requester is still served by its own keys.
    key(&mut input, 7, "escape");
    // Departure has no reply to wait on, so give the server's tick time to tear the dialog down.
    thread::sleep(ARM_DELAY);
    admit(&mut second, 8);
    key(&mut input, 8, "escape");

    let reply = receive_for(&mut second, 8);
    assert_eq!(outcome(&reply), ("cancelled".to_owned(), "user".to_owned()));
    drop(session);
}

#[test]
fn a_connection_without_the_portal_role_is_refused() {
    let session = Session::start();
    let mut plain = session.wire();
    plain.set_read_timeout(REPLY_WAIT);

    plain.send(&portal_ui(9, &consent(SCREENSHOT)));
    let reply = receive_for(&mut plain, 9);
    assert!(
        reply.error_code().is_some(),
        "a core connection is refused with a typed error"
    );
    drop(session);
}

#[test]
fn an_interface_without_a_screen_settles_as_unsupported_ui() {
    let (session, mut ends) = Session::start_privileged(&["portal"]);
    let mut portal = take(&mut ends, "portal");

    portal.send(&portal_ui(10, &consent(WALLPAPER)));

    let reply = receive_for(&mut portal, 10);
    assert_eq!(reply.error_code(), Some("unsupported-ui"));
    drop(session);
}
