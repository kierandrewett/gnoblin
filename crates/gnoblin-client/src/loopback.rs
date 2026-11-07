//! An in-process pair: a client wired straight to a model, with no socket.
//!
//! This exists so the client and the graph are tested together before either
//! touches a socket. It is not a simulation: the client sends real envelopes
//! through [`crate::Transport`], the server decodes them with the same codec a
//! socket will use, and every observation travels the wire path. What changes is
//! only how the bytes get from one side to the other, so when the socket lands the
//! tests keep their meaning and only the transport under them changes.
//!
//! Two properties make it deterministic:
//!
//! - The server runs the moment the client sends, in the client's own thread, so a
//!   reply is already queued when the client reads for it. There is no scheduler to
//!   interleave and nothing to poll.
//! - Request correlation ids are allocated per link on both sides, so a test never
//!   depends on which numbers a particular sequence happened to use.

use std::cell::{Ref, RefCell};
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::io;
use std::os::fd::{BorrowedFd, OwnedFd};
use std::rc::Rc;

use gnoblin_model::{
    Fields, Graph, ModelError, ObjectId, OperationId, OperationOutcome, OwnerId, RequestBroker, RequestError,
    RequestEvent, RequestId, Subscription, Tick, TypeKey, Value,
};
use gnoblin_proto::{CodecError, Envelope, Grant, Handshake, HandshakeKind, Version};

use crate::client::Client;
use crate::error::{ConnectError, HandshakeError};
use crate::wire;

/// One frame on the in-process link. A handshake and an envelope never share a
/// queue position, exactly as on a socket.
#[derive(Clone, Debug)]
enum LinkFrame {
    Handshake(Handshake),
    Envelope(Envelope),
    /// The grants frame that follows an accept. Only sent on accept, never on refuse.
    Grants(Vec<Grant>),
}

/// One client connection's state.
struct Link {
    owner: OwnerId,
    inbound: VecDeque<LinkFrame>,
    outbound: VecDeque<LinkFrame>,
    subscriptions: Vec<Subscription>,
    /// Correlation ids this link has used for the offers it was sent.
    next_offer: u64,
}

impl Link {
    fn new(owner: OwnerId) -> Link {
        Link {
            owner,
            inbound: VecDeque::new(),
            outbound: VecDeque::new(),
            subscriptions: Vec::new(),
            next_offer: 1,
        }
    }
}

/// The compositor's side: the graph, the request broker and one link per client.
struct Inner {
    graph: Graph,
    broker: RequestBroker,
    /// What each offered request is about, so the offer frame carries the
    /// proposition. The broker owns the state machine and deliberately knows nothing
    /// about what a request means.
    propositions: BTreeMap<RequestId, (String, String, String, Vec<u8>)>,
    version: Version,
    next_owner: u64,
    links: Vec<Link>,
    /// The grants every accepted connection receives after its accept frame.
    grants: Vec<Grant>,
}

impl Inner {
    fn link(&mut self, owner: OwnerId) -> Result<&mut Link, CodecError> {
        self.links
            .iter_mut()
            .find(|link| link.owner == owner)
            .ok_or_else(disconnected)
    }

    /// Run everything a client just sent, then hand it its frames.
    fn serve(&mut self, owner: OwnerId) {
        let inbound: Vec<LinkFrame> = match self.link(owner) {
            Ok(link) => link.inbound.drain(..).collect(),
            Err(_) => return,
        };
        let mut out = Vec::new();
        for frame in inbound {
            if let LinkFrame::Envelope(envelope) = frame {
                self.handle(owner, &envelope, &mut out);
            }
        }
        self.flush_subscriptions(owner, &mut out);
        if let Ok(link) = self.link(owner) {
            for frame in out {
                link.outbound.push_back(frame);
            }
        }
        self.broadcast_requests();
    }

    /// Decode and answer one request.
    fn handle(&mut self, owner: OwnerId, envelope: &Envelope, out: &mut Vec<LinkFrame>) {
        if envelope.namespace != wire::NAMESPACE {
            out.push(refusal(
                envelope,
                wire::code::UNKNOWN_MEMBER,
                "this connection serves `gnoblin` only",
            ));
            return;
        }
        let payload = envelope.payload().unwrap_or(&[]);
        match envelope.member.as_str() {
            wire::member::SUBSCRIBE => match wire::read_subscribe(payload) {
                Ok(filter) => {
                    let subscription = self.graph.subscribe(filter);
                    // The snapshot is queued before the link starts forwarding
                    // diffs, which is the whole of the late-subscriber rule.
                    out.push(LinkFrame::Envelope(Envelope::reply(
                        wire::NAMESPACE,
                        wire::member::SUBSCRIBE,
                        envelope.request_id,
                        wire::snapshot_payload(subscription.snapshot()),
                    )));
                    if let Ok(link) = self.link(owner) {
                        link.subscriptions.push(subscription);
                    }
                }
                Err(_) => out.push(refusal(
                    envelope,
                    wire::code::UNKNOWN_MEMBER,
                    "the subscribe payload did not decode",
                )),
            },
            wire::member::CALL => match wire::read_call(payload) {
                Ok((object, method, arguments)) => match self.graph.call(object, &method, arguments) {
                    Ok(operation) => out.push(LinkFrame::Envelope(Envelope::reply(
                        wire::NAMESPACE,
                        wire::member::CALL,
                        envelope.request_id,
                        operation.id().get().to_be_bytes().to_vec(),
                    ))),
                    Err(error) => out.push(refusal(envelope, refusal_code(&error), &error.to_string())),
                },
                Err(_) => out.push(refusal(
                    envelope,
                    wire::code::UNKNOWN_MEMBER,
                    "the call payload did not decode",
                )),
            },
            wire::member::CLAIM => match wire::read_claim(payload) {
                Ok(request) => match self.broker.claim(request, owner) {
                    Ok(()) => out.push(LinkFrame::Envelope(Envelope::reply(
                        wire::NAMESPACE,
                        wire::member::CLAIM,
                        envelope.request_id,
                        Vec::new(),
                    ))),
                    Err(error) => out.push(request_refusal(envelope, &error)),
                },
                Err(_) => out.push(refusal(
                    envelope,
                    wire::code::UNKNOWN_MEMBER,
                    "the claim payload did not decode",
                )),
            },
            wire::member::DECLINE => match wire::read_decline(payload) {
                Ok((request, reason)) => match self.broker.decline_with(request, owner, reason) {
                    Ok(()) => out.push(LinkFrame::Envelope(Envelope::reply(
                        wire::NAMESPACE,
                        wire::member::DECLINE,
                        envelope.request_id,
                        Vec::new(),
                    ))),
                    Err(error) => out.push(request_refusal(envelope, &error)),
                },
                Err(_) => out.push(refusal(
                    envelope,
                    wire::code::UNKNOWN_MEMBER,
                    "the decline payload did not decode",
                )),
            },
            wire::member::ANSWER => match wire::read_answer(payload) {
                Ok((request, answer)) => match self.broker.answer(request, owner, answer) {
                    Ok(()) => out.push(LinkFrame::Envelope(Envelope::reply(
                        wire::NAMESPACE,
                        wire::member::ANSWER,
                        envelope.request_id,
                        Vec::new(),
                    ))),
                    Err(error) => out.push(request_refusal(envelope, &error)),
                },
                Err(_) => out.push(refusal(
                    envelope,
                    wire::code::UNKNOWN_MEMBER,
                    "the answer payload did not decode",
                )),
            },
            _ => out.push(refusal(
                envelope,
                wire::code::UNKNOWN_MEMBER,
                "this connection does not serve that member",
            )),
        }
    }

    /// Move every queued observation into one link's outbox.
    fn flush_subscriptions(&mut self, owner: OwnerId, out: &mut Vec<LinkFrame>) {
        let subscriptions = match self.link(owner) {
            Ok(link) => link.subscriptions.clone(),
            Err(_) => return,
        };
        for subscription in subscriptions {
            while let Some(observation) = self.graph.next(&subscription) {
                out.push(LinkFrame::Envelope(wire::observation(&observation)));
            }
        }
    }

    /// Move every queued observation into the outbox of every subscribed link.
    fn flush_all(&mut self) {
        for index in 0..self.links.len() {
            let subscriptions = self.links[index].subscriptions.clone();
            let mut out = Vec::new();
            for subscription in subscriptions {
                while let Some(observation) = self.graph.next(&subscription) {
                    out.push(LinkFrame::Envelope(wire::observation(&observation)));
                }
            }
            let link = &mut self.links[index];
            for frame in out {
                link.outbound.push_back(frame);
            }
        }
    }

    /// Announce request offers and finishes to every link.
    ///
    /// Offers go to every connection, because a request is offered to the slot
    /// rather than to a chosen connection; the first claim wins. A finish goes to
    /// every connection too, and carries no answer: only the host that offered the
    /// request reads what the provider said.
    fn broadcast_requests(&mut self) {
        let events = self.broker.events();
        for event in events {
            for index in 0..self.links.len() {
                let frame = match &event {
                    RequestEvent::Offered { request, deadline } => {
                        let correlation = self.links[index].next_offer;
                        self.links[index].next_offer += 1;
                        let (slot, title, message, subject) = self
                            .propositions
                            .get(request)
                            .cloned()
                            .unwrap_or_else(|| (String::new(), String::new(), String::new(), Vec::new()));
                        wire::offer(correlation, *request, *deadline, &slot, &title, &message, &subject)
                    }
                    RequestEvent::Finished { request, outcome } => wire::finished(*request, outcome.into()),
                };
                self.links[index].outbound.push_back(LinkFrame::Envelope(frame));
            }
        }
    }
}

/// The code a model refusal reports.
fn refusal_code(error: &ModelError) -> &'static str {
    match error {
        ModelError::UnknownObject { .. } => wire::code::OBJECT_UNKNOWN,
        _ => wire::code::UNKNOWN_MEMBER,
    }
}

/// An error frame answering a request.
fn refusal(envelope: &Envelope, code: &str, message: &str) -> LinkFrame {
    LinkFrame::Envelope(Envelope::error(
        wire::NAMESPACE,
        envelope.member.clone(),
        envelope.request_id,
        code,
        message,
    ))
}

/// An error frame for a refused request transition.
fn request_refusal(envelope: &Envelope, error: &RequestError) -> LinkFrame {
    let code = match error {
        RequestError::Taken { .. } | RequestError::NotOwner { .. } => wire::code::REQUEST_TAKEN,
        RequestError::Finished { .. } => wire::code::REQUEST_FINISHED,
        RequestError::Unknown { .. } => wire::code::REQUEST_UNKNOWN,
    };
    refusal(envelope, code, &error.to_string())
}

/// The error a link reports when its owner is gone.
fn disconnected() -> CodecError {
    CodecError::Io(io::Error::new(io::ErrorKind::NotConnected, "the link is gone"))
}

/// The compositor side of an in-process pair.
///
/// The test drives the model through this handle: it creates objects, changes
/// properties, completes operations, offers requests and supplies ticks. Everything
/// it does reaches connected clients as ordinary frames.
pub struct Server {
    inner: Rc<RefCell<Inner>>,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Server")
    }
}

impl Server {
    /// Build a server around a graph, serving one IPC version.
    pub fn new(graph: Graph, version: Version) -> Server {
        Server {
            inner: Rc::new(RefCell::new(Inner {
                graph,
                broker: RequestBroker::new(),
                propositions: BTreeMap::new(),
                version,
                next_owner: 1,
                links: Vec::new(),
                grants: Vec::new(),
            })),
        }
    }

    /// Set the grants every connection accepted from now on receives after its accept.
    /// Stock servers grant nothing; a restricted socket configures its set here.
    pub fn set_grants(&mut self, grants: Vec<Grant>) {
        self.inner.borrow_mut().grants = grants;
    }

    /// Connect a client at `version`, exchanging the handshake in process.
    pub fn connect(&mut self, version: Version) -> Result<Client<Loopback>, ConnectError> {
        let owner = {
            let mut inner = self.inner.borrow_mut();
            let owner = OwnerId::new(inner.next_owner);
            inner.next_owner += 1;
            inner.links.push(Link::new(owner));
            owner
        };
        let transport = Loopback {
            inner: Rc::clone(&self.inner),
            owner,
        };
        Client::connect(transport, version)
    }

    /// The graph, for a test that wants to read state or the pending operations.
    pub fn graph(&self) -> Ref<'_, Graph> {
        Ref::map(self.inner.borrow(), |inner| &inner.graph)
    }

    /// Create an object and publish it to every subscriber.
    pub fn create(&mut self, ty: TypeKey, properties: Fields) -> Result<ObjectId, ModelError> {
        let mut inner = self.inner.borrow_mut();
        let created = inner.graph.create(ty, properties);
        if created.is_ok() {
            inner.flush_all();
        }
        created
    }

    /// Set one property and publish the diff.
    pub fn set(&mut self, object: ObjectId, property: &str, value: Value) -> Result<(), ModelError> {
        let mut inner = self.inner.borrow_mut();
        let changed = inner.graph.set(object, property, value);
        if changed.is_ok() {
            inner.flush_all();
        }
        changed.map(|_| ())
    }

    /// Raise a declared signal.
    pub fn signal(&mut self, object: ObjectId, signal: &str, fields: Fields) -> Result<(), ModelError> {
        let mut inner = self.inner.borrow_mut();
        let raised = inner.graph.signal(object, signal, fields);
        if raised.is_ok() {
            inner.flush_all();
        }
        raised.map(|_| ())
    }

    /// End an object's life.
    pub fn close(&mut self, object: ObjectId) -> Result<(), ModelError> {
        let mut inner = self.inner.borrow_mut();
        let closed = inner.graph.close(object);
        if closed.is_ok() {
            inner.flush_all();
        }
        closed.map(|_| ())
    }

    /// Complete a pending operation once.
    pub fn complete(&mut self, operation: OperationId, outcome: OperationOutcome) -> Result<(), ModelError> {
        let mut inner = self.inner.borrow_mut();
        let completed = inner.graph.complete(operation, outcome);
        if completed.is_ok() {
            inner.flush_all();
        }
        completed.map(|_| ())
    }

    /// Offer a request that fails at `deadline`, announcing it to every client.
    pub fn offer(&mut self, slot: &str, title: &str, message: &str, subject: &[u8], deadline: Tick) -> RequestId {
        let mut inner = self.inner.borrow_mut();
        let request = inner.broker.offer(deadline);
        inner.propositions.insert(
            request,
            (slot.to_owned(), title.to_owned(), message.to_owned(), subject.to_vec()),
        );
        inner.broadcast_requests();
        request
    }

    /// Fail every request at or past its deadline.
    pub fn tick(&mut self, now: Tick) -> usize {
        let mut inner = self.inner.borrow_mut();
        let expired = inner.broker.tick(now);
        inner.broadcast_requests();
        expired
    }

    /// The completion of a finished request.
    ///
    /// The answer value is not kept in the broker's tombstone, so it is not returned
    /// here: the offering host reads it once from the event stream.
    pub fn finished(&self, request: RequestId) -> Option<gnoblin_model::RequestCompletion> {
        let inner = self.inner.borrow();
        match inner.broker.state(request)? {
            gnoblin_model::RequestState::Finished(completion) => Some(completion),
            _ => None,
        }
    }
}

/// One client connection on an in-process link.
pub struct Loopback {
    inner: Rc<RefCell<Inner>>,
    owner: OwnerId,
}

impl std::fmt::Debug for Loopback {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Loopback")
    }
}

impl crate::transport::Transport for Loopback {
    fn send_handshake(&mut self, frame: &Handshake) -> Result<(), HandshakeError> {
        if frame.kind != HandshakeKind::Hello {
            return Err(HandshakeError::WrongKind {
                expected: HandshakeKind::Hello,
                got: frame.kind,
            });
        }
        let mut inner = self.inner.borrow_mut();
        let version = inner.version;
        let accepted = gnoblin_proto::check(version, frame.version).is_ok();
        let reply = if accepted {
            Handshake::accept(version)
        } else {
            Handshake::refuse(version)
        };
        let grants = if accepted { inner.grants.clone() } else { Vec::new() };
        let link = inner.link(self.owner)?;
        link.inbound.push_back(LinkFrame::Handshake(*frame));
        link.outbound.push_back(LinkFrame::Handshake(reply));
        if accepted {
            link.outbound.push_back(LinkFrame::Grants(grants));
        }
        Ok(())
    }

    fn receive_handshake(&mut self) -> Result<Handshake, HandshakeError> {
        let mut inner = self.inner.borrow_mut();
        match inner.link(self.owner)?.outbound.pop_front() {
            Some(LinkFrame::Handshake(handshake)) => Ok(handshake),
            Some(LinkFrame::Envelope(_) | LinkFrame::Grants(_)) => Err(HandshakeError::NotAHandshake),
            None => Err(HandshakeError::Codec(disconnected())),
        }
    }

    fn receive_grants(&mut self) -> Result<Vec<Grant>, HandshakeError> {
        let mut inner = self.inner.borrow_mut();
        match inner.link(self.owner)?.outbound.pop_front() {
            Some(LinkFrame::Grants(grants)) => Ok(grants),
            Some(LinkFrame::Handshake(_)) => Err(HandshakeError::Codec(CodecError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "a handshake arrived where the grants frame belonged",
            )))),
            Some(LinkFrame::Envelope(_)) => Err(HandshakeError::NotAHandshake),
            None => Err(HandshakeError::Codec(disconnected())),
        }
    }
}

impl gnoblin_proto::Transport for Loopback {
    fn send(&mut self, envelope: &Envelope, fds: &[BorrowedFd<'_>]) -> Result<(), CodecError> {
        if !fds.is_empty() {
            return Err(CodecError::Io(io::Error::new(
                io::ErrorKind::Unsupported,
                "the in-process link carries no descriptors",
            )));
        }
        let mut inner = self.inner.borrow_mut();
        inner
            .link(self.owner)?
            .inbound
            .push_back(LinkFrame::Envelope(envelope.clone()));
        inner.serve(self.owner);
        Ok(())
    }

    fn receive(&mut self, _fds: &mut Vec<OwnedFd>) -> Result<Option<Envelope>, CodecError> {
        // The link never produces descriptors, so the caller's vector is left as it was.
        let mut inner = self.inner.borrow_mut();
        match inner.link(self.owner)?.outbound.pop_front() {
            Some(LinkFrame::Envelope(envelope)) => Ok(Some(envelope)),
            Some(LinkFrame::Handshake(_) | LinkFrame::Grants(_)) => Err(CodecError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "a handshake arrived after the connection started",
            ))),
            None => Ok(None),
        }
    }
}
