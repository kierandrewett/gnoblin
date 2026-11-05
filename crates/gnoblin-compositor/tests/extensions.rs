//! Loaded extensions, end to end over the session's real IPC sockets.
//!
//! A provider role serves `example.ext`. A granted role calls it, gets the provider's reply
//! under its own request id, and hears the provider's events. An ungranted role and a core
//! socket are refused. The compositor reports `provider-gone` when the provider disconnects,
//! and a refused extension is absent while the session keeps serving.

mod support;

use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use gnoblin_compositor::ExtensionRoute;
use gnoblin_proto::{Envelope, Grant, Kind, Version};
use support::{Session, Wire};

const NAMESPACE: &str = "example.ext";
const PROVIDER: &str = "services";
const GRANTEE: &str = "viewer";
const STRANGER: &str = "stranger";
const DEADLINE: Duration = Duration::from_secs(5);

fn grant() -> Grant {
    Grant {
        namespace: NAMESPACE.to_owned(),
        version: Version::new(0, 1, 0),
    }
}

fn route() -> ExtensionRoute {
    ExtensionRoute {
        namespace: NAMESPACE.to_owned(),
        provider: PROVIDER.to_owned(),
    }
}

/// Takes one role's privileged end, which the guardian hands out once.
fn take(ends: &mut Vec<(String, UnixStream)>, role: &str) -> Wire {
    let index = ends
        .iter()
        .position(|(name, _)| name == role)
        .unwrap_or_else(|| panic!("the session handed out no end for {role}"));
    let (_, stream) = ends.remove(index);
    let wire = Wire::over(stream);
    wire.set_read_timeout(DEADLINE);
    wire
}

/// A session where `services` provides `example.ext`, `viewer` is granted it, and
/// `stranger` is not granted anything.
fn session() -> (Session, Vec<(String, UnixStream)>) {
    Session::start_privileged_extensions(
        &[(PROVIDER, Vec::new()), (GRANTEE, vec![grant()]), (STRANGER, Vec::new())],
        &[route()],
    )
}

#[test]
fn a_granted_client_calls_the_provider_and_hears_its_events() {
    let (_session, mut ends) = session();
    let mut provider = take(&mut ends, PROVIDER);
    let mut viewer = take(&mut ends, GRANTEE);

    viewer.send(&Envelope::request(NAMESPACE, "Greet", 7, b"hi".to_vec()));
    let forwarded = provider.receive().expect("the provider is sent the request");
    assert_eq!(forwarded.namespace, NAMESPACE);
    assert_eq!(forwarded.member, "Greet");
    assert_eq!(
        forwarded.payload(),
        Some(&b"hi"[..]),
        "the payload reaches the provider unread"
    );
    provider.send(&Envelope::reply(
        NAMESPACE,
        "Greet",
        forwarded.request_id,
        b"hello".to_vec(),
    ));

    let answer = viewer.receive().expect("the caller is sent the reply");
    assert!(matches!(answer.kind, Some(Kind::Reply(_))), "{answer:?}");
    assert_eq!(answer.request_id, 7, "the reply carries the caller's own request id");
    assert_eq!(answer.payload(), Some(&b"hello"[..]));

    provider.send(&Envelope::event(NAMESPACE, "Ticked", b"tick".to_vec()));
    let event = viewer.receive().expect("the grantee hears the provider's event");
    assert!(matches!(event.kind, Some(Kind::Event(_))), "{event:?}");
    assert_eq!(event.member, "Ticked");
    assert_eq!(event.payload(), Some(&b"tick"[..]));
}

#[test]
fn an_ungranted_role_and_a_core_socket_are_refused() {
    let (session, mut ends) = session();
    let mut provider = take(&mut ends, PROVIDER);
    let mut stranger = take(&mut ends, STRANGER);

    // The provider's event goes to grantees only, so the stranger's next frame is its own
    // refusal, not the event.
    provider.send(&Envelope::event(NAMESPACE, "Ticked", b"tick".to_vec()));
    stranger.send(&Envelope::request(NAMESPACE, "Greet", 3, Vec::new()));
    let refusal = stranger.receive().expect("the ungranted role is refused");
    assert_eq!(refusal.request_id, 3);
    assert_eq!(refusal.error_code(), Some("denied"), "{refusal:?}");

    let mut core = Wire::connect(session.socket());
    core.set_read_timeout(DEADLINE);
    core.send(&Envelope::request(NAMESPACE, "Greet", 4, Vec::new()));
    let refusal = core.receive().expect("the core socket is refused");
    assert_eq!(refusal.request_id, 4);
    assert_eq!(refusal.error_code(), Some("denied"), "{refusal:?}");
}

#[test]
fn a_caller_is_told_when_its_provider_is_gone() {
    let (_session, mut ends) = session();
    let provider = take(&mut ends, PROVIDER);
    let mut viewer = take(&mut ends, GRANTEE);
    provider.kill();

    // The compositor learns of the disconnect asynchronously. Until it has, a call may wait
    // on the dead provider; once it has, the call is answered `provider-gone`. Keep calling
    // until that answer arrives.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut id = 0;
    loop {
        id += 1;
        viewer.send(&Envelope::request(NAMESPACE, "Greet", id, Vec::new()));
        if let Some(frame) = viewer.receive_within() {
            if frame.error_code() == Some("provider-gone") {
                return;
            }
            panic!("an unexpected answer while the provider was gone: {frame:?}");
        }
        assert!(
            Instant::now() < deadline,
            "the caller was never told its provider is gone"
        );
    }
}

#[test]
fn a_route_with_no_connected_provider_answers_its_callers_provider_gone() {
    // The route names a provider role no connection holds, as when the provider never started.
    let (_session, mut ends) = Session::start_privileged_extensions(&[(GRANTEE, vec![grant()])], &[route()]);
    let mut viewer = take(&mut ends, GRANTEE);

    viewer.send(&Envelope::request(NAMESPACE, "Greet", 5, Vec::new()));
    let answer = viewer.receive().expect("the caller is answered");
    assert_eq!(answer.request_id, 5);
    assert_eq!(answer.error_code(), Some("provider-gone"), "{answer:?}");
}

#[test]
fn a_refused_extension_is_absent_and_the_session_keeps_serving() {
    // A failed load leaves the namespace out of the routes. The session must still start,
    // still answer its core socket, and answer a call to the absent namespace as unknown.
    let (session, mut ends) = Session::start_privileged_extensions(&[(GRANTEE, vec![grant()])], &[]);
    let mut core = Wire::connect(session.socket());
    core.set_read_timeout(DEADLINE);
    core.send(&Envelope::request(NAMESPACE, "Greet", 9, Vec::new()));
    let answer = core.receive().expect("the core socket is answered");
    assert_eq!(answer.request_id, 9);
    assert_eq!(answer.error_code(), Some("unknown-member"), "{answer:?}");

    let mut viewer = take(&mut ends, GRANTEE);
    viewer.send(&Envelope::request(NAMESPACE, "Greet", 10, Vec::new()));
    let answer = viewer.receive().expect("the granted role is answered");
    assert_eq!(answer.request_id, 10);
    assert_eq!(answer.error_code(), Some("unknown-member"), "{answer:?}");
}
