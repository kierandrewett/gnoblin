//! Replacing the built-in portal dialogs: a shell claims `portal.ui`, and the compositor routes a
//! consent request to it instead of drawing its own dialog.
//!
//! The built-in dialog is the default and the fallback; this is the path that replaces it
//! (design/rust-compositor.md, D19). Nothing here is a mock: the compositor is a running session on
//! its own sockets, the relay and the input role are real privileged connections, and the shell is
//! the real client library on the ordinary socket.
//!
//! # Failure modes first
//!
//! 1. **A claimed `portal.ui` is ignored and the built-in dialog opens anyway.** The claimant is
//!    offered the request, and its answer settles it with no key pressed anywhere:
//!    `a_claimant_is_offered_the_consent_and_its_allow_settles_it`.
//! 2. **A claimant's answer grants more than the compositor allows, or is asked twice.** An
//!    `allow` inside the 500 ms arming window is a refusal, `deny` refuses, and an answer that is
//!    neither is never consent. A `deny` and an early `allow` are terminal: the request ends and
//!    the built-in dialog does not open for it, so nothing asks the user again:
//!    `an_allow_inside_the_arming_window_is_a_refusal`, `a_deny_refuses`,
//!    `a_deny_is_terminal_and_the_built_in_dialog_never_opens`,
//!    `an_early_allow_is_terminal_and_the_built_in_dialog_never_opens`,
//!    `an_answer_that_is_neither_allow_nor_deny_falls_back_to_the_built_in_dialog`.
//! 3. **A claimant that cannot answer leaves the requester waiting, or worse, consenting.** It
//!    falls back to the built-in dialog, which opens with Deny focused: when the claimant is
//!    unresponsive past its window, declines, answers badly or disconnects mid-request:
//!    `an_unresponsive_claimant_falls_back_to_the_built_in_dialog`,
//!    `a_declining_claimant_falls_back_to_the_built_in_dialog`,
//!    `a_claimant_that_disconnects_mid_request_falls_back_to_the_built_in_dialog`.
//! 4. **A requester that gives up leaves the claimant drawing a dialog for nobody.** The offer
//!    ends and the claimant is told: `a_requester_that_cancels_ends_the_claimants_offer`.
//! 5. **Releasing the claim leaves the slot dead.** The built-in dialog serves again:
//!    `releasing_the_claim_restores_the_built_in_dialog`.
//! 6. **Anyone can take the slot.** A second claimant, the services role and a `none` claim are
//!    all refused: `a_second_shell_cannot_take_portal_ui`,
//!    `the_services_role_cannot_claim_portal_ui`, `a_none_claim_cannot_clear_another_connections_slot`.
//! 7. **`prompt.secret` still reaches its claimant.** The offer arrives, the answer reaches the
//!    host and a claimant that leaves ends the request: `a_prompt_secret_claimant_answers_the_host`,
//!    `a_prompt_secret_claimant_that_leaves_ends_the_request`.
//! 8. **A locked session is served a dialog by the claimant.** A request while the session is
//!    locked is refused `locked`: `a_locked_session_refuses_a_request_even_with_a_claimant`.
//! 9. **A connection that is not the owner's takes part.** It is refused `denied` when it claims or
//!    releases a slot, and when it claims, answers, declines or offers a request, and nothing it
//!    sent settles the owner's request: `a_connection_that_is_not_the_owner_cannot_claim_or_release_a_slot`,
//!    `a_connection_that_is_not_the_owner_cannot_claim_answer_decline_or_offer_a_request`.

mod support;

use std::os::unix::net::UnixStream;
use std::thread;
use std::time::Duration;

use gnoblin_client::unix::UnixTransport;
use gnoblin_client::wire::{PortalRequest, VerifiedCaller, portal_cancel, portal_key, portal_ui, read_portal_outcome};
use gnoblin_client::{Client, ClientError, Event, code};
use gnoblin_model::{Fields, Provider, RequestCompletion};
use gnoblin_proto::Envelope;
use gnoblin_types::RequestId;
use support::{Locker, Session, TIMEOUT, Wire};

/// The consent interface a Screenshot request names.
const SCREENSHOT: &str = "Screenshot";
/// The app id the relay reports for the caller.
const APP: &str = "org.example.Requester";
/// Longer than the arming window.
const ARM_DELAY: Duration = Duration::from_millis(600);
/// How long a test waits for a reply that must arrive.
const REPLY_WAIT: Duration = Duration::from_secs(5);

/// A consent request from `APP` that the requester will wait `timeout_ms` for.
fn consent(timeout_ms: u64) -> PortalRequest {
    PortalRequest {
        interface: SCREENSHOT.to_owned(),
        caller: VerifiedCaller::new(0, APP),
        prompt: "Share the screen?".to_owned(),
        timeout_ms,
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

/// A client on the ordinary socket, which is the session owner's own uid.
fn socket_client(session: &Session) -> Client<UnixTransport> {
    let mut client = Client::connect(
        UnixTransport::connect(session.socket()).expect("the session listens"),
        gnoblin_compositor::version(),
    )
    .expect("the handshake");
    client.set_read_timeout(Some(TIMEOUT)).expect("a read deadline");
    client
}

/// A client on a role's privileged end.
fn role_client(ends: &mut Vec<(String, UnixStream)>, role: &str) -> Client<UnixTransport> {
    let index = ends
        .iter()
        .position(|(name, _)| name == role)
        .unwrap_or_else(|| panic!("no privileged end for {role}"));
    let (_, stream) = ends.remove(index);
    let mut client = Client::connect(
        UnixTransport::from_stream(stream).expect("a transport"),
        gnoblin_compositor::version(),
    )
    .expect("the handshake");
    client.set_read_timeout(Some(TIMEOUT)).expect("a read deadline");
    client
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

/// What the claimant is offered.
struct Offer {
    request: RequestId,
    slot: String,
    title: String,
    message: String,
    subject: Vec<u8>,
}

/// The next offer the claimant receives.
fn next_offer(shell: &mut Client<UnixTransport>) -> Offer {
    match shell.next_event().expect("the offer arrives") {
        Event::RequestOffered {
            request,
            slot,
            title,
            message,
            subject,
            ..
        } => Offer {
            request,
            slot,
            title,
            message,
            subject,
        },
        other => panic!("the first event the claimant sees is the offer, got {other:?}"),
    }
}

/// Presses Escape on the built-in dialog until the requester's reply arrives, and returns it.
///
/// A key sent while no dialog is open does nothing, so the loop is what waits for the compositor
/// to fall back: the reply is `cancelled(user)` only if the built-in dialog opened and took the key.
fn settle_with_escape(relay: &mut Wire, input: &mut Wire, request_id: u64) -> (String, String) {
    relay.set_read_timeout(Duration::from_millis(100));
    for attempt in 0..150_u64 {
        input.send(&portal_key(1_000 + attempt, "escape", false));
        let _ack = input.receive();
        if let Some(reply) = relay.receive_within() {
            assert_eq!(reply.request_id, request_id, "the reply answers the request sent");
            return outcome(&reply);
        }
    }
    panic!("the built-in dialog never served the request");
}

/// A session with the relay and input roles and a shell that claimed `portal.ui`.
fn session_with_claimant() -> (Session, Wire, Wire, Client<UnixTransport>) {
    let (session, mut ends) = Session::start_privileged(&["portal", "input"]);
    let relay = take(&mut ends, "portal");
    let input = take(&mut ends, "input");
    let mut shell = socket_client(&session);
    shell
        .claim_slot("portal.ui", Provider::Shell)
        .expect("a shell may claim portal.ui");
    (session, relay, input, shell)
}

#[test]
fn a_claimant_is_offered_the_consent_and_its_allow_settles_it() {
    let (session, mut relay, _input, mut shell) = session_with_claimant();

    relay.send(&portal_ui(1, &consent(30_000)));
    let offer = next_offer(&mut shell);
    assert_eq!(offer.slot, "portal.ui");
    assert_eq!(offer.title, SCREENSHOT, "the title names the interface");
    assert_eq!(offer.message, "Share the screen?");
    assert_eq!(
        offer.subject,
        APP.as_bytes(),
        "the subject is the app id the relay verified, never one from the request body"
    );

    // No key is pressed anywhere: the claimant's answer alone settles the request, which is the
    // proof that the built-in dialog did not open for it.
    thread::sleep(ARM_DELAY);
    shell.claim(offer.request).expect("the claimant claims the offer");
    shell.answer(offer.request, b"allow").expect("the claimant answers");

    let reply = receive_for(&mut relay, 1);
    assert_eq!(outcome(&reply), ("handled".to_owned(), "allowed".to_owned()));
    drop(session);
}

#[test]
fn an_allow_inside_the_arming_window_is_a_refusal() {
    let (session, mut relay, _input, mut shell) = session_with_claimant();

    relay.send(&portal_ui(2, &consent(30_000)));
    let offer = next_offer(&mut shell);
    // No wait: a click that was already in progress when the dialog appeared must not allow.
    shell.claim(offer.request).expect("the claimant claims the offer");
    shell.answer(offer.request, b"allow").expect("the claimant answers");

    let reply = receive_for(&mut relay, 2);
    assert_eq!(outcome(&reply), ("cancelled".to_owned(), "user".to_owned()));
    drop(session);
}

#[test]
fn a_deny_refuses() {
    let (session, mut relay, _input, mut shell) = session_with_claimant();

    relay.send(&portal_ui(3, &consent(30_000)));
    let offer = next_offer(&mut shell);
    thread::sleep(ARM_DELAY);
    shell.claim(offer.request).expect("the claimant claims the offer");
    shell.answer(offer.request, b"deny").expect("the claimant answers");

    let reply = receive_for(&mut relay, 3);
    assert_eq!(outcome(&reply), ("cancelled".to_owned(), "user".to_owned()));
    drop(session);
}

/// Presses Escape a few times after a request settled and checks no second reply follows: a
/// built-in dialog that had opened for the same request would answer the first key.
fn assert_no_built_in_dialog(relay: &mut Wire, input: &mut Wire) {
    relay.set_read_timeout(Duration::from_millis(100));
    for attempt in 0..10_u64 {
        input.send(&portal_key(2_000 + attempt, "escape", false));
        let _ack = input.receive();
        assert!(
            relay.receive_within().is_none(),
            "a second reply arrived, so a built-in dialog opened for a settled request"
        );
    }
}

#[test]
fn a_deny_is_terminal_and_the_built_in_dialog_never_opens() {
    let (session, mut relay, mut input, mut shell) = session_with_claimant();

    relay.send(&portal_ui(11, &consent(30_000)));
    let offer = next_offer(&mut shell);
    thread::sleep(ARM_DELAY);
    shell.claim(offer.request).expect("the claimant claims the offer");
    shell.answer(offer.request, b"deny").expect("the claimant answers");

    let reply = receive_for(&mut relay, 11);
    assert_eq!(outcome(&reply), ("cancelled".to_owned(), "user".to_owned()));
    assert_no_built_in_dialog(&mut relay, &mut input);
    drop(session);
}

#[test]
fn an_early_allow_is_terminal_and_the_built_in_dialog_never_opens() {
    let (session, mut relay, mut input, mut shell) = session_with_claimant();

    relay.send(&portal_ui(12, &consent(30_000)));
    let offer = next_offer(&mut shell);
    shell.claim(offer.request).expect("the claimant claims the offer");
    shell.answer(offer.request, b"allow").expect("the claimant answers");

    let reply = receive_for(&mut relay, 12);
    assert_eq!(
        outcome(&reply),
        ("cancelled".to_owned(), "user".to_owned()),
        "an allow inside the arming window is a refusal"
    );
    assert_no_built_in_dialog(&mut relay, &mut input);
    drop(session);
}

#[test]
fn an_answer_that_is_neither_allow_nor_deny_falls_back_to_the_built_in_dialog() {
    let (session, mut relay, mut input, mut shell) = session_with_claimant();

    relay.send(&portal_ui(4, &consent(30_000)));
    let offer = next_offer(&mut shell);
    thread::sleep(ARM_DELAY);
    shell.claim(offer.request).expect("the claimant claims the offer");
    // Close to `allow` is not `allow`: nothing the compositor does not understand is consent.
    shell.answer(offer.request, b"Allow").expect("the claimant answers");

    // The built-in dialog took the request, and Escape on it refuses.
    assert_eq!(
        settle_with_escape(&mut relay, &mut input, 4),
        ("cancelled".to_owned(), "user".to_owned())
    );
    drop(session);
}

#[test]
fn an_unresponsive_claimant_falls_back_to_the_built_in_dialog() {
    let (session, mut relay, mut input, mut shell) = session_with_claimant();

    // A 2 s request gives the claimant a 1 s window and the built-in dialog the rest.
    relay.send(&portal_ui(5, &consent(2_000)));
    let offer = next_offer(&mut shell);
    assert_eq!(offer.slot, "portal.ui");
    // The claimant never claims or answers. Past its window the built-in dialog serves the request.
    assert_eq!(
        settle_with_escape(&mut relay, &mut input, 5),
        ("cancelled".to_owned(), "user".to_owned())
    );
    drop(session);
}

#[test]
fn a_declining_claimant_falls_back_to_the_built_in_dialog() {
    let (session, mut relay, mut input, mut shell) = session_with_claimant();

    relay.send(&portal_ui(6, &consent(30_000)));
    let offer = next_offer(&mut shell);
    shell.claim(offer.request).expect("the claimant claims the offer");
    shell.decline(offer.request).expect("the claimant declines");

    assert_eq!(
        settle_with_escape(&mut relay, &mut input, 6),
        ("cancelled".to_owned(), "user".to_owned())
    );
    drop(session);
}

#[test]
fn a_claimant_that_disconnects_mid_request_falls_back_to_the_built_in_dialog() {
    let (session, mut relay, mut input, mut shell) = session_with_claimant();

    relay.send(&portal_ui(7, &consent(30_000)));
    let offer = next_offer(&mut shell);
    shell.claim(offer.request).expect("the claimant claims the offer");
    drop(shell);

    assert_eq!(
        settle_with_escape(&mut relay, &mut input, 7),
        ("cancelled".to_owned(), "user".to_owned())
    );
    drop(session);
}

#[test]
fn a_requester_that_cancels_ends_the_claimants_offer() {
    let (session, mut relay, _input, mut shell) = session_with_claimant();

    relay.send(&portal_ui(8, &consent(30_000)));
    let offer = next_offer(&mut shell);
    relay.send(&portal_cancel(8));

    let reply = receive_for(&mut relay, 8);
    assert_eq!(outcome(&reply), ("cancelled".to_owned(), "user".to_owned()));
    match shell.next_event().expect("the claimant is told the offer ended") {
        Event::RequestFinished { request, completion } => {
            assert_eq!(request, offer.request);
            assert_eq!(completion, RequestCompletion::Ended);
        }
        other => panic!("expected the offer to end, got {other:?}"),
    }
    // A late answer to an offer nobody waits for is refused and settles nothing.
    assert!(shell.claim(offer.request).is_err(), "the offer is over");
    drop(session);
}

#[test]
fn releasing_the_claim_restores_the_built_in_dialog() {
    let (session, mut relay, mut input, mut shell) = session_with_claimant();
    shell.release_slot("portal.ui").expect("the holder releases the slot");

    relay.send(&portal_ui(9, &consent(30_000)));
    // Nobody claims the slot, so the built-in dialog serves the request and Escape refuses it.
    assert_eq!(
        settle_with_escape(&mut relay, &mut input, 9),
        ("cancelled".to_owned(), "user".to_owned())
    );
    drop(session);
}

#[test]
fn with_no_claimant_the_built_in_dialog_serves_the_request() {
    let (session, mut ends) = Session::start_privileged(&["portal", "input"]);
    let mut relay = take(&mut ends, "portal");
    let mut input = take(&mut ends, "input");

    relay.send(&portal_ui(10, &consent(30_000)));
    assert_eq!(
        settle_with_escape(&mut relay, &mut input, 10),
        ("cancelled".to_owned(), "user".to_owned())
    );
    drop(session);
}

#[test]
fn a_second_shell_cannot_take_portal_ui() {
    let (session, _relay, _input, _shell) = session_with_claimant();
    let mut second = socket_client(&session);
    match second
        .claim_slot("portal.ui", Provider::Shell)
        .expect_err("the slot is held")
    {
        ClientError::Refused { code, .. } => assert_eq!(code, code::DENIED),
        other => panic!("expected a refusal, got {other}"),
    }
    assert!(
        second.release_slot("portal.ui").is_err(),
        "a connection that does not hold the slot cannot release it"
    );
}

#[test]
fn the_services_role_cannot_claim_portal_ui() {
    let (session, mut ends) = Session::start_privileged(&["services"]);
    let mut services = role_client(&mut ends, "services");
    match services
        .claim_slot("portal.ui", Provider::Services)
        .expect_err("the services role draws nothing")
    {
        ClientError::Refused { code, .. } => assert_eq!(code, code::DENIED),
        other => panic!("expected a refusal, got {other}"),
    }
    drop(session);
}

#[test]
fn a_none_claim_cannot_clear_another_connections_slot() {
    let (session, _relay, _input, _shell) = session_with_claimant();
    let mut intruder = socket_client(&session);
    match intruder
        .claim_slot("portal.ui", Provider::None)
        .expect_err("a none claim is refused")
    {
        ClientError::Refused { code, .. } => assert_eq!(code, code::DENIED),
        other => panic!("expected a refusal, got {other}"),
    }
}

#[test]
fn a_prompt_secret_claimant_answers_the_host() {
    let (session, mut ends) = Session::start_privileged(&["services"]);
    let mut services = role_client(&mut ends, "services");
    let mut shell = socket_client(&session);
    shell
        .claim_slot("prompt.secret", Provider::Shell)
        .expect("a shell claims the passphrase prompt");

    let request = services
        .offer(
            "prompt.secret",
            "Unlock the keyring",
            "Enter your passphrase",
            b"keyring",
            Duration::from_secs(30),
        )
        .expect("the slot has a provider");
    let offer = next_offer(&mut shell);
    assert_eq!(offer.request, request);
    assert_eq!(offer.slot, "prompt.secret");
    assert_eq!(offer.title, "Unlock the keyring");
    assert_eq!(offer.subject, b"keyring");

    shell.claim(request).expect("the claimant claims it");
    shell.answer(request, b"correct horse").expect("the claimant answers");

    let mut answered = None;
    for _ in 0..4 {
        match services.next_event().expect("the host is told how it ended") {
            Event::RequestAnswered { request: ended, answer } => {
                assert_eq!(ended, request);
                answered = Some(answer);
            }
            Event::RequestFinished { .. } => {}
            other => panic!("unexpected event {other:?}"),
        }
        if answered.is_some() {
            break;
        }
    }
    assert_eq!(
        answered.as_ref().map(|answer| answer.as_slice()),
        Some(&b"correct horse"[..]),
        "only the host reads the answer"
    );
    drop(session);
}

#[test]
fn a_prompt_secret_claimant_that_leaves_ends_the_request() {
    let (session, mut ends) = Session::start_privileged(&["services"]);
    let mut services = role_client(&mut ends, "services");
    let mut shell = socket_client(&session);
    shell
        .claim_slot("prompt.secret", Provider::Shell)
        .expect("a shell claims the passphrase prompt");

    let request = services
        .offer(
            "prompt.secret",
            "Unlock the keyring",
            "Enter your passphrase",
            b"keyring",
            Duration::from_secs(30),
        )
        .expect("the slot has a provider");
    let offer = next_offer(&mut shell);
    shell.claim(offer.request).expect("the claimant claims it");
    drop(shell);

    // The host stops waiting at once: the prompt had no one left to draw it, and no answer
    // exists for it, so the request fails closed instead of waiting out its deadline.
    match services.next_event().expect("the host is told the request ended") {
        Event::RequestFinished {
            request: ended,
            completion,
        } => {
            assert_eq!(ended, request);
            assert_eq!(completion, RequestCompletion::Ended);
        }
        other => panic!("expected the request to end, got {other:?}"),
    }
    drop(session);
}

#[test]
fn a_locked_session_refuses_a_request_even_with_a_claimant() {
    let (session, mut relay, _input, _shell) = session_with_claimant();
    let mut locker = Locker::connect(session.wayland());
    locker.lock();

    // The lock reaches the portal from the session's observation within a sweep, so a request that
    // beats it is offered to the claimant: it is withdrawn and the next one is tried. The first
    // request that settles must settle as `locked`.
    relay.set_read_timeout(Duration::from_millis(150));
    let mut refused = false;
    for request_id in 100..180_u64 {
        relay.send(&portal_ui(request_id, &consent(30_000)));
        match relay.receive_within() {
            Some(reply) if reply.error_code() == Some("locked") => {
                refused = true;
                break;
            }
            Some(other) => panic!("a request settled as {other:?} instead of waiting or being refused"),
            None => {
                relay.send(&portal_cancel(request_id));
                while relay.receive_within().is_some() {}
            }
        }
    }
    assert!(refused, "the portal never learned the session was locked");
    drop(session);
}

/// The uid this test process runs as, which a real session reads as the owner's.
fn own_uid() -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").expect("the process directory").uid()
}

/// Checks `result` is the compositor's `denied` refusal for a connection that is not the owner's.
fn assert_not_owner<T: std::fmt::Debug>(result: Result<T, ClientError>, what: &str) {
    match result {
        Err(ClientError::Refused { code, message, .. }) => {
            assert_eq!(code, code::DENIED, "{what}: refused with the wrong code");
            assert!(
                message.contains("session owner"),
                "{what}: the refusal names the owner gate, got `{message}`"
            );
        }
        other => panic!("{what}: a connection that is not the owner's was served, got {other:?}"),
    }
}

/// A session whose ordinary socket serves connections of a uid that is not the owner's, with the
/// relay and input roles and a `runtime` role connection (the owner's) that claimed `portal.ui`.
/// The last client is such a non-owner connection.
fn session_with_outsider() -> (Session, Wire, Wire, Client<UnixTransport>, Client<UnixTransport>) {
    let foreign = own_uid().wrapping_add(1);
    let (session, mut ends) = Session::start_privileged_configured(&["portal", "input", "runtime"], |config| {
        config.ipc_owner_uid = Some(foreign);
    });
    let relay = take(&mut ends, "portal");
    let input = take(&mut ends, "input");
    let mut shell = role_client(&mut ends, "runtime");
    shell
        .claim_slot("portal.ui", Provider::Shell)
        .expect("the owner's connection claims portal.ui");
    let outsider = socket_client(&session);
    (session, relay, input, shell, outsider)
}

#[test]
fn a_connection_that_is_not_the_owner_cannot_claim_or_release_a_slot() {
    // The failure this prevents: any program the owner's uid does not run, or a connection the
    // owner did not make, takes `portal.ui`, `prompt.secret` or `auth.polkit` and answers every
    // consent and passphrase prompt. The table test `slots_and_the_request_exchange_are_gated_on_
    // the_owner_connection` shows the members are listed; this shows the dispatcher refuses.
    let foreign = own_uid().wrapping_add(1);
    let (session, mut ends) = Session::start_privileged_configured(&["portal", "input"], |config| {
        config.ipc_owner_uid = Some(foreign);
    });
    let mut relay = take(&mut ends, "portal");
    let mut input = take(&mut ends, "input");
    let mut outsider = socket_client(&session);

    for slot in ["portal.ui", "prompt.secret", "auth.polkit"] {
        assert_not_owner(outsider.claim_slot(slot, Provider::Shell), "slot.claim");
        assert_not_owner(outsider.release_slot(slot), "slot.release");
    }
    // The refusal took nothing: no claimant exists, so the built-in dialog serves a request.
    relay.send(&portal_ui(20, &consent(30_000)));
    assert_eq!(
        settle_with_escape(&mut relay, &mut input, 20),
        ("cancelled".to_owned(), "user".to_owned())
    );
    drop(session);
}

#[test]
fn a_connection_that_is_not_the_owner_cannot_claim_answer_decline_or_offer_a_request() {
    // The failure this prevents: a connection that is not the owner's answers `allow` to a consent
    // request it was never offered. The owner's claimant is offered the request, and the outsider
    // names that request id.
    let (session, mut relay, _input, mut shell, mut outsider) = session_with_outsider();

    relay.send(&portal_ui(21, &consent(30_000)));
    let offer = next_offer(&mut shell);
    thread::sleep(ARM_DELAY);

    assert_not_owner(outsider.claim(offer.request), "request.claim");
    assert_not_owner(outsider.answer(offer.request, b"allow"), "request.answer");
    assert_not_owner(outsider.decline(offer.request), "request.decline");
    assert_not_owner(
        outsider.offer(
            "portal.ui",
            "Screenshot",
            "Share?",
            b"org.example",
            Duration::from_secs(5),
        ),
        "request.offer",
    );

    // Nothing the outsider sent settled the request: the owner's claimant still holds it and its
    // answer is the one that counts.
    shell.claim(offer.request).expect("the claimant still claims the offer");
    shell.answer(offer.request, b"deny").expect("the claimant answers");
    let reply = receive_for(&mut relay, 21);
    assert_eq!(
        outcome(&reply),
        ("cancelled".to_owned(), "user".to_owned()),
        "the claimant's deny settled it, not the outsider's allow"
    );
    drop(session);
}
