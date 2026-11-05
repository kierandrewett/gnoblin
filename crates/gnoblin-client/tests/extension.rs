//! Extension calls and events through the client, driven by a scripted peer.
//!
//! The in-process loopback serves only the core namespace, so these tests stand a
//! peer in front of the byte transport: every envelope the client sends is handed to
//! a responder that answers with the frames a provider or the compositor would send.
//! The client above the transport runs unchanged.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::os::fd::{BorrowedFd, OwnedFd};
use std::rc::Rc;

use gnoblin_client::{Client, ClientError, Event, HandshakeError, Transport, wire};
use gnoblin_model::{Fields, Value};
use gnoblin_proto::{CodecError, Envelope, Grant, Handshake, HandshakeKind, Version};

const VERSION: Version = Version::new(0, 2, 0);
const NAMESPACE: &str = "example.ext";

/// Answers one sent envelope with the frames the peer sends back, in order.
type Responder = Box<dyn FnMut(&Envelope) -> Vec<Envelope>>;

/// A transport that records what the client sends and answers through a responder.
struct ScriptedPeer {
    inbound: VecDeque<Envelope>,
    sent: Rc<RefCell<Vec<Envelope>>>,
    responder: Responder,
}

impl gnoblin_proto::Transport for ScriptedPeer {
    fn send(&mut self, envelope: &Envelope, _fds: &[BorrowedFd<'_>]) -> Result<(), CodecError> {
        self.sent.borrow_mut().push(envelope.clone());
        let answers = (self.responder)(envelope);
        self.inbound.extend(answers);
        Ok(())
    }

    fn receive(&mut self, _fds: &mut Vec<OwnedFd>) -> Result<Option<Envelope>, CodecError> {
        Ok(self.inbound.pop_front())
    }
}

impl Transport for ScriptedPeer {
    fn send_handshake(&mut self, frame: &Handshake) -> Result<(), HandshakeError> {
        assert_eq!(frame.kind, HandshakeKind::Hello);
        Ok(())
    }

    fn receive_handshake(&mut self) -> Result<Handshake, HandshakeError> {
        Ok(Handshake::accept(VERSION))
    }

    fn receive_grants(&mut self) -> Result<Vec<Grant>, HandshakeError> {
        Ok(Vec::new())
    }
}

/// A connected client whose peer answers with `responder`, and the log of what it sent.
fn connect(responder: Responder) -> (Client<ScriptedPeer>, Rc<RefCell<Vec<Envelope>>>) {
    let sent = Rc::new(RefCell::new(Vec::new()));
    let peer = ScriptedPeer {
        inbound: VecDeque::new(),
        sent: Rc::clone(&sent),
        responder,
    };
    let client = Client::connect(peer, VERSION).expect("the versions match");
    (client, sent)
}

fn text(name: &str, value: &str) -> Fields {
    let mut fields = Fields::new();
    fields.insert(name.to_owned(), Value::Text(value.to_owned()));
    fields
}

#[test]
fn a_call_returns_the_reply_fields() {
    let (mut client, _sent) = connect(Box::new(|request| {
        vec![Envelope::reply(
            NAMESPACE,
            request.member.clone(),
            request.request_id,
            wire::encode_fields(&text("greeting", "hello, world")),
        )]
    }));

    let reply = client
        .call_extension(NAMESPACE, "Probe.greet", &text("name", "world"))
        .expect("the provider replies");
    assert_eq!(reply, text("greeting", "hello, world"));
}

#[test]
fn a_call_sends_the_namespace_member_and_encoded_arguments() {
    let (mut client, sent) = connect(Box::new(|request| {
        vec![Envelope::reply(
            NAMESPACE,
            request.member.clone(),
            request.request_id,
            wire::encode_fields(&Fields::new()),
        )]
    }));
    let arguments = text("name", "world");
    client
        .call_extension(NAMESPACE, "Probe.greet", &arguments)
        .expect("the provider replies");

    let sent = sent.borrow();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].namespace, NAMESPACE);
    assert_eq!(sent[0].member, "Probe.greet");
    let payload = sent[0].payload().expect("a request frame carries a payload");
    assert_eq!(wire::decode_fields(payload).expect("the arguments decode"), arguments);
}

#[test]
fn a_refusal_arrives_with_the_providers_code_and_message() {
    let (mut client, _sent) = connect(Box::new(|request| {
        vec![Envelope::error(
            NAMESPACE,
            request.member.clone(),
            request.request_id,
            "probe-unknown",
            "no probe 7",
        )]
    }));

    match client.call_extension(NAMESPACE, "Probe.greet", &text("name", "world")) {
        Err(ClientError::Refused { member, code, message }) => {
            assert_eq!(member, "Probe.greet");
            assert_eq!(code, "probe-unknown");
            assert_eq!(message, "no probe 7");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn an_extension_event_arriving_during_a_call_is_queued_as_an_extension_event() {
    let (mut client, _sent) = connect(Box::new(|request| {
        vec![
            Envelope::event(NAMESPACE, "Probe.changed", wire::encode_fields(&text("state", "moved"))),
            Envelope::reply(
                NAMESPACE,
                request.member.clone(),
                request.request_id,
                wire::encode_fields(&Fields::new()),
            ),
        ]
    }));

    client
        .call_extension(NAMESPACE, "Probe.greet", &text("name", "world"))
        .expect("the provider replies");
    match client.next_event().expect("the event was queued") {
        Event::Extension {
            namespace,
            member,
            fields,
        } => {
            assert_eq!(namespace, NAMESPACE);
            assert_eq!(member, "Probe.changed");
            assert_eq!(fields, text("state", "moved"));
        }
        other => panic!("expected an extension event, got {other:?}"),
    }
}

#[test]
fn a_foreign_reply_that_answers_no_outstanding_request_is_dropped() {
    let (mut client, _sent) = connect(Box::new(|request| {
        vec![
            // A reply to an id this client never sent: not the answer, and not an event.
            Envelope::reply("other.ext", "Other.method", request.request_id + 100, Vec::new()),
            Envelope::reply(
                NAMESPACE,
                request.member.clone(),
                request.request_id,
                wire::encode_fields(&text("ok", "yes")),
            ),
        ]
    }));

    let reply = client
        .call_extension(NAMESPACE, "Probe.greet", &text("name", "world"))
        .expect("the matching reply answers the call");
    assert_eq!(reply, text("ok", "yes"));
    assert!(matches!(client.next_event(), Err(ClientError::Disconnected)));
}

#[test]
fn a_foreign_event_arriving_during_a_call_is_queued_too() {
    // The responder sends an event from a namespace this client never called, then the
    // reply: the event is queued as an extension event, the call still returns.
    let (mut client, _sent) = connect(Box::new(|request| {
        vec![
            Envelope::event("other.ext", "Other.tick", wire::encode_fields(&text("n", "1"))),
            Envelope::reply(
                NAMESPACE,
                request.member.clone(),
                request.request_id,
                wire::encode_fields(&Fields::new()),
            ),
        ]
    }));
    client
        .call_extension(NAMESPACE, "Probe.greet", &Fields::new())
        .expect("the reply answers the call");
    match client.next_event().expect("the foreign event was queued") {
        Event::Extension { namespace, member, .. } => {
            assert_eq!(namespace, "other.ext");
            assert_eq!(member, "Other.tick");
        }
        other => panic!("expected an extension event, got {other:?}"),
    }
}
