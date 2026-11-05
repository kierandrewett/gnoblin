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
    Fields, Filter, ObjectId, Observation, Operation, OperationOutcome, Provider, RequestCompletion, RequestId,
    Snapshot, Tick,
};
use gnoblin_proto::{Capabilities, Envelope, Grant, Handshake, HandshakeKind, Kind, Mismatch, Version};

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
        /// The slot the request was offered to.
        slot: String,
        /// The title the provider shows.
        title: String,
        /// The sentence the provider shows.
        message: String,
    },
    /// The answer to a request this client offered. Only the host that offered the
    /// request receives it, and the frame it arrived in is flagged sensitive.
    RequestAnswered {
        /// The request.
        request: RequestId,
        /// The provider's answer, in the slot's own encoding. The bytes may be a
        /// passphrase, so nothing here logs them.
        answer: Vec<u8>,
    },
    /// A request ended. The completion carries no answer value, because an answer
    /// may be secret and only the host that offered the request reads it.
    RequestFinished {
        /// The request that ended.
        request: RequestId,
        /// How it ended.
        completion: RequestCompletion,
    },
    /// An event a loaded extension sent to a connection it was granted to. The
    /// member is `Object.Signal`, and the fields are the signal's payload.
    Extension {
        /// The extension namespace that sent the event.
        namespace: String,
        /// The member, `Object.Signal`.
        member: String,
        /// The signal's fields, decoded with the core value codec.
        fields: Fields,
    },
}

/// One connection to the compositor.
#[derive(Debug)]
pub struct Client<T: Transport> {
    transport: T,
    version: Version,
    capabilities: Capabilities,
    grants: Vec<Grant>,
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
                // The grants frame follows every accept, before any other frame.
                let grants = transport.receive_grants()?;
                Ok(Client {
                    transport,
                    version: answer.version,
                    // What the compositor granted travels on the accept frame; a
                    // refusal carried none, so an ordinary connection holds nothing.
                    capabilities: answer.capabilities,
                    grants,
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

    /// The extension namespaces and versions the compositor granted this connection.
    ///
    /// The grants frame arrives right after the accept, so this is known as soon as
    /// the client connects. It is empty when nothing is granted.
    pub fn granted_extensions(&self) -> &[Grant] {
        &self.grants
    }

    /// The IPC version the compositor serves, which is also the version this client
    /// negotiated.
    pub fn version(&self) -> Version {
        self.version
    }

    /// The capabilities this connection was granted.
    ///
    /// It is the compositor's decision, read from the handshake, and it is empty for
    /// every connection that reached the ordinary socket. A client uses it to report
    /// what it can do rather than to check itself: the compositor refuses a member a
    /// connection does not hold the capability for, so a client that guessed wrong
    /// learns it from that refusal.
    pub fn capabilities(&self) -> Capabilities {
        self.capabilities
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

    /// Call a method a loaded extension declares, returning the fields it replied with.
    ///
    /// `member` is `Object.Method` and `namespace` the extension's namespace. The
    /// compositor routes the request to the provider and relays the provider's reply
    /// or refusal on this connection, so a refusal arrives as [`ClientError::Refused`]
    /// with the provider's code and message. The arguments and the reply use the core
    /// value codec, so the schema decides what each field means and this layer only
    /// moves bytes.
    pub fn call_extension(&mut self, namespace: &str, member: &str, arguments: &Fields) -> Result<Fields, ClientError> {
        let request = self.next_id();
        let payload = wire::encode_fields(arguments);
        let frame = Envelope::request(namespace, member, request, payload);
        self.send(&frame)?;
        let payload = self.await_reply(request)?;
        wire::decode_fields(&payload).map_err(|reason| ClientError::Protocol {
            member: member.to_owned(),
            reason,
        })
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

    /// Offer a request to the slot that owns it, and learn its identity.
    ///
    /// Privileged: the compositor answers [`crate::code::DENIED`] when this connection
    /// does not hold `offer-request`. A slot nobody provides is refused with
    /// [`crate::code::NO_PROVIDER`] naming the slot, which is the difference between a
    /// prompt that cannot be drawn and a prompt that never gets answered.
    ///
    /// `timeout` is how long the offer stands before the compositor fails it as timed
    /// out; the deadline the provider receives is the compositor's own tick.
    pub fn offer(
        &mut self,
        slot: &str,
        title: &str,
        message: &str,
        timeout: std::time::Duration,
    ) -> Result<RequestId, ClientError> {
        let timeout = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
        let correlation = self.next_id();
        let frame = wire::request_offer(correlation, slot, title, message, timeout);
        self.send(&frame)?;
        let payload = self.await_reply(correlation)?;
        let request = wire::read_revision(&payload).map_err(|reason| ClientError::Protocol {
            member: wire::member::OFFER.to_owned(),
            reason,
        })?;
        Ok(RequestId::new(request))
    }

    /// Claim a slot, so offers for it are sent to this connection.
    ///
    /// A shell claims the slots it draws; the services role claims the ones it
    /// provides by default. A second shell is refused with the holder named, and the
    /// services role cannot take a slot back while a shell holds it.
    pub fn claim_slot(&mut self, slot: &str, provider: Provider) -> Result<(), ClientError> {
        let correlation = self.next_id();
        let frame = wire::slot_claim(correlation, slot, provider);
        self.send(&frame)?;
        let _ = self.await_reply(correlation)?;
        Ok(())
    }

    /// Release a slot this connection claimed.
    pub fn release_slot(&mut self, slot: &str) -> Result<(), ClientError> {
        let correlation = self.next_id();
        let frame = wire::slot_release(correlation, slot);
        self.send(&frame)?;
        let _ = self.await_reply(correlation)?;
        Ok(())
    }

    /// Commit a configuration document, returning the revision the compositor
    /// assigned.
    ///
    /// Privileged: the compositor answers [`crate::code::DENIED`] when this connection
    /// does not hold `commit-config`. The document is opaque to the compositor, which
    /// holds the last accepted one for as long as it runs: that is what makes a
    /// runtime restart cost nothing but the reload.
    pub fn commit_config(&mut self, document: &[u8]) -> Result<u64, ClientError> {
        let correlation = self.next_id();
        let frame = wire::config_commit(correlation, document);
        self.send(&frame)?;
        let payload = self.await_reply(correlation)?;
        wire::read_revision(&payload).map_err(|reason| ClientError::Protocol {
            member: wire::member::CONFIG_COMMIT.to_owned(),
            reason,
        })
    }

    /// The last configuration the compositor accepted, or `None` when it has accepted
    /// none.
    ///
    /// Privileged like [`Client::commit_config`], because the document is the user's
    /// configuration rather than public session state.
    pub fn configuration(&mut self) -> Result<Option<(u64, Vec<u8>)>, ClientError> {
        let correlation = self.next_id();
        let frame = wire::config_current(correlation);
        self.send(&frame)?;
        let payload = self.await_reply(correlation)?;
        let (revision, document) = wire::read_configuration(&payload).map_err(|reason| ClientError::Protocol {
            member: wire::member::CONFIG_CURRENT.to_owned(),
            reason,
        })?;
        Ok(if revision == 0 {
            None
        } else {
            Some((revision, document))
        })
    }

    /// Ask the transport for a read deadline, where it has one.
    ///
    /// A role pumps on a clock: it wants a read that returns when nothing has arrived yet
    /// ([`ClientError::Idle`]) so it can check its own signals. A transport with no deadline
    /// to set says so and the caller keeps a blocking read.
    ///
    /// # Errors
    ///
    /// The transport's refusal, or [`crate::WireError::Kind`]-shaped unsupported answer.
    pub fn set_read_timeout(&mut self, timeout: Option<std::time::Duration>) -> Result<(), ClientError> {
        self.transport.set_read_timeout(timeout)
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
    ///
    /// A read that timed out is [`ClientError::Idle`] rather than a failure: a caller
    /// that set a read timeout on the socket is asking "is there a frame yet", and the
    /// answer "not yet" is what lets a role pump handlers and check its own signals in
    /// one loop.
    fn receive(&mut self) -> Result<Envelope, ClientError> {
        let mut descriptors = Vec::new();
        let outcome = self.transport.receive(&mut descriptors);
        self.descriptors.append(&mut descriptors);
        match outcome {
            Ok(Some(envelope)) => Ok(envelope),
            Ok(None) => Err(ClientError::Disconnected),
            Err(error) if idle(&error) => Err(ClientError::Idle),
            Err(error) => Err(ClientError::Codec(error)),
        }
    }

    /// Read until the reply to `request` arrives, queueing events on the way.
    fn await_reply(&mut self, request: u64) -> Result<Vec<u8>, ClientError> {
        loop {
            // A reply is owed, so an idle read means "not yet" and the wait continues.
            let envelope = match self.receive() {
                Err(ClientError::Idle) => continue,
                other => other?,
            };
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
        if envelope.namespace != wire::NAMESPACE {
            // An extension's event is queued as the event it is. Any other foreign
            // frame is not ours to read and is dropped, as it always was.
            if let Some(Kind::Event(event)) = &envelope.kind {
                let fields = wire::decode_fields(&event.payload).map_err(|reason| ClientError::Protocol {
                    member: envelope.member.clone(),
                    reason,
                })?;
                self.inbox.push_back(Event::Extension {
                    namespace: envelope.namespace.clone(),
                    member: envelope.member.clone(),
                    fields,
                });
            }
            return Ok(());
        }
        let decoded = wire::decode(envelope).map_err(|reason| ClientError::Protocol {
            member: envelope.member.clone(),
            reason,
        })?;
        match decoded {
            None => {}
            Some(wire::Incoming::Observation(observation)) => self.inbox.push_back(Event::Model(observation)),
            Some(wire::Incoming::Offered {
                request,
                deadline,
                slot,
                title,
                message,
            }) => {
                self.inbox.push_back(Event::RequestOffered {
                    request,
                    deadline,
                    slot,
                    title,
                    message,
                });
            }
            Some(wire::Incoming::Answered { request, answer }) => {
                self.inbox.push_back(Event::RequestAnswered { request, answer });
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

/// Whether a codec error is a read that had nothing to give yet.
fn idle(error: &gnoblin_proto::CodecError) -> bool {
    match error {
        gnoblin_proto::CodecError::Io(io) => {
            matches!(io.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)
        }
        _ => false,
    }
}
