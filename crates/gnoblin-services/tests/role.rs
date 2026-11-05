//! The services role, over a real socket pair.
//!
//! The role is the compositor's privileged client, so the peer in these tests is the
//! compositor's side of that pair: it accepts the handshake with the grant a services
//! connection receives, answers the frames the role sends and pushes the frames the role has
//! to react to. Nothing is faked on the role's side — it is the shipped `Role`, its shipped
//! client and its shipped state machines.
//!
//! # Failure modes first
//!
//! 1. **The guardian did not pass a connection.** The role refuses at once and names the
//!    variable: `the_role_refuses_without_a_privileged_connection`.
//! 2. **An offer arrives for a slot this role provides and cannot draw.** It is claimed and
//!    declined immediately, so the host that needed the decision is not left waiting:
//!    `an_offer_the_role_cannot_draw_is_declined_at_once`.
//! 3. **The role never says who provides its slots.** The three the plan gives it a default
//!    for are claimed on the connection, and published by the compositor:
//!    `the_role_claims_the_slots_the_plan_gives_it`.

use std::io::{BufReader, Write as _};
use std::os::fd::IntoRawFd as _;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gnoblin_client::wire;
use gnoblin_model::{RequestId, Tick};
use gnoblin_proto::{Capabilities, Capability, Envelope, Handshake, HandshakeKind, read_frame, write_frame};
use gnoblin_services::{Buses, Role};

/// The version this test's peer advertises.
const VERSION: (u32, u32, u32) = gnoblin_proto::IPC_VERSION;

/// The compositor's side of one privileged pair.
struct Peer {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Peer {
    /// Answer the client's hello with the grant a services connection holds.
    fn accept(stream: UnixStream) -> Peer {
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .expect("the peer's deadline");
        let mut peer = Peer {
            reader: BufReader::new(stream.try_clone().expect("a duplicated socket")),
            stream,
        };
        let hello = peer.handshake();
        assert_eq!(hello.kind, HandshakeKind::Hello, "the role opens with a hello");
        let grant = Capabilities::of(&[Capability::OfferRequest]);
        let answer = Handshake::grant(gnoblin_proto::Version::new(VERSION.0, VERSION.1, VERSION.2), grant);
        peer.stream.write_all(&answer.encode()).expect("the accept");
        peer.stream.flush().expect("the accept");
        gnoblin_proto::write_grants(&mut peer.stream, &[]).expect("the grants frame after the accept");
        peer
    }

    fn handshake(&mut self) -> Handshake {
        let mut frame = [0u8; gnoblin_proto::HANDSHAKE_LEN];
        std::io::Read::read_exact(&mut self.reader, &mut frame).expect("a handshake");
        Handshake::decode(&frame).expect("a valid handshake")
    }

    /// The next frame, or `None` when the role closed the connection.
    /// The next frame, or a failure naming what the role did instead of sending one.
    fn next(&mut self) -> Option<Envelope> {
        match read_frame(&mut self.reader) {
            Ok(frame) => frame,
            Err(error) => panic!("the role sent nothing before the peer's deadline: {error}"),
        }
    }

    /// Answer one request frame with a payload.
    fn reply(&mut self, envelope: &Envelope, payload: Vec<u8>) {
        let reply = Envelope::reply(
            &envelope.namespace,
            envelope.member.clone(),
            envelope.request_id,
            payload,
        );
        write_frame(&mut self.stream, &reply).expect("the reply");
        self.stream.flush().expect("the reply");
    }

    /// Push one frame the role has to react to.
    fn send(&mut self, envelope: &Envelope) {
        write_frame(&mut self.stream, envelope).expect("the frame");
        self.stream.flush().expect("the frame");
    }
}

/// One running role, its stop flag and the thread it owns.
struct Running {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<Result<(), gnoblin_services::RoleError>>>,
}

impl Running {
    fn start(number: i32) -> Running {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let role = Role {
            env: Box::new(move |name| (name == "GNOBLIN_IPC_FD").then(|| number.to_string())),
            deadline: std::time::Duration::from_secs(30),
            buses: Buses::None,
        };
        let thread = std::thread::Builder::new()
            .name("services-role".to_owned())
            .spawn(move || role.run(move || flag.load(Ordering::SeqCst), Vec::new()))
            .expect("the role thread");
        Running {
            stop,
            thread: Some(thread),
        }
    }

    /// Ask the role to stop and report how it ended.
    fn finish(mut self) -> Result<(), gnoblin_services::RoleError> {
        self.stop.store(true, Ordering::SeqCst);
        self.thread
            .take()
            .expect("the role thread")
            .join()
            .expect("the role did not panic")
    }
}

/// The frames the role sends before it settles: one claim per slot it provides.
fn claims(peer: &mut Peer) -> Vec<String> {
    let mut slots = Vec::new();
    while let Some(envelope) = peer.next() {
        match envelope.member.as_str() {
            wire::member::SLOT_CLAIM => {
                let (slot, provider) = wire::read_slot_claim(envelope.payload().unwrap_or(&[])).expect("a claim");
                assert_eq!(
                    provider,
                    gnoblin_services::Provider::Services,
                    "the role claims its defaults as the services provider"
                );
                slots.push(slot);
                peer.reply(&envelope, Vec::new());
            }
            _ => panic!("unexpected frame `{}`", envelope.member),
        }
        if slots.len() == 3 {
            break;
        }
    }
    slots
}

#[test]
fn the_role_refuses_without_a_privileged_connection() {
    let role = Role {
        env: Box::new(|_| None),
        deadline: std::time::Duration::from_secs(1),
        buses: Buses::None,
    };
    let error = role.run(|| true, Vec::new()).expect_err("no connection");
    let message = error.to_string();
    assert!(message.contains("GNOBLIN_IPC_FD"), "{message}");
}

#[test]
fn the_role_claims_the_slots_the_plan_gives_it() {
    let (role_end, peer_end) = UnixStream::pair().expect("a socket pair");
    let running = Running::start(role_end.into_raw_fd());
    let mut peer = Peer::accept(peer_end);
    let mut claimed = claims(&mut peer);
    claimed.sort();
    assert_eq!(
        claimed,
        vec![
            "auth.polkit".to_owned(),
            "idle.policy".to_owned(),
            "location.agent".to_owned()
        ],
        "the three slots whose default provider the plan gives this role"
    );
    running.finish().expect("the role ends cleanly");
}

#[test]
fn an_offer_the_role_cannot_draw_is_declined_at_once() {
    // The failure this prevents: the services role is the polkit slot's default provider, it
    // draws nothing, and the offer sits until its deadline so polkitd reports "Not
    // authorized" only after two minutes.
    let (role_end, peer_end) = UnixStream::pair().expect("a socket pair");
    let running = Running::start(role_end.into_raw_fd());
    let mut peer = Peer::accept(peer_end);
    claims(&mut peer);

    // A request the role is the provider of.
    peer.send(&wire::offer(
        0,
        RequestId::new(7),
        Tick::new(120_000),
        "auth.polkit",
        "Authentication required",
        "Authenticate to run /usr/bin/mount",
    ));

    // It claims the request, because only its owner may end it, and then declines it.
    let claim = peer.next().expect("the claim");
    assert_eq!(claim.member, wire::member::CLAIM, "{claim:?}");
    assert_eq!(
        wire::read_claim(claim.payload().unwrap_or(&[])).expect("a claim"),
        RequestId::new(7)
    );
    peer.reply(&claim, Vec::new());

    let decline = peer.next().expect("the decline");
    assert_eq!(decline.member, wire::member::DECLINE, "{decline:?}");
    assert_eq!(
        wire::read_decline(decline.payload().unwrap_or(&[])).expect("a decline"),
        RequestId::new(7)
    );
    peer.reply(&decline, Vec::new());

    running.finish().expect("the role ends cleanly");
}

#[test]
fn an_answer_reaches_the_agent_that_offered_the_request() {
    // The role's own offers are answered by a shell; the answer comes back as one frame and
    // only the role sees its value. An unknown request is refused by both agents, which is a
    // refusal and not a failure of the connection.
    let (role_end, peer_end) = UnixStream::pair().expect("a socket pair");
    let running = Running::start(role_end.into_raw_fd());
    let mut peer = Peer::accept(peer_end);
    claims(&mut peer);

    peer.send(&wire::answered(RequestId::new(99), b"alice"));
    peer.send(&wire::finished(
        RequestId::new(99),
        gnoblin_model::RequestCompletion::Answered,
    ));

    // The role is still serving: a frame for a request no agent offered changes nothing, and
    // the connection stays usable.
    peer.send(&wire::offer(
        0,
        RequestId::new(8),
        Tick::new(120_000),
        "location.agent",
        "Location",
        "Allow this application to use your location",
    ));
    let claim = peer.next().expect("the claim");
    assert_eq!(claim.member, wire::member::CLAIM, "{claim:?}");
    peer.reply(&claim, Vec::new());
    let decline = peer.next().expect("the decline");
    assert_eq!(decline.member, wire::member::DECLINE, "{decline:?}");
    peer.reply(&decline, Vec::new());

    running.finish().expect("the role ends cleanly");
}
