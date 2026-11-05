//! The object model on the wire.
//!
//! The schema declares objects, properties, methods, signals and error codes; the
//! envelope (`proto/envelope.proto`) carries a namespace, a member and a payload.
//! Between those two sits the part of the IPC that is not an object member:
//! subscribing, the observation stream and the request layer. It is hand-written
//! here for the same reason the envelope is: an envelope is the layer that carries
//! schema members, so the members that *are* the layer cannot be schema members
//! themselves.
//!
//! Payload layout is fixed and big-endian, with `u64` lengths so a payload can
//! never carry a length a `u32` would have to truncate:
//!
//! | member                        | direction | payload                                          |
//! | ----------------------------- | --------- | ------------------------------------------------ |
//! | `object.subscribe`            | client    | filter                                            |
//! | reply to `object.subscribe`   | compositor| snapshot                                          |
//! | `object.call`                 | client    | object, method, arguments                         |
//! | reply to `object.call`        | compositor| operation                                         |
//! | `request.claim`               | client    | request                                           |
//! | `request.answer`              | client    | request, answer                                   |
//! | `request.decline`             | client    | request                                           |
//! | `object.opened`               | compositor| revision, object view                             |
//! | `object.changed`              | compositor| revision, object, property, value                 |
//! | `object.signalled`            | compositor| revision, object, signal, fields                  |
//! | `object.closed`               | compositor| revision, object                                  |
//! | `operation.finished`          | compositor| revision, operation, object, outcome              |
//! | `request.offer`               | privileged| slot, title, message, timeout                      |
//! | `request.offered`             | compositor| request, deadline, slot, title, message            |
//! | `request.answered`            | compositor| request, answer (flagged sensitive)                |
//! | `request.finished`            | compositor| request, completion                               |
//! | `slot.claim`                  | client    | slot, provider                                    |
//! | `slot.release`                | client    | slot                                              |
//! | `config.commit`               | privileged| document bytes                                    |
//! | `config.current`              | privileged| nothing; the last accepted revision and document  |
//!
//! Every observation payload starts with its revision, so the client can enforce
//! the same ordering rule the graph does and never has to infer it.
//!
//! A member this build does not know decodes to `None` rather than an error:
//! additions are compatible by construction (the caret version rule), so a newer
//! compositor may send members this client does not use, and dropping them is what
//! keeps an older client working.

use gnoblin_model::{
    ErrorCode, Fields, Filter, ObjectId, ObjectView, Observation, OperationId, OperationOutcome, Provider,
    RequestCompletion, RequestId, Revision, Snapshot, Tick, TypeKey, Value,
};
use gnoblin_proto::Envelope;

use crate::error::WireError;

/// The namespace of every member on this layer.
pub const NAMESPACE: &str = "gnoblin";

/// The members this layer defines.
pub mod member {
    /// Client to compositor: subscribe and receive a snapshot.
    pub const SUBSCRIBE: &str = "object.subscribe";
    /// Client to compositor: call a declared method.
    pub const CALL: &str = "object.call";
    /// Client to compositor: claim an offered request.
    pub const CLAIM: &str = "request.claim";
    /// Client to compositor: answer a request it holds. The schema marks its
    /// `answer` field secret, so every frame for it is flagged sensitive.
    pub const ANSWER: &str = "request.answer";
    /// Client to compositor: decline a request it holds. The schema marks no field
    /// secret, because a declined request carries no value.
    pub const DECLINE: &str = "request.decline";
    /// Compositor to client: an object came into existence.
    pub const OPENED: &str = "object.opened";
    /// Compositor to client: one property changed.
    pub const CHANGED: &str = "object.changed";
    /// Compositor to client: a declared signal was raised.
    pub const SIGNALLED: &str = "object.signalled";
    /// Compositor to client: an object ended.
    pub const CLOSED: &str = "object.closed";
    /// Compositor to client: a method call finished.
    pub const OPERATION_FINISHED: &str = "operation.finished";
    /// Compositor to client: a request is available to claim.
    pub const REQUEST_OFFERED: &str = "request.offered";
    /// Compositor to client: a request ended.
    pub const REQUEST_FINISHED: &str = "request.finished";
    /// Privileged client to compositor: offer a request to the slot that owns it.
    pub const OFFER: &str = "request.offer";
    /// Client to compositor: claim a slot, so offers for it come to this
    /// connection.
    pub const SLOT_CLAIM: &str = "slot.claim";
    /// Client to compositor: release a slot this connection claimed.
    pub const SLOT_RELEASE: &str = "slot.release";
    /// Privileged client to compositor: commit a configuration document.
    pub const CONFIG_COMMIT: &str = "config.commit";
    /// Privileged client to compositor: the last document the compositor accepted.
    pub const CONFIG_CURRENT: &str = "config.current";
}

/// The error codes this layer reports in an error frame.
pub mod code {
    /// Another owner holds the request.
    pub const REQUEST_TAKEN: &str = "request-taken";
    /// The request already finished.
    pub const REQUEST_FINISHED: &str = "request-finished";
    /// The compositor never offered the request, or it is forgotten.
    pub const REQUEST_UNKNOWN: &str = "request-unknown";
    /// No live object has that identity.
    pub const OBJECT_UNKNOWN: &str = "object-unknown";
    /// The member is not one this layer serves.
    pub const UNKNOWN_MEMBER: &str = gnoblin_proto::UNKNOWN_MEMBER;
    /// This connection does not hold the capability the member needs.
    ///
    /// A member a privileged connection serves is answered with this rather than
    /// `unknown-member` when an ordinary connection calls it: the member exists, and
    /// the caller is what is missing.
    pub const DENIED: &str = "denied";
    /// The slot name is not one this build has.
    pub const UNKNOWN_SLOT: &str = "unknown-slot";
    /// No provider claims the slot, so the request was not offered.
    ///
    /// Fail closed: a request nobody will answer ends here instead of waiting out its
    /// deadline on the host that needed the decision.
    pub const NO_PROVIDER: &str = "no-provider";
}

/// One decoded frame from the compositor.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    /// A change to the object graph.
    Observation(Observation),
    /// A request this client may claim, with what the host that offered it says the
    /// provider has to show.
    Offered {
        /// The request.
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
    /// The answer to a request this connection offered. Only the host that offered
    /// the request ever receives this, and the frame is flagged sensitive because it
    /// carries what the user typed.
    Answered {
        /// The request.
        request: RequestId,
        /// The provider's answer, in the slot's own encoding.
        answer: Vec<u8>,
    },
    /// A request ended. The outcome carries no answer value: only the host that
    /// offered the request reads that.
    Finished {
        /// The request that ended.
        request: RequestId,
        /// How it ended.
        completion: RequestCompletion,
    },
}

/// A request asking for a subscription's snapshot first.
pub fn subscribe(request_id: u64, filter: Filter) -> Envelope {
    let mut writer = Writer::new();
    match filter {
        Filter::All => writer.u8(0),
        Filter::Object(object) => {
            writer.u8(1);
            writer.u64(object.get());
        }
    }
    Envelope::request(NAMESPACE, member::SUBSCRIBE, request_id, writer.finish())
}

/// A request calling a declared method.
pub fn call(request_id: u64, object: ObjectId, method: &str, arguments: &Fields) -> Envelope {
    let mut writer = Writer::new();
    writer.u64(object.get());
    writer.text(method);
    writer.fields(arguments);
    Envelope::request(NAMESPACE, member::CALL, request_id, writer.finish())
}

/// A request claiming an offered request.
pub fn claim(request_id: u64, request: RequestId) -> Envelope {
    let mut writer = Writer::new();
    writer.u64(request.get());
    Envelope::request(NAMESPACE, member::CLAIM, request_id, writer.finish())
}

/// A request answering a request the client holds.
///
/// An empty `answer` is a real answer — consent with no content — and is encoded as
/// a zero-length value, not as an ended request: ending has its own member, so the
/// two outcomes can never be confused by their bytes.
pub fn answer(request_id: u64, request: RequestId, answer: &[u8]) -> Envelope {
    let mut writer = Writer::new();
    writer.u64(request.get());
    writer.bytes(answer);
    Envelope::request(NAMESPACE, member::ANSWER, request_id, writer.finish())
}

/// A request declining a request the client holds: the provider will not answer it.
///
/// This is the refusal and provider-gone outcome, and it carries no value at all,
/// which is why its frame is never flagged sensitive.
pub fn decline(request_id: u64, request: RequestId) -> Envelope {
    let mut writer = Writer::new();
    writer.u64(request.get());
    Envelope::request(NAMESPACE, member::DECLINE, request_id, writer.finish())
}

/// An offer, sent as a request frame so it carries a correlation id and looks to
/// the client like the request it is. The client does not answer it: it claims
/// through [`claim`] and answers through [`answer`].
///
/// The proposition travels with the offer because the schema's `request` object
/// carries only the identity and the deadline, and a provider that had to ask what a
/// request is about would need a second round trip before it could draw anything.
pub fn offer(request_id: u64, request: RequestId, deadline: Tick, slot: &str, title: &str, message: &str) -> Envelope {
    let mut writer = Writer::new();
    writer.u64(request.get());
    writer.u64(deadline.get());
    writer.text(slot);
    writer.text(title);
    writer.text(message);
    Envelope::request(NAMESPACE, member::REQUEST_OFFERED, request_id, writer.finish())
}

/// The answer to a request, sent to the connection that offered it.
///
/// It travels as an *event* on the schema's `request.answer` member, so the generated
/// sensitivity table flags it and [`Envelope::redact`] drops the bytes: an answer is
/// the one frame on this layer that carries a password, and no host has to remember
/// that to log it safely.
pub fn answered(request: RequestId, answer: &[u8]) -> Envelope {
    let mut writer = Writer::new();
    writer.u64(request.get());
    writer.bytes(answer);
    Envelope::event(NAMESPACE, member::ANSWER, writer.finish())
}

/// A request offering a request to the slot that owns it. Privileged.
pub fn request_offer(request_id: u64, slot: &str, title: &str, message: &str, timeout_ms: u64) -> Envelope {
    let mut writer = Writer::new();
    writer.text(slot);
    writer.text(title);
    writer.text(message);
    writer.u64(timeout_ms);
    Envelope::request(NAMESPACE, member::OFFER, request_id, writer.finish())
}

/// A request claiming a slot.
pub fn slot_claim(request_id: u64, slot: &str, provider: Provider) -> Envelope {
    let mut writer = Writer::new();
    writer.text(slot);
    writer.text(provider.as_str());
    Envelope::request(NAMESPACE, member::SLOT_CLAIM, request_id, writer.finish())
}

/// A request releasing a slot this connection claimed.
pub fn slot_release(request_id: u64, slot: &str) -> Envelope {
    let mut writer = Writer::new();
    writer.text(slot);
    Envelope::request(NAMESPACE, member::SLOT_RELEASE, request_id, writer.finish())
}

/// A request committing a configuration document. Privileged.
pub fn config_commit(request_id: u64, document: &[u8]) -> Envelope {
    let mut writer = Writer::new();
    writer.bytes(document);
    Envelope::request(NAMESPACE, member::CONFIG_COMMIT, request_id, writer.finish())
}

/// A request for the last document the compositor accepted. Privileged.
pub fn config_current(request_id: u64) -> Envelope {
    Envelope::request(NAMESPACE, member::CONFIG_CURRENT, request_id, Vec::new())
}

/// The end of a request, as a subscriber may learn it.
pub fn finished(request: RequestId, completion: RequestCompletion) -> Envelope {
    let mut writer = Writer::new();
    writer.u64(request.get());
    writer.u8(match completion {
        RequestCompletion::Answered => 0,
        RequestCompletion::Ended => 1,
        RequestCompletion::TimedOut => 2,
    });
    Envelope::event(NAMESPACE, member::REQUEST_FINISHED, writer.finish())
}

/// Encode one observation.
pub fn observation(observation: &Observation) -> Envelope {
    let mut writer = Writer::new();
    writer.u64(observation.revision().get());
    let member = match observation {
        Observation::Opened { view, .. } => {
            writer.view(view);
            member::OPENED
        }
        Observation::Changed {
            object,
            property,
            value,
            ..
        } => {
            writer.u64(object.get());
            writer.text(property);
            writer.value(value);
            member::CHANGED
        }
        Observation::Signalled {
            object, signal, fields, ..
        } => {
            writer.u64(object.get());
            writer.text(signal);
            writer.fields(fields);
            member::SIGNALLED
        }
        Observation::Closed { object, .. } => {
            writer.u64(object.get());
            member::CLOSED
        }
        Observation::Finished {
            operation,
            object,
            outcome,
            ..
        } => {
            writer.u64(operation.get());
            writer.u64(object.get());
            match outcome {
                OperationOutcome::Done => writer.u8(0),
                OperationOutcome::Failed(code) => {
                    writer.u8(1);
                    writer.text(code.as_str());
                }
            }
            member::OPERATION_FINISHED
        }
    };
    Envelope::event(NAMESPACE, member, writer.finish())
}

/// Encode a snapshot, the payload of a `object.subscribe` reply.
pub fn snapshot_payload(snapshot: &Snapshot) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.u64(snapshot.revision().get());
    writer.u64(snapshot.objects().len() as u64);
    for view in snapshot.objects() {
        writer.view(view);
    }
    writer.finish()
}

/// Decode a snapshot.
pub fn read_snapshot(payload: &[u8]) -> Result<Snapshot, WireError> {
    let mut reader = Reader::new(payload);
    let revision = Revision::new(reader.u64()?);
    let count = reader.length()?;
    let mut objects = Vec::new();
    for _ in 0..count {
        objects.push(reader.view()?);
    }
    reader.done()?;
    Ok(Snapshot::new(revision, objects))
}

/// Decode the operation id a `object.call` reply names.
pub fn read_operation(payload: &[u8]) -> Result<OperationId, WireError> {
    let mut reader = Reader::new(payload);
    let operation = OperationId::new(reader.u64()?);
    reader.done()?;
    Ok(operation)
}

/// Decode a `object.subscribe` request.
pub fn read_subscribe(payload: &[u8]) -> Result<Filter, WireError> {
    let mut reader = Reader::new(payload);
    let filter = match reader.u8()? {
        0 => Filter::All,
        1 => Filter::Object(ObjectId::new(reader.u64()?)),
        tag => return Err(WireError::Tag(tag)),
    };
    reader.done()?;
    Ok(filter)
}

/// Decode a `object.call` request.
pub fn read_call(payload: &[u8]) -> Result<(ObjectId, String, Fields), WireError> {
    let mut reader = Reader::new(payload);
    let object = ObjectId::new(reader.u64()?);
    let method = reader.text()?;
    let arguments = reader.fields()?;
    reader.done()?;
    Ok((object, method, arguments))
}

/// Decode a `request.claim` request.
pub fn read_claim(payload: &[u8]) -> Result<RequestId, WireError> {
    let mut reader = Reader::new(payload);
    let request = RequestId::new(reader.u64()?);
    reader.done()?;
    Ok(request)
}

/// Decode a `request.answer` request.
pub fn read_answer(payload: &[u8]) -> Result<(RequestId, Vec<u8>), WireError> {
    let mut reader = Reader::new(payload);
    let request = RequestId::new(reader.u64()?);
    let answer = reader.bytes()?;
    reader.done()?;
    Ok((request, answer))
}

/// Decode a `request.decline` request.
pub fn read_decline(payload: &[u8]) -> Result<RequestId, WireError> {
    let mut reader = Reader::new(payload);
    let request = RequestId::new(reader.u64()?);
    reader.done()?;
    Ok(request)
}

/// Decode a `request.offer` request. Privileged.
pub fn read_offer(payload: &[u8]) -> Result<(String, String, String, u64), WireError> {
    let mut reader = Reader::new(payload);
    let slot = reader.text()?;
    let title = reader.text()?;
    let message = reader.text()?;
    let timeout = reader.u64()?;
    reader.done()?;
    Ok((slot, title, message, timeout))
}

/// Decode a `slot.claim` request.
pub fn read_slot_claim(payload: &[u8]) -> Result<(String, Provider), WireError> {
    let mut reader = Reader::new(payload);
    let slot = reader.text()?;
    let provider = Provider::parse(&reader.text()?).ok_or(WireError::Code)?;
    reader.done()?;
    Ok((slot, provider))
}

/// Decode a `slot.release` request.
pub fn read_slot_release(payload: &[u8]) -> Result<String, WireError> {
    let mut reader = Reader::new(payload);
    let slot = reader.text()?;
    reader.done()?;
    Ok(slot)
}

/// Decode a `config.commit` request: the document, as the runtime encoded it.
pub fn read_config_commit(payload: &[u8]) -> Result<Vec<u8>, WireError> {
    let mut reader = Reader::new(payload);
    let document = reader.bytes()?;
    reader.done()?;
    Ok(document)
}

/// Decode the revision a `config.commit` reply names.
pub fn read_revision(payload: &[u8]) -> Result<u64, WireError> {
    let mut reader = Reader::new(payload);
    let revision = reader.u64()?;
    reader.done()?;
    Ok(revision)
}

/// Decode the `config.current` reply: the revision and the document, or zeros for
/// "nothing accepted yet".
pub fn read_configuration(payload: &[u8]) -> Result<(u64, Vec<u8>), WireError> {
    let mut reader = Reader::new(payload);
    let revision = reader.u64()?;
    let document = reader.bytes()?;
    reader.done()?;
    Ok((revision, document))
}

/// Encode the `config.current` reply.
pub fn configuration_payload(revision: u64, document: &[u8]) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.u64(revision);
    writer.bytes(document);
    writer.finish()
}

/// Decode one frame from the compositor.
///
/// `Ok(None)` is a member this build does not use, which is dropped rather than
/// reported: the version rule makes additions compatible, so a newer compositor may
/// legitimately send members an older client has never heard of.
pub fn decode(envelope: &Envelope) -> Result<Option<Incoming>, WireError> {
    if envelope.namespace != NAMESPACE {
        return Ok(None);
    }
    let payload = envelope.payload().unwrap_or(&[]);
    let incoming = match envelope.member.as_str() {
        member::OPENED => {
            let mut reader = Reader::new(payload);
            let revision = Revision::new(reader.u64()?);
            let view = reader.view()?;
            reader.done()?;
            Incoming::Observation(Observation::Opened { view, revision })
        }
        member::CHANGED => {
            let mut reader = Reader::new(payload);
            let revision = Revision::new(reader.u64()?);
            let object = ObjectId::new(reader.u64()?);
            let property = reader.text()?;
            let value = reader.value()?;
            reader.done()?;
            Incoming::Observation(Observation::Changed {
                object,
                property,
                value,
                revision,
            })
        }
        member::SIGNALLED => {
            let mut reader = Reader::new(payload);
            let revision = Revision::new(reader.u64()?);
            let object = ObjectId::new(reader.u64()?);
            let signal = reader.text()?;
            let fields = reader.fields()?;
            reader.done()?;
            Incoming::Observation(Observation::Signalled {
                object,
                signal,
                fields,
                revision,
            })
        }
        member::CLOSED => {
            let mut reader = Reader::new(payload);
            let revision = Revision::new(reader.u64()?);
            let object = ObjectId::new(reader.u64()?);
            reader.done()?;
            Incoming::Observation(Observation::Closed { object, revision })
        }
        member::OPERATION_FINISHED => {
            let mut reader = Reader::new(payload);
            let revision = Revision::new(reader.u64()?);
            let operation = OperationId::new(reader.u64()?);
            let object = ObjectId::new(reader.u64()?);
            let outcome = match reader.u8()? {
                0 => OperationOutcome::Done,
                1 => {
                    let code = ErrorCode::parse(&reader.text()?).ok_or(WireError::Code)?;
                    OperationOutcome::Failed(code)
                }
                tag => return Err(WireError::Tag(tag)),
            };
            reader.done()?;
            Incoming::Observation(Observation::Finished {
                operation,
                object,
                outcome,
                revision,
            })
        }
        member::REQUEST_OFFERED => {
            let mut reader = Reader::new(payload);
            let request = RequestId::new(reader.u64()?);
            let deadline = Tick::new(reader.u64()?);
            let slot = reader.text()?;
            let title = reader.text()?;
            let message = reader.text()?;
            reader.done()?;
            Incoming::Offered {
                request,
                deadline,
                slot,
                title,
                message,
            }
        }
        member::ANSWER => {
            let mut reader = Reader::new(payload);
            let request = RequestId::new(reader.u64()?);
            let answer = reader.bytes()?;
            reader.done()?;
            Incoming::Answered { request, answer }
        }
        member::REQUEST_FINISHED => {
            let mut reader = Reader::new(payload);
            let request = RequestId::new(reader.u64()?);
            let completion = match reader.u8()? {
                0 => RequestCompletion::Answered,
                1 => RequestCompletion::Ended,
                2 => RequestCompletion::TimedOut,
                tag => return Err(WireError::Tag(tag)),
            };
            reader.done()?;
            Incoming::Finished { request, completion }
        }
        _ => return Ok(None),
    };
    Ok(Some(incoming))
}

/// The payload writer.
///
/// Lengths are `u64` so encoding can never truncate, and the writer needs no
/// fallible path: everything here is a value the model already validated.
struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn new() -> Writer {
        Writer { bytes: Vec::new() }
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.u64(bytes.len() as u64);
        self.bytes.extend_from_slice(bytes);
    }

    fn text(&mut self, text: &str) {
        self.bytes(text.as_bytes());
    }

    fn fields(&mut self, fields: &Fields) {
        self.u64(fields.len() as u64);
        for (name, value) in fields {
            self.text(name);
            self.value(value);
        }
    }

    fn view(&mut self, view: &ObjectView) {
        self.u64(view.id().get());
        self.text(view.ty().namespace());
        self.text(view.ty().name());
        self.fields(view.properties());
    }

    fn value(&mut self, value: &Value) {
        match value {
            Value::Bool(inner) => {
                self.u8(0);
                self.u8(u8::from(*inner));
            }
            Value::Int(inner) => {
                self.u8(1);
                self.bytes.extend_from_slice(&inner.to_be_bytes());
            }
            Value::Uint(inner) => {
                self.u8(2);
                self.u64(*inner);
            }
            Value::Double(inner) => {
                self.u8(3);
                self.u64(inner.to_bits());
            }
            Value::Text(inner) => {
                self.u8(4);
                self.text(inner);
            }
            Value::Bytes(inner) => {
                self.u8(5);
                self.bytes(inner);
            }
            Value::Object(inner) => {
                self.u8(6);
                self.u64(inner.get());
            }
            Value::List(inner) => {
                self.u8(7);
                self.u64(inner.len() as u64);
                for value in inner {
                    self.value(value);
                }
            }
        }
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

/// The payload reader.
///
/// Every read is bounds-checked, a count never preallocates, and a decode that
/// leaves bytes behind is a refusal: a half-understood payload would silently break
/// the model's delivery rule.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes, at: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], WireError> {
        let end = self.at.checked_add(count).ok_or(WireError::Truncated)?;
        let slice = self.bytes.get(self.at..end).ok_or(WireError::Truncated)?;
        self.at = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, WireError> {
        let byte = *self.take(1)?.first().ok_or(WireError::Truncated)?;
        Ok(byte)
    }

    fn u64(&mut self) -> Result<u64, WireError> {
        let slice = self.take(8)?;
        let array: [u8; 8] = slice.try_into().map_err(|_| WireError::Truncated)?;
        Ok(u64::from_be_bytes(array))
    }

    fn length(&mut self) -> Result<usize, WireError> {
        usize::try_from(self.u64()?).map_err(|_| WireError::Truncated)
    }

    fn bytes(&mut self) -> Result<Vec<u8>, WireError> {
        let length = self.length()?;
        Ok(self.take(length)?.to_vec())
    }

    fn text(&mut self) -> Result<String, WireError> {
        let length = self.length()?;
        let slice = self.take(length)?;
        String::from_utf8(slice.to_vec()).map_err(|_| WireError::Utf8)
    }

    fn fields(&mut self) -> Result<Fields, WireError> {
        let count = self.length()?;
        let mut fields = Fields::new();
        for _ in 0..count {
            let name = self.text()?;
            let value = self.value()?;
            if fields.insert(name, value).is_some() {
                return Err(WireError::Duplicate);
            }
        }
        Ok(fields)
    }

    fn view(&mut self) -> Result<ObjectView, WireError> {
        let id = ObjectId::new(self.u64()?);
        let namespace = self.text()?;
        let name = self.text()?;
        let properties = self.fields()?;
        Ok(ObjectView::new(id, TypeKey::new(namespace, name), properties))
    }

    fn value(&mut self) -> Result<Value, WireError> {
        let value = match self.u8()? {
            0 => match self.u8()? {
                0 => Value::Bool(false),
                1 => Value::Bool(true),
                tag => return Err(WireError::Tag(tag)),
            },
            1 => {
                let slice = self.take(8)?;
                let array: [u8; 8] = slice.try_into().map_err(|_| WireError::Truncated)?;
                Value::Int(i64::from_be_bytes(array))
            }
            2 => Value::Uint(self.u64()?),
            3 => Value::Double(f64::from_bits(self.u64()?)),
            4 => Value::Text(self.text()?),
            5 => Value::Bytes(self.bytes()?),
            6 => Value::Object(ObjectId::new(self.u64()?)),
            7 => {
                let count = self.length()?;
                let mut values = Vec::new();
                for _ in 0..count {
                    values.push(self.value()?);
                }
                Value::List(values)
            }
            tag => return Err(WireError::Tag(tag)),
        };
        Ok(value)
    }

    fn done(self) -> Result<(), WireError> {
        if self.at == self.bytes.len() {
            Ok(())
        } else {
            Err(WireError::Trailing)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use gnoblin_model::OBJECT_GONE;
    use gnoblin_schema::Catalog;

    use super::*;

    fn fields(pairs: &[(&str, Value)]) -> Fields {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect()
    }

    fn view(id: u64) -> ObjectView {
        ObjectView::new(
            ObjectId::new(id),
            TypeKey::gnoblin("window"),
            fields(&[
                ("id", Value::Uint(id)),
                ("title", Value::Text("Report".to_owned())),
                ("focused", Value::Bool(true)),
                ("scale", Value::Double(1.5)),
                ("blob", Value::Bytes(vec![0, 255, 7])),
                ("parent", Value::Object(ObjectId::new(9))),
                ("list", Value::List(vec![Value::Int(-3), Value::Text("two".to_owned())])),
            ]),
        )
    }

    #[test]
    fn every_value_variant_survives_a_round_trip() {
        let encoded = fields(&[
            ("bool", Value::Bool(false)),
            ("int", Value::Int(i64::MIN)),
            ("uint", Value::Uint(u64::MAX)),
            ("double", Value::Double(-0.25)),
            ("text", Value::Text("hello".to_owned())),
            ("bytes", Value::Bytes(vec![1, 2, 3])),
            ("object", Value::Object(ObjectId::new(4))),
            ("list", Value::List(vec![Value::Bool(true)])),
        ]);
        let mut writer = Writer::new();
        writer.fields(&encoded);
        let payload = writer.finish();
        let mut reader = Reader::new(&payload);
        assert_eq!(reader.fields().expect("decodes"), encoded);
        reader.done().expect("nothing left over");
    }

    #[test]
    fn every_observation_survives_a_round_trip() {
        let observations = [
            Observation::Opened {
                view: view(1),
                revision: Revision::new(7),
            },
            Observation::Changed {
                object: ObjectId::new(1),
                property: "title".to_owned(),
                value: Value::Text("Renamed".to_owned()),
                revision: Revision::new(8),
            },
            Observation::Signalled {
                object: ObjectId::new(1),
                signal: "closed".to_owned(),
                fields: Fields::new(),
                revision: Revision::new(9),
            },
            Observation::Closed {
                object: ObjectId::new(1),
                revision: Revision::new(10),
            },
            Observation::Finished {
                operation: OperationId::new(3),
                object: ObjectId::new(1),
                outcome: OperationOutcome::Failed(OBJECT_GONE),
                revision: Revision::new(11),
            },
            Observation::Finished {
                operation: OperationId::new(4),
                object: ObjectId::new(1),
                outcome: OperationOutcome::Done,
                revision: Revision::new(12),
            },
        ];
        for observation in observations {
            let frame = super::observation(&observation);
            assert_eq!(
                decode(&frame).expect("decodes"),
                Some(Incoming::Observation(observation))
            );
        }
    }

    #[test]
    fn a_snapshot_round_trips_with_its_revision() {
        let snapshot = Snapshot::new(Revision::new(42), vec![view(1), view(2)]);
        let decoded = read_snapshot(&snapshot_payload(&snapshot)).expect("decodes");
        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn a_truncated_payload_is_refused() {
        let snapshot = Snapshot::new(Revision::new(1), vec![view(1)]);
        let payload = snapshot_payload(&snapshot);
        for cut in 0..payload.len() {
            assert!(
                read_snapshot(&payload[..cut]).is_err(),
                "a payload cut at {cut} decoded"
            );
        }
    }

    #[test]
    fn trailing_bytes_are_refused() {
        let mut payload = snapshot_payload(&Snapshot::new(Revision::new(1), Vec::new()));
        payload.push(0);
        assert_eq!(read_snapshot(&payload), Err(WireError::Trailing));
    }

    #[test]
    fn an_unknown_value_tag_is_refused() {
        let payload = [9_u8];
        let mut reader = Reader::new(&payload);
        assert_eq!(reader.value(), Err(WireError::Tag(9)));
    }

    #[test]
    fn an_unknown_member_is_dropped_rather_than_failing() {
        let envelope = Envelope::event(NAMESPACE, "object.invented", vec![1, 2, 3]);
        assert_eq!(decode(&envelope).expect("decodes"), None);
        let other_namespace = Envelope::event("example.ext", "probe.question", Vec::new());
        assert_eq!(decode(&other_namespace).expect("decodes"), None);
    }

    #[test]
    fn a_claim_an_answer_and_an_end_round_trip() {
        let frame = claim(5, RequestId::new(2));
        assert_eq!(read_claim(frame.payload().expect("payload")), Ok(RequestId::new(2)));

        for answer in [vec![7, 8], Vec::new()] {
            let frame = self::answer(6, RequestId::new(2), &answer);
            assert_eq!(
                read_answer(frame.payload().expect("payload")),
                Ok((RequestId::new(2), answer))
            );
        }

        let frame = decline(7, RequestId::new(2));
        assert_eq!(read_decline(frame.payload().expect("payload")), Ok(RequestId::new(2)));
    }

    #[test]
    fn a_subscribe_and_a_call_round_trip() {
        for filter in [Filter::All, Filter::Object(ObjectId::new(3))] {
            let frame = subscribe(1, filter);
            assert_eq!(read_subscribe(frame.payload().expect("payload")), Ok(filter));
        }
        let arguments = fields(&[("force", Value::Bool(true))]);
        let frame = call(2, ObjectId::new(3), "focus", &arguments);
        assert_eq!(
            read_call(frame.payload().expect("payload")),
            Ok((ObjectId::new(3), "focus".to_owned(), arguments))
        );
    }

    #[test]
    fn an_offer_and_a_finish_round_trip() {
        let frame = offer(
            1,
            RequestId::new(2),
            Tick::new(500),
            "auth.polkit",
            "Authentication required",
            "Authenticate to run /usr/bin/mount",
        );
        assert_eq!(
            decode(&frame).expect("decodes"),
            Some(Incoming::Offered {
                request: RequestId::new(2),
                deadline: Tick::new(500),
                slot: "auth.polkit".to_owned(),
                title: "Authentication required".to_owned(),
                message: "Authenticate to run /usr/bin/mount".to_owned(),
            })
        );
        let frame = finished(RequestId::new(2), RequestCompletion::TimedOut);
        assert_eq!(
            decode(&frame).expect("decodes"),
            Some(Incoming::Finished {
                request: RequestId::new(2),
                completion: RequestCompletion::TimedOut
            })
        );
    }

    #[test]
    fn the_offer_request_round_trips_its_proposition() {
        let frame = request_offer(1, "auth.polkit", "Title", "Message", 30_000);
        assert_eq!(
            read_offer(frame.payload().expect("payload")),
            Ok((
                "auth.polkit".to_owned(),
                "Title".to_owned(),
                "Message".to_owned(),
                30_000
            ))
        );
        assert!(
            !frame.is_sensitive(),
            "a proposition is what the provider draws, not a secret"
        );
    }

    #[test]
    fn a_slot_claim_and_a_release_round_trip() {
        let frame = slot_claim(1, "auth.polkit", Provider::Shell);
        assert_eq!(
            read_slot_claim(frame.payload().expect("payload")),
            Ok(("auth.polkit".to_owned(), Provider::Shell))
        );
        let frame = slot_claim(2, "prompt.secret", Provider::Services);
        assert_eq!(
            read_slot_claim(frame.payload().expect("payload")),
            Ok(("prompt.secret".to_owned(), Provider::Services))
        );
        let frame = slot_release(3, "auth.polkit");
        assert_eq!(
            read_slot_release(frame.payload().expect("payload")),
            Ok("auth.polkit".to_owned())
        );
    }

    #[test]
    fn a_configuration_commit_and_its_answer_round_trip() {
        let document = br#"{"input":{"repeat-delay":250}}"#.to_vec();
        let frame = config_commit(1, &document);
        assert_eq!(
            read_config_commit(frame.payload().expect("payload")),
            Ok(document.clone())
        );
        assert!(!frame.is_sensitive(), "a configuration document is not a secret");

        let payload = configuration_payload(7, &document);
        assert_eq!(read_configuration(&payload), Ok((7, document)));
        assert_eq!(read_configuration(&configuration_payload(0, &[])), Ok((0, Vec::new())));
    }

    #[test]
    fn an_answer_to_an_offered_request_is_flagged_and_redacts() {
        // The failure this prevents: the answer to a prompt travels back to the host
        // that offered it on a member the schema does not mark secret, so a log or
        // `gnoblinctl ipc dump` keeps the passphrase.
        const SECRET: &[u8] = b"hunter2";
        let frame = answered(RequestId::new(2), SECRET);
        assert_eq!(frame.member, member::ANSWER);
        assert!(frame.is_sensitive(), "the schema marks the answer field secret");
        assert_eq!(frame.redact().payload().map(<[u8]>::to_vec), Some(Vec::new()));
        assert_eq!(
            decode(&frame).expect("decodes"),
            Some(Incoming::Answered {
                request: RequestId::new(2),
                answer: SECRET.to_vec()
            })
        );
    }

    #[test]
    fn an_answer_frame_is_flagged_and_redacted_by_the_schema() {
        // The schema marks `request.answer` secret, so the generated table flags
        // every frame that carries an answer and redaction drops its bytes. This is
        // the property that keeps a prompt answer out of a log or a frame dump, and
        // it holds because this layer names the member the schema declares.
        let frame = answer(6, RequestId::new(2), b"hunter2");
        assert!(
            frame.is_sensitive(),
            "an answer carries secret material and must be flagged"
        );
        assert_eq!(
            frame.redact().payload().map(<[u8]>::to_vec),
            Some(Vec::new()),
            "redaction keeps the routing fields and drops the answer"
        );
    }

    #[test]
    fn a_decline_frame_is_not_flagged() {
        // Ending a request carries no value, so the frame is readable: the schema
        // marks no field of `request.end` secret, and the generated table agrees.
        let frame = decline(1, RequestId::new(2));
        assert_eq!(frame.member, member::DECLINE);
        assert!(!frame.is_sensitive(), "a declined request carries nothing secret");
        assert_eq!(frame.redact(), frame, "redaction leaves it alone");
        assert_eq!(read_decline(frame.payload().expect("payload")), Ok(RequestId::new(2)));
    }

    #[test]
    fn an_empty_answer_is_an_answer_not_a_decline() {
        // The failure this prevents: treating zero bytes as "no answer". An empty
        // answer is consent with no content, it travels on `request.answer` (which
        // is flagged), and it decodes to an empty value rather than to an end.
        let request = RequestId::new(4);
        let frame = answer(1, request, &[]);
        assert_eq!(frame.member, member::ANSWER, "an empty answer is an answer");
        assert!(frame.is_sensitive(), "the member is still the secret one");
        assert_eq!(
            read_answer(frame.payload().expect("payload")),
            Ok((request, Vec::new()))
        );
        assert_eq!(
            read_decline(decline(1, request).payload().expect("payload")),
            Ok(request)
        );
        assert_ne!(
            member::ANSWER,
            member::DECLINE,
            "the outcomes are different members, not different bytes"
        );
    }

    #[test]
    fn a_claim_frame_is_not_flagged() {
        // The other half of the rule: claiming names a request and nothing else, so
        // a dump of it is useful for debugging.
        assert!(!claim(1, RequestId::new(2)).is_sensitive());
        assert!(!offer(1, RequestId::new(2), Tick::new(500), "auth.polkit", "t", "m").is_sensitive());
    }

    #[test]
    fn a_request_answer_is_flagged_sensitive_and_redacts() {
        // The failure mode: a prompt's password travels in an answer, the layer is
        // not an object member, and nothing flags the frame, so a log or the parity
        // recorder keeps the bytes. `request.answer` is a schema member with a
        // secret field, so the envelope the client builds is flagged and the one
        // thing that may be shown loses the payload.
        const SECRET: &[u8] = b"hunter2";
        let frame = self::answer(6, RequestId::new(2), SECRET);
        assert_eq!(frame.member, member::ANSWER);
        assert!(frame.is_sensitive(), "an answer carries secret material");

        let payload = frame.payload().expect("payload");
        assert!(
            payload.windows(SECRET.len()).any(|window| window == SECRET),
            "the flag is what protects the bytes, not the encoding"
        );

        let redacted = frame.redact();
        assert!(redacted.is_sensitive(), "a redacted frame stays marked");
        assert_eq!(redacted.payload(), Some(&b""[..]));
        assert_eq!(redacted.member, member::ANSWER, "the routing fields survive");
    }

    #[test]
    fn a_finished_request_is_not_flagged() {
        // Only the frame that carries the answer is secret, so the completion every
        // subscriber sees stays readable.
        let frame = finished(RequestId::new(2), RequestCompletion::Answered);
        assert!(!frame.is_sensitive());
        assert_eq!(frame, frame.redact());
    }

    #[test]
    fn the_request_members_are_the_schema_members() {
        // The member paths on this layer are the schema's, not this file's opinion: a
        // rename in request.toml that this crate misses would silently stop flagging
        // answers, which is the failure the whole table exists to prevent.
        let catalog = Catalog::load_dir(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../gnoblin-schema/schema"
        )))
        .expect("the checked-in schema loads");
        let namespace = gnoblin_schema::Namespace::parse(NAMESPACE).expect("namespace");
        let request = catalog
            .objects(&namespace)
            .find(|object| object.name == "request")
            .expect("the schema declares the request object");

        let claimed: Vec<&str> = request.methods.iter().map(|method| method.name.as_str()).collect();
        assert_eq!(
            claimed,
            vec!["claim", "answer", "decline"],
            "the methods this layer serves"
        );
        let signalled: Vec<&str> = request.signals.iter().map(|signal| signal.name.as_str()).collect();
        assert_eq!(signalled, vec!["offered", "finished"], "the signals this layer serves");

        for (object, member, path) in [
            ("request", "claim", member::CLAIM),
            ("request", "answer", member::ANSWER),
            ("request", "decline", member::DECLINE),
            ("request", "offered", member::REQUEST_OFFERED),
            ("request", "finished", member::REQUEST_FINISHED),
        ] {
            assert_eq!(gnoblin_schema::proto::member_path(object, member), path);
        }

        let answered = request
            .methods
            .iter()
            .find(|method| method.name == "answer")
            .expect("answer is declared");
        assert!(answered.is_sensitive(), "an answer field is secret");
    }
}
