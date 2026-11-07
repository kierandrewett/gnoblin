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

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::os::fd::OwnedFd;

use gnoblin_model::{
    DeclineReason, Fields, Filter, ObjectId, Observation, Operation, OperationOutcome, Provider, RequestCompletion,
    RequestId, Snapshot, Tick,
};
use gnoblin_proto::{Capabilities, Envelope, Grant, Handshake, HandshakeKind, Kind, Mismatch, Version};

use crate::error::{ClientError, ConnectError, HandshakeError, WireError};
use crate::transport::Transport;
use crate::wire;

/// A handle on one portal UI request, from [`Client::portal_ui`] until its outcome
/// is taken. It is the wire correlation id, which the compositor echoes on the
/// outcome, so it cannot be confused with another request on this connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PortalTicket(u64);

/// How the built-in portal UI settled a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortalOutcome {
    /// The user answered; the detail is the grant the compositor recorded.
    Handled(String),
    /// The request was withdrawn or dismissed before an answer; the detail says why.
    Cancelled(String),
    /// The user declined the request.
    Declined,
    /// The compositor could not settle the request; the detail is its reason.
    Error(String),
}

/// Something the compositor told this client.
#[derive(Clone, PartialEq)]
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
        /// What the request is about, as the offer carried it. Empty when none was named.
        subject: Vec<u8>,
    },
    /// The answer to a request this client offered. Only the host that offered the
    /// request receives it, and the frame it arrived in is flagged sensitive.
    RequestAnswered {
        /// The request.
        request: RequestId,
        /// The provider's answer, in the slot's own encoding. The bytes may be a
        /// passphrase, so nothing here logs them.
        answer: zeroize::Zeroizing<Vec<u8>>,
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

// The answer is a secret, so a log or a panic shows its length, never its bytes.
impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Event::Model(observation) => f.debug_tuple("Model").field(observation).finish(),
            Event::RequestOffered {
                request,
                deadline,
                slot,
                title,
                message,
                subject,
            } => f
                .debug_struct("RequestOffered")
                .field("request", request)
                .field("deadline", deadline)
                .field("slot", slot)
                .field("title", title)
                .field("message", message)
                .field("subject", subject)
                .finish(),
            Event::RequestAnswered { request, answer } => f
                .debug_struct("RequestAnswered")
                .field("request", request)
                .field("answer", &format_args!("{} bytes redacted", answer.len()))
                .finish(),
            Event::RequestFinished { request, completion } => f
                .debug_struct("RequestFinished")
                .field("request", request)
                .field("completion", completion)
                .finish(),
            Event::Extension {
                namespace,
                member,
                fields,
            } => f
                .debug_struct("Extension")
                .field("namespace", namespace)
                .field("member", member)
                .field("fields", fields)
                .finish(),
        }
    }
}

/// One connection to the compositor.
#[derive(Debug)]
pub struct Client<T: Transport> {
    transport: T,
    version: Version,
    capabilities: Capabilities,
    grants: Vec<Grant>,
    next_request: u64,
    /// Requests sent whose reply has not yet been taken. A reply for an id not in
    /// this set belongs to nobody (the request was forgotten or never existed), so
    /// it is dropped rather than held.
    pending: BTreeSet<u64>,
    /// Replies to pending requests that arrived while another request was awaited.
    /// Each waits here until its own [`Client::await_reply`] takes it.
    stashed: BTreeMap<u64, Envelope>,
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
                    pending: BTreeSet::new(),
                    stashed: BTreeMap::new(),
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
            // A reply to a portal ticket nobody is awaiting yet is held for its
            // `take_portal_outcome` call, not dropped as a stray reply.
            if self.holds_for_ticket(&envelope) {
                self.stashed.insert(envelope.request_id, envelope);
                continue;
            }
            self.deliver(&envelope)?;
            if let Some(event) = self.inbox.pop_front() {
                return Ok(event);
            }
        }
    }

    /// Ask the built-in portal UI for a decision without waiting for it.
    ///
    /// The compositor settles the request later, under the same ticket. Collect the
    /// outcome with [`Client::portal_outcome`] (blocking) or
    /// [`Client::take_portal_outcome`] (only if it already arrived). Privileged: the
    /// compositor refuses unless this connection holds `portal.ui`.
    pub fn portal_ui(&mut self, request: &wire::PortalRequest) -> Result<PortalTicket, ClientError> {
        let correlation = self.next_id();
        let frame = wire::portal_ui(correlation, request);
        self.send(&frame)?;
        self.pending.insert(correlation);
        Ok(PortalTicket(correlation))
    }

    /// Wait until the portal UI settles `ticket`, and say how it settled.
    pub fn portal_outcome(&mut self, ticket: PortalTicket) -> Result<PortalOutcome, ClientError> {
        let payload = self.await_reply(ticket.0)?;
        Self::outcome_from(payload)
    }

    /// The outcome of `ticket` if it has already arrived, without reading the
    /// transport. `None` means it is still pending.
    pub fn take_portal_outcome(&mut self, ticket: PortalTicket) -> Option<Result<PortalOutcome, ClientError>> {
        let envelope = self.stashed.remove(&ticket.0)?;
        self.pending.remove(&ticket.0);
        Some(Self::reply_payload(envelope).and_then(Self::outcome_from))
    }

    /// Withdraw a portal request. The compositor settles it as cancelled, and the
    /// outcome then waits for [`Client::take_portal_outcome`] like any other.
    pub fn portal_cancel(&mut self, ticket: PortalTicket) -> Result<(), ClientError> {
        let frame = wire::portal_cancel(ticket.0);
        self.send(&frame)
    }

    /// Decode a settled portal request. A refusal the compositor sent is an error
    /// outcome, because the request did reach the compositor; a transport failure
    /// stays an error of the call.
    fn outcome_from(payload: Vec<u8>) -> Result<PortalOutcome, ClientError> {
        let (kind, detail) = wire::read_portal_outcome(&payload).map_err(|reason| ClientError::Protocol {
            member: wire::member::PORTAL_UI.to_owned(),
            reason,
        })?;
        Ok(match kind.as_str() {
            "handled" => PortalOutcome::Handled(detail),
            "cancelled" => PortalOutcome::Cancelled(detail),
            "declined" => PortalOutcome::Declined,
            _ => PortalOutcome::Error(format!("unknown outcome `{kind}`")),
        })
    }

    /// Whether `envelope` answers a portal ticket this client still holds.
    fn holds_for_ticket(&self, envelope: &Envelope) -> bool {
        matches!(&envelope.kind, Some(Kind::Reply(_)) | Some(Kind::Error(_)))
            && self.pending.contains(&envelope.request_id)
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
        self.decline_with(request, None)
    }

    /// Decline a request this client claimed, saying why. `None` is the same as [`Client::decline`].
    pub fn decline_with(&mut self, request: RequestId, reason: Option<DeclineReason>) -> Result<(), ClientError> {
        let correlation = self.next_id();
        let frame = wire::decline(correlation, request, reason);
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
    ///
    /// `subject` is what the request is about, as opaque bytes. The compositor carries
    /// it to the provider without reading it, so a provider can answer by subject
    /// rather than by the sentence it shows.
    pub fn offer(
        &mut self,
        slot: &str,
        title: &str,
        message: &str,
        subject: &[u8],
        timeout: std::time::Duration,
    ) -> Result<RequestId, ClientError> {
        let timeout = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
        let correlation = self.next_id();
        let frame = wire::request_offer(correlation, slot, title, message, subject, timeout);
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

    /// Ask the runtime to re-read the configuration, returning the revision it committed.
    ///
    /// Privileged. The compositor answers when the runtime has answered: with the
    /// revision when the reload was accepted, or [`crate::code::CONFIG_REJECTED`] with
    /// the runtime's diagnostic when the document is wrong, which leaves the last
    /// accepted document in force. Requests made while one is outstanding share its
    /// answer.
    pub fn reload_config(&mut self) -> Result<u64, ClientError> {
        let correlation = self.next_id();
        let frame = wire::config_reload(correlation);
        self.send(&frame)?;
        let payload = self.await_reply(correlation)?;
        wire::read_revision(&payload).map_err(|reason| ClientError::Protocol {
            member: wire::member::CONFIG_RELOAD.to_owned(),
            reason,
        })
    }

    /// Tell the compositor the runtime refused the reload, with the diagnostic that
    /// says where the document is wrong. Privileged, and the runtime's half of
    /// [`Client::reload_config`].
    pub fn reject_config(&mut self, message: &str) -> Result<(), ClientError> {
        let correlation = self.next_id();
        let frame = wire::config_reject(correlation, message);
        self.send(&frame)?;
        let _ = self.await_reply(correlation)?;
        Ok(())
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

    /// Send one request frame, and record its id as awaiting a reply.
    fn send(&mut self, envelope: &Envelope) -> Result<(), ClientError> {
        self.transport.send(envelope, &[]).map_err(ClientError::Codec)?;
        self.pending.insert(envelope.request_id);
        Ok(())
    }

    /// Stop expecting the reply to `request`, so a late reply to it is dropped.
    ///
    /// A reply that already arrived and was stashed is discarded too. Test-only until
    /// a caller can abandon a call without awaiting it.
    #[cfg(test)]
    fn forget(&mut self, request: u64) {
        self.pending.remove(&request);
        self.stashed.remove(&request);
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

    /// Read until the reply to `request` arrives, routing other replies on the way.
    ///
    /// A reply to another pending request is stashed under its id for that request's
    /// own await. A reply whose id is not pending belongs to nobody and is dropped, as
    /// is any frame that is not a reply; events are queued as before.
    fn await_reply(&mut self, request: u64) -> Result<Vec<u8>, ClientError> {
        if let Some(envelope) = self.stashed.remove(&request) {
            self.pending.remove(&request);
            return Self::reply_payload(envelope);
        }
        loop {
            // A reply is owed, so an idle read means "not yet" and the wait continues.
            let envelope = match self.receive() {
                Err(ClientError::Idle) => continue,
                other => other?,
            };
            // Only a reply or an error can answer a request. An offer the compositor
            // sent in the other direction may happen to carry the same correlation id,
            // and it is delivered as the event it is.
            let answers = matches!(&envelope.kind, Some(Kind::Reply(_)) | Some(Kind::Error(_)));
            if answers && self.pending.contains(&envelope.request_id) {
                if envelope.request_id == request {
                    self.pending.remove(&request);
                    return Self::reply_payload(envelope);
                }
                self.stashed.insert(envelope.request_id, envelope);
                continue;
            }
            self.deliver(&envelope)?;
        }
    }

    /// The payload of a reply, or the refusal an error frame carries.
    fn reply_payload(mut envelope: Envelope) -> Result<Vec<u8>, ClientError> {
        let member = std::mem::take(&mut envelope.member);
        match envelope.kind.take() {
            Some(Kind::Reply(mut reply)) => Ok(std::mem::take(&mut reply.payload)),
            Some(Kind::Error(error)) => Err(ClientError::Refused {
                member,
                code: error.code,
                message: error.message,
            }),
            _ => Err(ClientError::Protocol {
                member,
                reason: WireError::Kind,
            }),
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
                subject,
            }) => {
                self.inbox.push_back(Event::RequestOffered {
                    request,
                    deadline,
                    slot,
                    title,
                    message,
                    subject,
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

#[cfg(test)]
mod routing_tests {
    //! Reply routing by correlation id, driven through a scripted transport so that
    //! two requests can be in flight at once, which the public API cannot do.

    use std::collections::VecDeque;
    use std::os::fd::{BorrowedFd, OwnedFd};

    use gnoblin_proto::{CodecError, Envelope, Transport, Version};

    use super::{Capabilities, Client, PortalOutcome, PortalTicket, wire};

    /// A transport that replays scripted frames and records what the client sent.
    #[derive(Debug, Default)]
    struct Script {
        inbound: VecDeque<Envelope>,
        sent: Vec<Envelope>,
    }

    impl Transport for Script {
        fn send(&mut self, envelope: &Envelope, _fds: &[BorrowedFd<'_>]) -> Result<(), CodecError> {
            self.sent.push(envelope.clone());
            Ok(())
        }

        fn receive(&mut self, _fds: &mut Vec<OwnedFd>) -> Result<Option<Envelope>, CodecError> {
            Ok(self.inbound.pop_front())
        }
    }

    // The routing tests build the client directly and never handshake.
    impl crate::transport::Transport for Script {
        fn send_handshake(&mut self, _frame: &gnoblin_proto::Handshake) -> Result<(), crate::error::HandshakeError> {
            unreachable!("the routing tests never handshake")
        }

        fn receive_handshake(&mut self) -> Result<gnoblin_proto::Handshake, crate::error::HandshakeError> {
            unreachable!("the routing tests never handshake")
        }

        fn receive_grants(&mut self) -> Result<Vec<gnoblin_proto::Grant>, crate::error::HandshakeError> {
            unreachable!("the routing tests never handshake")
        }
    }

    /// A client over a scripted transport, with no handshake state beyond the defaults.
    fn client(inbound: Vec<Envelope>) -> Client<Script> {
        let script = Script {
            inbound: inbound.into(),
            sent: Vec::new(),
        };
        Client {
            transport: script,
            version: Version::new(0, 2, 0),
            capabilities: Capabilities::NONE,
            grants: Vec::new(),
            next_request: 1,
            pending: Default::default(),
            stashed: Default::default(),
            inbox: Default::default(),
            descriptors: Vec::new(),
        }
    }

    /// A reply frame answering `id`, in the wire namespace.
    fn reply(id: u64) -> Envelope {
        Envelope::reply(wire::NAMESPACE, "test.member", id, vec![id as u8])
    }

    /// Send a request frame the way the client does, returning its id.
    fn send_request(client: &mut Client<Script>) -> u64 {
        let id = client.next_id();
        let frame = Envelope::request(wire::NAMESPACE, "test.member", id, Vec::new());
        client.send(&frame).expect("the scripted transport accepts every frame");
        id
    }

    #[test]
    fn a_reply_for_an_unknown_id_is_dropped() {
        let mut client = client(vec![reply(9), reply(1)]);
        let first = send_request(&mut client);
        let payload = client.await_reply(first).expect("the reply to request 1 arrives");
        assert_eq!(payload, vec![1]);
        assert!(client.stashed.is_empty(), "a reply nobody awaits is not held");
    }

    #[test]
    fn two_requests_in_flight_each_take_their_own_reply() {
        // The compositor answers the second request first.
        let mut client = client(vec![reply(2), reply(1)]);
        let first = send_request(&mut client);
        let second = send_request(&mut client);

        let first_payload = client.await_reply(first).expect("request 1 is answered");
        assert_eq!(first_payload, vec![1]);
        assert!(
            client.stashed.contains_key(&second),
            "the early reply to request 2 is held"
        );

        // Request 2 takes its stashed reply without reading the transport again.
        let second_payload = client.await_reply(second).expect("request 2 is answered");
        assert_eq!(second_payload, vec![2]);
        assert!(client.stashed.is_empty());
        assert!(client.pending.is_empty());
    }

    #[test]
    fn a_reply_after_its_request_was_forgotten_is_dropped() {
        // Request 1 is abandoned; its late reply must not disturb request 2.
        let mut client = client(vec![reply(1), reply(2)]);
        let abandoned = send_request(&mut client);
        let live = send_request(&mut client);
        client.forget(abandoned);

        let payload = client.await_reply(live).expect("request 2 is answered");
        assert_eq!(payload, vec![2]);
        assert!(client.stashed.is_empty(), "the late reply to request 1 is not held");
        assert!(client.pending.is_empty());
    }

    #[test]
    fn a_stashed_reply_for_a_forgotten_request_is_discarded() {
        // Request 2's reply arrives while request 1 is awaited, then request 2 is abandoned.
        let mut client = client(vec![reply(2), reply(1)]);
        let first = send_request(&mut client);
        let second = send_request(&mut client);
        client.await_reply(first).expect("request 1 is answered");
        assert!(client.stashed.contains_key(&second));
        client.forget(second);
        assert!(client.stashed.is_empty());
    }

    /// A `portal.ui` request, the shape the relay sends.
    fn portal_request() -> wire::PortalRequest {
        wire::PortalRequest {
            interface: "org.freedesktop.impl.portal.Access".to_owned(),
            caller: wire::VerifiedCaller::new(0, "org.example.App"),
            prompt: "Allow the camera".to_owned(),
            timeout_ms: 0,
            options: Default::default(),
            parent_window: String::new(),
        }
    }

    /// A `portal.ui` reply to `id`, carrying how the request settled.
    fn outcome(id: u64, kind: &str, detail: &str) -> Envelope {
        Envelope::reply(
            wire::NAMESPACE,
            wire::member::PORTAL_UI,
            id,
            wire::portal_outcome_payload(kind, detail),
        )
    }

    #[test]
    fn a_reply_to_an_unknown_ticket_is_not_taken_as_an_outcome() {
        // Ticket 9 was never sent; its reply must not surface under any ticket.
        let mut client = client(vec![outcome(9, "handled", "allowed")]);
        let ticket = client.portal_ui(&portal_request()).expect("the ticket is sent");
        assert!(client.take_portal_outcome(PortalTicket(9)).is_none());
        assert!(
            client.take_portal_outcome(ticket).is_none(),
            "the ticket is still pending"
        );
    }

    #[test]
    fn a_cancelled_ticket_collects_the_cancelled_outcome() {
        let mut client = client(vec![outcome(1, "cancelled", "withdrawn")]);
        let ticket = client.portal_ui(&portal_request()).expect("the ticket is sent");
        client.portal_cancel(ticket).expect("the withdrawal is sent");
        let settled = client.portal_outcome(ticket).expect("the outcome arrives");
        assert_eq!(settled, PortalOutcome::Cancelled("withdrawn".to_owned()));
        assert!(client.pending.is_empty());
    }

    #[test]
    fn two_tickets_in_flight_each_collect_their_own_outcome() {
        // The compositor settles the second ticket first.
        let mut client = client(vec![outcome(2, "declined", ""), outcome(1, "handled", "allowed")]);
        let first = client.portal_ui(&portal_request()).expect("ticket 1 is sent");
        let second = client.portal_ui(&portal_request()).expect("ticket 2 is sent");
        assert_eq!(
            client.portal_outcome(first).expect("ticket 1 settles"),
            PortalOutcome::Handled("allowed".to_owned())
        );
        assert_eq!(
            client.portal_outcome(second).expect("ticket 2 settles"),
            PortalOutcome::Declined
        );
        assert!(client.stashed.is_empty() && client.pending.is_empty());
    }

    #[test]
    fn an_outcome_that_arrives_before_the_take_is_held_for_it() {
        // The outcome arrives while the client reads events; nobody awaits it yet.
        let mut client = client(vec![outcome(1, "handled", "allowed")]);
        let ticket = client.portal_ui(&portal_request()).expect("the ticket is sent");
        // The script then runs dry, which ends the read; the held outcome survives it.
        assert!(client.next_event().is_err());
        let taken = client.take_portal_outcome(ticket).expect("the outcome is held");
        assert_eq!(taken.expect("it decodes"), PortalOutcome::Handled("allowed".to_owned()));
        assert!(client.take_portal_outcome(ticket).is_none(), "an outcome is taken once");
    }
}
