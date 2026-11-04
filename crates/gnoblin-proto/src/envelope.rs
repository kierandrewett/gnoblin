//! The RPC envelope every frame carries.
//!
//! Gnoblin owns this layer rather than using a Protocol Buffers service
//! definition (design/rust-compositor.md, D16): the compositor has to route an
//! extension's frames by namespace without decoding the extension's payload, and
//! has to drop the payload of a `sensitive` frame before anything logs it. Both
//! are envelope work, so the envelope is a message of its own.
//!
//! Tags are fixed for good. A field number that has been used is never reused,
//! even after a member is removed, because a client built against an older
//! version would silently decode the new meaning.

/// The envelope flag that marks a frame as carrying secret material.
/// The generated client sets it on any message with a field the schema marks
/// `secret` (for example `bsm.greeter.auth_response`). Logs, the parity recorder
/// and `gnoblinctl ipc dump` drop the payload of such a frame, so none of them
/// needs the extension's schema to keep the secret out.
pub const FLAG_SENSITIVE: u32 = 1 << 0;

/// One IPC message: where it goes, what it is, and the kind-specific body.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Envelope {
    /// The schema namespace the member belongs to, for example `gnoblin` or
    /// `bsm.greeter`. The compositor routes on this and never decodes a payload
    /// from a namespace it does not serve itself.
    #[prost(string, tag = "1")]
    pub namespace: String,
    /// The object type the message is about, such as `window`.
    #[prost(string, tag = "2")]
    pub member: String,
    /// Pairs a reply or an error with its request. Zero on events.
    #[prost(uint64, tag = "3")]
    pub request_id: u64,
    /// See [`FLAG_SENSITIVE`]. Absent flags are zero, so a client that sends
    /// nothing means "not sensitive".
    #[prost(uint32, tag = "4")]
    pub flags: u32,
    #[prost(oneof = "Kind", tags = "5, 6, 7, 8")]
    pub kind: Option<Kind>,
}

/// The four kinds of frame.
#[derive(Clone, PartialEq, prost::Oneof)]
pub enum Kind {
    #[prost(message, tag = "5")]
    Request(Request),
    #[prost(message, tag = "6")]
    Reply(Reply),
    #[prost(message, tag = "7")]
    Event(Event),
    #[prost(message, tag = "8")]
    Error(Error),
}

/// A call: `member` in `namespace`, answered by a [`Reply`] or an [`Error`] with
/// the same request id.
///
/// The payload is a message generated from the schema for that member, or an
/// extension's own message, which the compositor forwards undecoded.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Request {
    #[prost(bytes = "vec", tag = "1")]
    pub payload: Vec<u8>,
}

/// The answer to a request.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Reply {
    #[prost(bytes = "vec", tag = "1")]
    pub payload: Vec<u8>,
}

/// Something that happened without anyone asking: a property change, a signal, a
/// completion.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Event {
    #[prost(bytes = "vec", tag = "1")]
    pub payload: Vec<u8>,
}

/// A request that failed.
///
/// The code is the schema's kebab-case error name, carried as text rather than as
/// a number so an older peer can still report a code this build has never heard
/// of.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Error {
    #[prost(string, tag = "1")]
    pub code: String,
    /// Human-readable detail. Never carries a secret: a message like "wrong
    /// password" is not the password, and the reply to a sensitive request is
    /// sensitive itself.
    #[prost(string, tag = "2")]
    pub message: String,
}

/// The error code a peer sends when it does not serve the requested member.
///
/// Matches the `unknown-member` error in `crates/gnoblin-schema/schema/session.toml`.
pub const UNKNOWN_MEMBER: &str = "unknown-member";

impl Envelope {
    /// A request whose answer is expected.
    pub fn request(
        namespace: impl Into<String>,
        member: impl Into<String>,
        request_id: u64,
        payload: Vec<u8>,
    ) -> Envelope {
        Envelope {
            namespace: namespace.into(),
            member: member.into(),
            request_id,
            flags: 0,
            kind: Some(Kind::Request(Request { payload })),
        }
    }

    /// The answer to a request.
    pub fn reply(
        namespace: impl Into<String>,
        member: impl Into<String>,
        request_id: u64,
        payload: Vec<u8>,
    ) -> Envelope {
        Envelope {
            namespace: namespace.into(),
            member: member.into(),
            request_id,
            flags: 0,
            kind: Some(Kind::Reply(Reply { payload })),
        }
    }

    /// An unsolicited event. Request id is zero because nothing is waiting.
    pub fn event(namespace: impl Into<String>, member: impl Into<String>, payload: Vec<u8>) -> Envelope {
        Envelope {
            namespace: namespace.into(),
            member: member.into(),
            request_id: 0,
            flags: 0,
            kind: Some(Kind::Event(Event { payload })),
        }
    }

    /// A failed request.
    pub fn error(
        namespace: impl Into<String>,
        member: impl Into<String>,
        request_id: u64,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Envelope {
        Envelope {
            namespace: namespace.into(),
            member: member.into(),
            request_id,
            flags: 0,
            kind: Some(Kind::Error(Error {
                code: code.into(),
                message: message.into(),
            })),
        }
    }

    /// Mark the frame as carrying secret material.
    pub fn mark_sensitive(&mut self) -> &mut Envelope {
        self.flags |= FLAG_SENSITIVE;
        self
    }

    /// Whether the payload is secret and must not be logged or stored.
    pub fn is_sensitive(&self) -> bool {
        self.flags & FLAG_SENSITIVE != 0
    }

    /// The payload bytes, if this frame kind carries any.
    pub fn payload(&self) -> Option<&[u8]> {
        match &self.kind {
            Some(Kind::Request(request)) => Some(&request.payload),
            Some(Kind::Reply(reply)) => Some(&reply.payload),
            Some(Kind::Event(event)) => Some(&event.payload),
            Some(Kind::Error(_)) | None => None,
        }
    }

    /// The error code, if this is an error frame.
    pub fn error_code(&self) -> Option<&str> {
        match &self.kind {
            Some(Kind::Error(error)) => Some(&error.code),
            _ => None,
        }
    }

    /// A copy safe to log or record: a sensitive frame keeps its routing fields
    /// and loses its payload.
    pub fn redact(&self) -> Envelope {
        if !self.is_sensitive() {
            return self.clone();
        }
        let kind = match &self.kind {
            Some(Kind::Request(_)) => Some(Kind::Request(Request::default())),
            Some(Kind::Reply(_)) => Some(Kind::Reply(Reply::default())),
            Some(Kind::Event(_)) => Some(Kind::Event(Event::default())),
            Some(Kind::Error(_)) => Some(Kind::Error(Error {
                code: "redacted".to_owned(),
                message: String::new(),
            })),
            None => None,
        };
        Envelope {
            namespace: self.namespace.clone(),
            member: self.member.clone(),
            request_id: self.request_id,
            flags: self.flags,
            kind,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Envelope, Kind, UNKNOWN_MEMBER};

    #[test]
    fn a_sensitive_frame_redacts_its_payload() {
        let mut envelope = Envelope::request("bsm.greeter", "greeter", 7, b"hunter2".to_vec());
        envelope.mark_sensitive();
        assert!(envelope.is_sensitive());
        assert_eq!(envelope.payload(), Some(&b"hunter2"[..]));

        let redacted = envelope.redact();
        assert!(
            redacted.is_sensitive(),
            "the flag survives so consumers keep treating it as secret"
        );
        assert_eq!(redacted.payload(), Some(&b""[..]));
        assert_eq!(redacted.member, "greeter");
        assert_eq!(redacted.request_id, 7);
    }

    #[test]
    fn a_plain_frame_redacts_to_itself() {
        let envelope = Envelope::event("gnoblin", "session", vec![1, 2, 3]);
        assert!(!envelope.is_sensitive());
        assert_eq!(envelope.redact(), envelope);
    }

    #[test]
    fn an_error_redacts_to_its_code() {
        let mut envelope = Envelope::error("gnoblin", "window", 3, UNKNOWN_MEMBER, "no such member");
        envelope.mark_sensitive();
        let redacted = envelope.redact();
        assert_eq!(redacted.error_code(), Some("redacted"));
        assert!(matches!(redacted.kind, Some(Kind::Error(_))));
        assert_eq!(envelope.error_code(), Some(UNKNOWN_MEMBER));
    }
}
