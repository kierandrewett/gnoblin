//! The IPC client: handshake, subscribe, call, await, claim.
//!
//! A `Client` owns one connection and speaks only the object model on it. It never
//! interprets what a property means and never decides policy: it presents the
//! compositor's observations in order and hands a caller the operations and
//! requests it asked for.
//!
//! Two rules make it usable without an async runtime:
//!
//! - Every read path is the same read path. A reply that arrives while an event is
//!   expected is consumed and its slot filled; an observation that arrives while a
//!   reply is awaited is queued in order. Nothing is dropped and nothing is
//!   reordered, so a caller may interleave [`Client::next_event`],
//!   [`Client::await_operation`] and further calls freely.
//! - Waiting is a read, not a timer. [`Client::await_operation`] reads frames until
//!   the operation's one completion arrives; the caller supplies the connection
//!   behaviour, so a blocking socket blocks and the in-process loopback does not.
//!
//! Descriptors that arrive beside a frame are kept for the caller to take
//! ([`Client::take_descriptors`]) rather than closed on the floor. Nothing in this
//! layer sends one yet: the members that carry a frame or an fd arrive with
//! `gnoblin-capture`.

use std::collections::VecDeque;
use std::os::fd::OwnedFd;

use gnoblin_model::{
    Fields, Filter, ObjectId, Observation, Operation, OperationOutcome, RequestCompletion, RequestId, Snapshot, Tick,
};
use gnoblin_proto::{Envelope, Handshake, HandshakeKind, Kind, Mismatch, Version};

use crate::error::{ClientError, ConnectError, HandshakeError, WireError};
use crate::transport::Transport;
use crate::wire;

/// Something the compositor told this client.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// A change to the object graph, in revision order.
    Model(Observation),
    /// A request this client may claim.
    RequestOffered {
        /// The new request.
        request: RequestId,
        /// When it fails if nobody claims and answers it.
        deadline: Tick,
    },
    /// A request ended. The completion carries no answer value, because an answer
    /// may be secret and only the host that offered the request reads it.
    RequestFinished {
        /// The request that ended.
        request: RequestId,
        /// How it ended.
        completion: RequestCompletion,
    },
}

/// One connection to the compositor.
#[derive(Debug)]
pub struct Client<T: Transport> {
    transport: T,
    version: Version,
    next_request: u64,
    inbox: VecDeque<Event>,
    descriptors: Vec<OwnedFd>,
}

impl<T: Transport> Client<T> {
    /// Exchange the handshake and take the connection.
    ///
    /// Both directions are checked: a refusal names the compositor's version, and an
    /// accept whose version cannot serve this build is refused here rather than
    /// failing later on a member that moved.
    pub fn connect(mut transport: T, version: Version) -> Result<Client<T>, ConnectError> {
        transport.send_handshake(&Handshake::hello(version))?;
        let answer = transport.receive_handshake()?;
        match answer.kind {
            HandshakeKind::Accept => {
                gnoblin_proto::check(answer.version, version).map_err(ConnectError::Refused)?;
                Ok(Client {
                    transport,
                    version: answer.version,
                    next_request: 1,
                    inbox: VecDeque::new(),
                    descriptors: Vec::new(),
                })
            }
            HandshakeKind::Refuse => Err(ConnectError::Refused(Mismatch {
                server: answer.version,
                client: version,
            })),
            HandshakeKind::Hello => Err(ConnectError::Handshake(HandshakeError::WrongKind {
                expected: HandshakeKind::Accept,
                got: HandshakeKind::Hello,
            })),
        }
    }

    /// The IPC version the compositor serves, which is also the version this client
    /// negotiated.
    pub fn version(&self) -> Version {
        self.version
    }

    /// Take any descriptors that arrived beside frames.
    ///
    /// The transport appends them to the caller; the client holds them so a caller
    /// that wants them can take them and anything else is closed when the client
    /// drops.
    pub fn take_descriptors(&mut self) -> Vec<OwnedFd> {
        std::mem::take(&mut self.descriptors)
    }

    /// Subscribe, receiving the snapshot to start from.
    ///
    /// Every observation [`Client::next_event`] returns afterwards is later than the
    /// snapshot's revision, exactly once and in order: the compositor sends the
    /// snapshot before it forwards a single diff.
    pub fn subscribe(&mut self, filter: Filter) -> Result<Snapshot, ClientError> {
        let request = self.next_id();
        let frame = wire::subscribe(request, filter);
        self.send(&frame)?;
        let payload = self.await_reply(request)?;
        wire::read_snapshot(&payload).map_err(|reason| ClientError::Protocol {
            member: wire::member::SUBSCRIBE.to_owned(),
            reason,
        })
    }

    /// Call a declared method, returning the operation its completion is matched on.
    pub fn call(&mut self, object: ObjectId, method: &str, arguments: &Fields) -> Result<Operation, ClientError> {
        let request = self.next_id();
        let frame = wire::call(request, object, method, arguments);
        self.send(&frame)?;
        let payload = self.await_reply(request)?;
        let operation = wire::read_operation(&payload).map_err(|reason| ClientError::Protocol {
            member: wire::member::CALL.to_owned(),
            reason,
        })?;
        Ok(Operation::new(operation, object))
    }

    /// Read frames until the operation's single completion arrives.
    ///
    /// Events read on the way stay in the inbox, so a caller that waits for one
    /// operation does not lose the changes that happened meanwhile.
    pub fn await_operation(&mut self, operation: Operation) -> Result<OperationOutcome, ClientError> {
        loop {
            if let Some(outcome) = self.inbox.iter().find_map(|event| match event {
                Event::Model(Observation::Finished {
                    operation: completed,
                    outcome,
                    ..
                }) if *completed == operation.id() => Some(outcome.clone()),
                _ => None,
            }) {
                return Ok(outcome);
            }
            let envelope = self.receive()?;
            self.deliver(&envelope)?;
        }
    }

    /// The next event, in the order the compositor sent it.
    pub fn next_event(&mut self) -> Result<Event, ClientError> {
        if let Some(event) = self.inbox.pop_front() {
            return Ok(event);
        }
        loop {
            let envelope = self.receive()?;
            self.deliver(&envelope)?;
            if let Some(event) = self.inbox.pop_front() {
                return Ok(event);
            }
        }
    }

    /// Claim an offered request.
    ///
    /// Exactly one owner can win; a refusal arrives as
    /// [`ClientError::Refused`] with [`crate::code::REQUEST_TAKEN`] when another
    /// owner holds it and [`crate::code::REQUEST_FINISHED`] when it already ended.
    pub fn claim(&mut self, request: RequestId) -> Result<(), ClientError> {
        let correlation = self.next_id();
        let frame = wire::claim(correlation, request);
        self.send(&frame)?;
        let _ = self.await_reply(correlation)?;
        Ok(())
    }

    /// Answer a request this client claimed.
    ///
    /// An empty answer is a legitimate answer: the provider chose with no content.
    /// Use [`Client::decline`] instead when the provider will not answer.
    pub fn answer(&mut self, request: RequestId, answer: &[u8]) -> Result<(), ClientError> {
        let correlation = self.next_id();
        let frame = wire::answer(correlation, request, answer);
        self.send(&frame)?;
        let _ = self.await_reply(correlation)?;
        Ok(())
    }

    /// Decline a request this client claimed: the provider will not answer it.
    ///
    /// This is the refusal and provider-gone outcome, and it is a different member
    /// from [`Client::answer`] so the two can never be confused.
    pub fn decline(&mut self, request: RequestId) -> Result<(), ClientError> {
        let correlation = self.next_id();
        let frame = wire::decline(correlation, request);
        self.send(&frame)?;
        let _ = self.await_reply(correlation)?;
        Ok(())
    }

    /// Give the connection back, for a caller that owns the socket.
    pub fn into_inner(self) -> T {
        self.transport
    }

    /// Send one request frame.
    fn send(&mut self, envelope: &Envelope) -> Result<(), ClientError> {
        self.transport.send(envelope, &[]).map_err(ClientError::Codec)
    }

    /// Take one frame from the transport.
    fn receive(&mut self) -> Result<Envelope, ClientError> {
        let mut descriptors = Vec::new();
        let outcome = self.transport.receive(&mut descriptors);
        self.descriptors.append(&mut descriptors);
        match outcome {
            Ok(Some(envelope)) => Ok(envelope),
            Ok(None) => Err(ClientError::Disconnected),
            Err(error) => Err(ClientError::Codec(error)),
        }
    }

    /// Read until the reply to `request` arrives, queueing events on the way.
    fn await_reply(&mut self, request: u64) -> Result<Vec<u8>, ClientError> {
        loop {
            let envelope = self.receive()?;
            // Only a reply or an error can answer this request. An offer the
            // compositor sent in the other direction may happen to carry the same
            // correlation id, and it is delivered as the event it is.
            let answers = matches!(&envelope.kind, Some(Kind::Reply(_)) | Some(Kind::Error(_)));
            if answers && envelope.request_id == request {
                return match envelope.kind {
                    Some(Kind::Reply(reply)) => Ok(reply.payload),
                    Some(Kind::Error(error)) => Err(ClientError::Refused {
                        member: envelope.member,
                        code: error.code,
                        message: error.message,
                    }),
                    _ => Err(ClientError::Protocol {
                        member: envelope.member,
                        reason: WireError::Kind,
                    }),
                };
            }
            self.deliver(&envelope)?;
        }
    }

    /// Decode a frame into a queued event.
    fn deliver(&mut self, envelope: &Envelope) -> Result<(), ClientError> {
        let decoded = wire::decode(envelope).map_err(|reason| ClientError::Protocol {
            member: envelope.member.clone(),
            reason,
        })?;
        match decoded {
            None => {}
            Some(wire::Incoming::Observation(observation)) => self.inbox.push_back(Event::Model(observation)),
            Some(wire::Incoming::Offered { request, deadline }) => {
                self.inbox.push_back(Event::RequestOffered { request, deadline });
            }
            Some(wire::Incoming::Finished { request, completion }) => {
                self.inbox.push_back(Event::RequestFinished { request, completion });
            }
        }
        Ok(())
    }

    /// The next correlation id for a request this client sends.
    fn next_id(&mut self) -> u64 {
        let id = self.next_request;
        self.next_request += 1;
        id
    }
}
