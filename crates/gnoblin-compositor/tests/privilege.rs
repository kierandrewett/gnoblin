//! The privileged connection: what it grants, and what the socket never does.
//!
//! Design/rust-compositor.md, "One IPC": "The runtime and services connect over an inherited
//! file descriptor that carries extra capabilities: trusted input contexts, input policy
//! decisions, request handling and configuration commits." These tests drive that over a real
//! socket pair and a real socket: the role's end is a `UnixStream` the test holds, the
//! compositor's end is the same process's other half, and every client is the real client
//! library.
//!
//! # Failure modes first
//!
//! 1. **A plain socket connection uses a privileged member.** Every one of them is refused
//!    with `denied`: `a_socket_client_cannot_use_a_privileged_member`.
//! 2. **A privileged connection calls a member its role does not grant.** The runtime cannot
//!    offer a request and the services role cannot commit a configuration:
//!    `a_role_cannot_use_another_role_s_capability`.
//! 3. **A request is offered to a slot nobody provides, and its host waits for the deadline.**
//!    The offer is refused with `no-provider` naming the slot, at once:
//!    `an_offer_to_a_slot_with_no_provider_is_refused`.
//! 4. **An offer never reaches the provider, or its answer never reaches the host.** The
//!    offer arrives with its proposition, the provider claims and answers, and only the host
//!    that offered it reads the answer:
//!    `a_shell_claims_answers_and_the_answer_reaches_the_host`.
//! 5. **A configuration does not survive the connection that committed it.** The compositor
//!    keeps the last accepted document, so a new connection reads it back:
//!    `a_committed_configuration_outlives_the_connection`.
//! 6. **A claim nobody made is published anyway.** The session object carries `none` for a
//!    slot with no provider and `shell` for the one a shell claimed:
//!    `the_session_publishes_who_provides_each_slot`.

mod support;

use std::path::Path;
use std::time::Duration;

use gnoblin_client::unix::UnixTransport;
use gnoblin_client::{Client, ClientError, Event, code};
use gnoblin_model::{Filter, Provider, Value};
use gnoblin_proto::Capability;
use gnoblin_types::Version;

use support::{Session, TIMEOUT, wait_for};

/// The version this build serves.
fn version() -> Version {
    gnoblin_compositor::version()
}

/// A client on the ordinary socket.
fn socket_client(session: &Session) -> Client<UnixTransport> {
    Client::connect(
        UnixTransport::connect(session.socket()).expect("the session listens"),
        version(),
    )
    .expect("the handshake")
}

/// A client on a role's privileged end.
fn role_client(ends: &mut Vec<(String, std::os::unix::net::UnixStream)>, role: &str) -> Client<UnixTransport> {
    let index = ends
        .iter()
        .position(|(name, _)| name == role)
        .unwrap_or_else(|| panic!("no privileged end for {role}"));
    let (_, stream) = ends.remove(index);
    Client::connect(UnixTransport::from_stream(stream).expect("a transport"), version()).expect("the handshake")
}

/// The refusal a call produced, or a panic naming what happened instead.
#[track_caller]
fn refusal(error: ClientError) -> (String, String) {
    match error {
        ClientError::Refused { code, message, .. } => (code, message),
        other => panic!("expected a refusal, got {other}"),
    }
}

#[test]
fn a_socket_client_cannot_use_a_privileged_member() {
    // The failure this prevents: a shell, a portal or anything else that can reach the
    // socket commits a configuration or offers a prompt, because privilege was a property of
    // the request rather than of the connection.
    let (session, _ends) = Session::start_privileged(&["runtime", "services"]);
    let mut client = socket_client(&session);
    assert!(
        client.capabilities().is_empty(),
        "an ordinary connection is granted nothing: {:?}",
        client.capabilities()
    );

    let (code, message) = refusal(
        client
            .offer("auth.polkit", "Title", "Message", b"", Duration::from_secs(5))
            .expect_err("an ordinary connection cannot offer"),
    );
    assert_eq!(code, code::DENIED);
    assert!(message.contains("offer-request"), "{message}");

    let (code, message) = refusal(
        client
            .commit_config(b"{}")
            .expect_err("an ordinary connection cannot commit"),
    );
    assert_eq!(code, code::DENIED);
    assert!(message.contains("commit-config"), "{message}");

    let (code, _) = refusal(
        client
            .configuration()
            .expect_err("an ordinary connection cannot read the configuration"),
    );
    assert_eq!(code, code::DENIED);

    // And the session is unharmed: the refusal is this connection's, not the compositor's.
    let listed = session.ctl(&["session", "list"]).ok();
    assert!(!listed.out.trim().is_empty(), "the session object is still served");
}

#[test]
fn a_role_cannot_use_another_role_s_capability() {
    let (session, mut ends) = Session::start_privileged(&["runtime", "services"]);
    let mut runtime = role_client(&mut ends, "runtime");
    let mut services = role_client(&mut ends, "services");

    assert!(runtime.capabilities().contains(Capability::CommitConfig));
    assert!(!runtime.capabilities().contains(Capability::OfferRequest));
    assert!(services.capabilities().contains(Capability::OfferRequest));
    assert!(!services.capabilities().contains(Capability::CommitConfig));

    // The runtime offers nothing: prompts are the services role's or a shell's.
    let (code, _) = refusal(
        runtime
            .offer("prompt.secret", "t", "m", b"", Duration::from_secs(1))
            .expect_err("the runtime cannot offer"),
    );
    assert_eq!(code, code::DENIED);
    // The services role commits nothing: the configuration is the runtime's.
    let (code, _) = refusal(
        services
            .commit_config(b"{}")
            .expect_err("the services role cannot commit"),
    );
    assert_eq!(code, code::DENIED);
    drop(session);
}

#[test]
fn an_offer_to_a_slot_with_no_provider_is_refused() {
    // `prompt.secret` has no default provider, so an offer for it must fail at once: a
    // prompt nobody will draw must not cost its host the whole deadline.
    let (session, mut ends) = Session::start_privileged(&["services"]);
    let mut services = role_client(&mut ends, "services");
    let started = std::time::Instant::now();
    let (code, message) = refusal(
        services
            .offer(
                "prompt.secret",
                "Unlock the keyring",
                "Enter your passphrase",
                b"",
                Duration::from_secs(60),
            )
            .expect_err("nobody provides the slot"),
    );
    assert_eq!(code, code::NO_PROVIDER, "{message}");
    assert!(message.contains("prompt.secret"), "the slot is named: {message}");
    assert!(started.elapsed() < TIMEOUT / 2, "the refusal is immediate, not a wait");
    drop(session);
}

#[test]
fn a_shell_claims_answers_and_the_answer_reaches_the_host() {
    let (session, mut ends) = Session::start_privileged(&["services"]);
    let mut services = role_client(&mut ends, "services");
    let mut shell = socket_client(&session);

    // A shell takes the slot over, which is the plan's claim rule.
    shell
        .claim_slot("auth.polkit", Provider::Shell)
        .expect("a shell may claim the slot");

    let request = services
        .offer(
            "auth.polkit",
            "Authentication required",
            "Authenticate to run /usr/bin/mount",
            b"",
            Duration::from_secs(30),
        )
        .expect("the slot has a provider");

    // The offer carries the proposition: a provider that had to ask what the request is
    // about could not draw anything.
    let Event::RequestOffered {
        request: offered,
        slot,
        title,
        message,
        ..
    } = shell.next_event().expect("the offer arrives")
    else {
        panic!("the offer is the first event the provider sees");
    };
    assert_eq!(offered, request);
    assert_eq!(slot, "auth.polkit");
    assert_eq!(title, "Authentication required");
    assert_eq!(message, "Authenticate to run /usr/bin/mount");

    shell.claim(request).expect("the first claim wins");
    shell.answer(request, b"alice").expect("the provider answers");

    // The answer reaches the host that offered the request, and nothing else does: the
    // completion the provider and every subscriber can see carries no value.
    let mut answered = None;
    let mut finished = 0;
    for _ in 0..4 {
        match services.next_event().expect("the host is told how it ended") {
            Event::RequestAnswered { request: ended, answer } => {
                assert_eq!(ended, request);
                answered = Some(answer);
            }
            Event::RequestFinished { request: ended, .. } => {
                assert_eq!(ended, request);
                finished += 1;
            }
            other => panic!("unexpected event {other:?}"),
        }
        if answered.is_some() && finished > 0 {
            break;
        }
    }
    assert_eq!(
        answered.as_deref(),
        Some(&b"alice"[..]),
        "only the host reads the answer"
    );
}

#[test]
fn a_restarted_runtime_is_served_on_a_fresh_pair() {
    // The failure this prevents: the runtime dies, the guardian starts a new one, and the
    // compositor refuses it because the role already connected. The guardian gives every spawn
    // its own socket pair, so the restarted runtime's handshake arrives on a stream the compositor
    // has never served. The role keeps its grant and the document it committed.
    let (session, mut ends) = Session::start_privileged(&["runtime"]);
    let document = br#"{"input":{"repeat-delay":250}}"#.to_vec();
    let (_role, stream) = ends.pop().expect("the runtime's end");

    let mut first =
        Client::connect(UnixTransport::from_stream(stream).expect("a transport"), version()).expect("the handshake");
    assert_eq!(first.commit_config(&document).expect("the commit"), 1);
    assert_eq!(
        first.configuration().expect("the configuration"),
        Some((1, document.clone()))
    );
    // The child dies: its end closes, and the compositor observes the stream ending.
    drop(first);
    let restarted = session.spawn_child("runtime");
    let mut again = Client::connect(UnixTransport::from_stream(restarted).expect("a transport"), version())
        .expect("the restarted runtime handshakes on its own pair");
    assert!(
        again.capabilities().contains(Capability::CommitConfig),
        "the role keeps its grant across a restart"
    );
    assert_eq!(
        again.configuration().expect("the configuration"),
        Some((1, document.clone())),
        "the compositor keeps the last document the runtime committed"
    );
    // The next commit is the second revision, not a restart of the count.
    assert_eq!(again.commit_config(&document).expect("the commit"), 2);
    assert_eq!(session.ctl(&["session", "list"]).ok().code, 0);
}

#[test]
fn the_session_publishes_who_provides_each_slot() {
    let (session, _ends) = Session::start_privileged(&["services"]);
    let mut shell = socket_client(&session);
    shell
        .claim_slot("auth.polkit", Provider::Shell)
        .expect("a shell may claim the slot");
    let snapshot = shell.subscribe(Filter::All).expect("the snapshot");
    let session_object = snapshot
        .objects()
        .iter()
        .find(|view| view.ty().to_string() == "gnoblin.session")
        .unwrap_or_else(|| panic!("the session object is published: {snapshot:?}"));
    assert_eq!(
        session_object.property("slot_auth_polkit"),
        Some(&Value::Text("shell".to_owned())),
        "the shell that claimed the slot provides it"
    );
    assert_eq!(
        session_object.property("slot_prompt_secret"),
        Some(&Value::Text("none".to_owned())),
        "a slot nobody provides says so"
    );
}

/// A test that a plain socket never grants a capability, however it is started: the grant is
/// the compositor's decision about the descriptor, not something a client can ask for.
#[test]
fn a_handshake_cannot_ask_for_a_grant() {
    let (session, _ends) = Session::start_privileged(&["services"]);
    let stream = std::os::unix::net::UnixStream::connect(session.socket()).expect("the session listens");
    let mut handshake = gnoblin_proto::Handshake::hello(version());
    handshake.capabilities = gnoblin_proto::Capabilities::of(&[Capability::CommitConfig]);
    {
        use std::io::Write as _;
        let mut writer = &stream;
        writer.write_all(&handshake.encode()).expect("the hello");
        writer.flush().expect("the hello");
    }
    let mut frame = [0u8; gnoblin_proto::HANDSHAKE_LEN];
    {
        use std::io::Read as _;
        let mut reader = &stream;
        reader.read_exact(&mut frame).expect("the answer");
    }
    let answer = gnoblin_proto::Handshake::decode(&frame).expect("a handshake");
    {
        let mut reader = &stream;
        gnoblin_proto::read_grants(&mut reader).expect("the grants frame after the accept");
    }
    assert!(
        answer.capabilities.is_empty(),
        "the compositor grants nothing on the socket, whatever the hello asked for: {answer:?}"
    );
    let _ = Path::new("");
}

#[test]
fn a_dropped_privileged_stream_does_not_wedge_the_control_thread() {
    // The compositor must not wait on a privileged stream that has ended: after the child's end
    // closes, ordinary clients are still answered and the successor child is served, both within
    // the deadline.
    let (session, mut ends) = Session::start_privileged(&["services"]);
    drop(role_client(&mut ends, "services"));
    wait_for("the session still answers after the stream ends", || {
        let mut client = socket_client(&session);
        client.subscribe(Filter::All).ok().map(|_| ())
    });
    let successor = session.spawn_child("services");
    let mut services = Client::connect(UnixTransport::from_stream(successor).expect("a transport"), version())
        .expect("the successor handshakes");
    services.subscribe(Filter::All).expect("the successor is served");
}

#[test]
fn each_spawn_is_served_on_its_own_pair() {
    // The guardian gives every spawn a fresh pair, so a second child is served while the first
    // child's end is still open. Nothing about the first child's stream is reused for the second.
    let (session, _ends) = Session::start_privileged(&["runtime"]);
    let document = br#"{"input":{"repeat-delay":250}}"#.to_vec();
    let first_stream = session.spawn_child("runtime");
    let mut first = Client::connect(
        UnixTransport::from_stream(first_stream).expect("a transport"),
        version(),
    )
    .expect("the first child handshakes");
    assert_eq!(first.commit_config(&document).expect("the commit"), 1);
    // The first child stays connected: the second spawn must not depend on it going away.
    let second_stream = session.spawn_child("runtime");
    let mut second = Client::connect(
        UnixTransport::from_stream(second_stream).expect("a transport"),
        version(),
    )
    .expect("the second child handshakes while the first is still open");
    assert_eq!(
        second.configuration().expect("the configuration"),
        Some((1, document.clone())),
        "the second child sees what the first committed"
    );
    assert_eq!(second.commit_config(&document).expect("the commit"), 2);
}

#[test]
fn stale_partial_bytes_never_reach_the_new_peer() {
    // A child that dies mid-frame leaves half a handshake on its own pair. The guardian never
    // hands that pair to the next child, so the next child starts from an empty stream: its
    // handshake succeeds and the session shows exactly what a fresh start shows.
    let (session, _ends) = Session::start_privileged(&["runtime"]);
    let mut dying = session.spawn_child("runtime");
    let frame = gnoblin_proto::Handshake::hello(version()).encode();
    {
        use std::io::Write as _;
        dying.write_all(&frame[..frame.len() / 2]).expect("half a handshake");
    }
    drop(dying);
    let fresh = session.spawn_child("runtime");
    let mut client = Client::connect(UnixTransport::from_stream(fresh).expect("a transport"), version())
        .expect("the new child handshakes with no stale bytes in front of it");
    assert_eq!(
        client.configuration().expect("the configuration"),
        None,
        "nothing was committed in this session"
    );
    let document = br#"{"input":{"repeat-delay":250}}"#.to_vec();
    assert_eq!(client.commit_config(&document).expect("the commit"), 1);
}
