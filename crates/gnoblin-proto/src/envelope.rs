//! The envelope's Rust helpers, on top of the generated types.
//!
//! The message itself is `proto/envelope.proto`, compiled by `build.rs` with the
//! documents it generates from the schema, so the struct and its tags come from the
//! text rather than from hand-written Rust that could drift. This module adds the behaviour the
//! wire type cannot express: constructors that pair a frame with its request id,
//! redaction, and the rule that sets [`FLAG_SENSITIVE`] from the schema.
//!
//! Sensitivity is not the caller's decision. The table `build.rs` generates from the
//! schema (`generated::sensitive`) lists the members marked secret; every
//! constructor flags a frame for one of them, and the frame decoder re-applies the
//! flag, so a peer that sends zero cannot clear it. There is no public way to set or
//! clear the flag by hand.

use crate::generated::ipc::envelope::Kind;
use crate::generated::ipc::{Envelope, Error, Event, Reply, Request};
use crate::generated::sensitive;

/// The envelope flag that marks a frame as carrying secret material.
///
/// The generated client sets it on any message with a field the schema marks
/// `secret` (for example `request.answer`, whose `answer` field carries a password).
/// The member is the schema's qualified member path, so an extension's frames are
/// named the same way. Logs, the parity recorder and `gnoblinctl ipc dump` call
/// [`Envelope::redact`] before they show anything, so none of them needs the
/// extension's schema to keep the secret out.
pub const FLAG_SENSITIVE: u32 = 1 << 0;

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
        Envelope::build(namespace, member, request_id, Kind::Request(Request { payload }))
    }

    /// A request whose payload is a generated message.
    pub fn request_message<M: prost::Message>(
        namespace: impl Into<String>,
        member: impl Into<String>,
        request_id: u64,
        message: &M,
    ) -> Envelope {
        Envelope::request(namespace, member, request_id, message.encode_to_vec())
    }

    /// The answer to a request.
    pub fn reply(
        namespace: impl Into<String>,
        member: impl Into<String>,
        request_id: u64,
        payload: Vec<u8>,
    ) -> Envelope {
        Envelope::build(namespace, member, request_id, Kind::Reply(Reply { payload }))
    }

    /// The answer to a request, whose payload is a generated message.
    pub fn reply_message<M: prost::Message>(
        namespace: impl Into<String>,
        member: impl Into<String>,
        request_id: u64,
        message: &M,
    ) -> Envelope {
        Envelope::reply(namespace, member, request_id, message.encode_to_vec())
    }

    /// An unsolicited event. Request id is zero because nothing is waiting.
    pub fn event(namespace: impl Into<String>, member: impl Into<String>, payload: Vec<u8>) -> Envelope {
        Envelope::build(namespace, member, 0, Kind::Event(Event { payload }))
    }

    /// An event whose payload is a generated message.
    pub fn event_message<M: prost::Message>(
        namespace: impl Into<String>,
        member: impl Into<String>,
        message: &M,
    ) -> Envelope {
        Envelope::event(namespace, member, message.encode_to_vec())
    }

    /// A failed request.
    pub fn error(
        namespace: impl Into<String>,
        member: impl Into<String>,
        request_id: u64,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Envelope {
        Envelope::build(
            namespace,
            member,
            request_id,
            Kind::Error(Error {
                code: code.into(),
                message: message.into(),
            }),
        )
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

    /// The payload decoded as a generated message, or `None` when this kind has no
    /// payload or it does not decode.
    pub fn payload_as<M: prost::Message + Default>(&self) -> Option<M> {
        M::decode(self.payload()?).ok()
    }

    /// The error code, if this is an error frame.
    pub fn error_code(&self) -> Option<&str> {
        match &self.kind {
            Some(Kind::Error(error)) => Some(&error.code),
            _ => None,
        }
    }

    /// The only way to show a frame to a log, a dump or a recorder.
    ///
    /// A sensitive frame keeps its routing fields and loses its payload; a plain
    /// frame comes back unchanged. Because the secret bytes are dropped here
    /// rather than at each call site, no consumer has to know which members are
    /// secret.
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

    /// Build a frame and let the schema decide the sensitive flag.
    fn build(namespace: impl Into<String>, member: impl Into<String>, request_id: u64, kind: Kind) -> Envelope {
        let mut envelope = Envelope {
            namespace: namespace.into(),
            member: member.into(),
            request_id,
            flags: 0,
            kind: Some(kind),
        };
        envelope.flag_schema_sensitivity();
        envelope
    }

    /// Set [`FLAG_SENSITIVE`] when the schema says this member is secret.
    ///
    /// Called by every constructor and by the frame decoder, so neither a forgetful
    /// caller nor a peer that omits the flag can make a secret frame look plain.
    pub(crate) fn flag_schema_sensitivity(&mut self) {
        if sensitive::is_sensitive(&self.namespace, &self.member) {
            self.flags |= FLAG_SENSITIVE;
        }
    }
}

impl Envelope {
    /// Overwrite the payload of a sensitive frame in place.
    ///
    /// The payload is the only field the schema can mark secret, so the routing fields
    /// are left alone. A plain frame is untouched, so this is always safe to call.
    pub fn wipe_secret(&mut self) {
        if !self.is_sensitive() {
            return;
        }
        match &mut self.kind {
            Some(Kind::Request(request)) => zeroize::Zeroize::zeroize(&mut request.payload),
            Some(Kind::Reply(reply)) => zeroize::Zeroize::zeroize(&mut reply.payload),
            Some(Kind::Event(event)) => zeroize::Zeroize::zeroize(&mut event.payload),
            Some(Kind::Error(_)) | None => {}
        }
    }
}

// Debug is written by hand because the derived one prints the payload bytes. A frame
// that carries a secret therefore shows its routing and its length, never its contents.
impl std::fmt::Debug for Envelope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The oneof has no `Debug` of its own, and a payload is shown by length only.
        let (kind, len) = match &self.kind {
            Some(Kind::Request(request)) => ("request", request.payload.len()),
            Some(Kind::Reply(reply)) => ("reply", reply.payload.len()),
            Some(Kind::Event(event)) => ("event", event.payload.len()),
            Some(Kind::Error(error)) => ("error", error.code.len()),
            None => ("none", 0),
        };
        f.debug_struct("Envelope")
            .field("namespace", &self.namespace)
            .field("member", &self.member)
            .field("request_id", &self.request_id)
            .field("flags", &self.flags)
            .field("kind", &kind)
            .field("payload_len", &len)
            .finish()
    }
}

impl Drop for Envelope {
    fn drop(&mut self) {
        // A secret frame is wiped on the way out, so the broker's copy of an answer does
        // not outlive the request it was for. Plain frames are left as they are.
        self.wipe_secret();
    }
}

#[cfg(test)]
mod tests {
    use prost::Message as _;

    use super::{Envelope, Kind, UNKNOWN_MEMBER};
    use crate::generated::gnoblin::request::AnswerArgs;
    use crate::generated::gnoblin::window::Properties;

    const SECRET: &str = "hunter2";

    #[test]
    fn a_schema_secret_member_is_always_flagged() {
        // Built through the public API with no extra call: the schema decides.
        let envelope = Envelope::request_message(
            "gnoblin",
            "request.answer",
            7,
            &AnswerArgs {
                request: 1,
                answer: SECRET.as_bytes().to_vec(),
            },
        );
        assert!(envelope.is_sensitive());
        assert_eq!(
            envelope.payload_as::<AnswerArgs>().expect("payload").answer,
            SECRET.as_bytes()
        );

        // Every constructor, not only requests.
        for envelope in [
            Envelope::request("gnoblin", "request.answer", 1, Vec::new()),
            Envelope::reply("gnoblin", "request.answer", 1, Vec::new()),
            Envelope::event("gnoblin", "request.answer", Vec::new()),
            Envelope::error("gnoblin", "request.answer", 1, UNKNOWN_MEMBER, "no"),
        ] {
            assert!(envelope.is_sensitive(), "{envelope:?}");
        }
    }

    #[test]
    fn a_plain_member_is_not_flagged() {
        let envelope = Envelope::request_message(
            "gnoblin",
            "window.focus",
            1,
            // Only the members this test reads, so a schema that adds a property
            // does not break a test about sensitivity.
            &Properties {
                id: 12,
                title: "editor".to_owned(),
                app_id: "code".to_owned(),
                focused: true,
                // Only the members this test reads: a property the schema adds
                // later must not break a test about sensitivity.
                ..Properties::default()
            },
        );
        assert!(!envelope.is_sensitive());
        assert_eq!(
            envelope.payload_as::<Properties>().expect("payload").title,
            "editor".to_owned()
        );
    }

    #[test]
    fn redaction_drops_the_secret_bytes() {
        let envelope = Envelope::request_message(
            "gnoblin",
            "request.answer",
            7,
            &AnswerArgs {
                request: 1,
                answer: SECRET.as_bytes().to_vec(),
            },
        );
        let encoded = envelope.encode_to_vec();
        assert!(
            encoded.windows(SECRET.len()).any(|window| window == SECRET.as_bytes()),
            "the flag is what protects the bytes, not the encoding"
        );

        let redacted = envelope.redact();
        assert!(
            redacted.is_sensitive(),
            "a redacted frame stays marked so nothing re-logs it as plain"
        );
        assert_eq!(redacted.payload(), Some(&b""[..]));
        let encoded = redacted.encode_to_vec();
        assert!(
            !encoded.windows(SECRET.len()).any(|window| window == SECRET.as_bytes()),
            "a redacted frame must not contain the secret"
        );
    }

    #[test]
    fn a_plain_frame_redacts_to_itself() {
        let envelope = Envelope::event("gnoblin", "ready", vec![1, 2, 3]);
        assert!(!envelope.is_sensitive());
        assert_eq!(envelope.redact(), envelope);
    }

    #[test]
    fn an_error_redacts_to_its_code() {
        let envelope = Envelope::error("gnoblin", "request.answer", 3, UNKNOWN_MEMBER, "no such member");
        let redacted = envelope.redact();
        assert_eq!(redacted.error_code(), Some("redacted"));
        assert!(matches!(redacted.kind, Some(Kind::Error(_))));
        assert_eq!(envelope.error_code(), Some(UNKNOWN_MEMBER));
    }

    #[test]
    fn debug_never_prints_the_secret_bytes() {
        let envelope = Envelope::request_message(
            "gnoblin",
            "request.answer",
            7,
            &AnswerArgs {
                request: 1,
                answer: SECRET.as_bytes().to_vec(),
            },
        );
        let shown = format!("{envelope:?}");
        assert!(!shown.contains(SECRET), "a log line must not carry the secret: {shown}");
        assert!(shown.contains("payload_len"), "the length is still shown: {shown}");
    }

    #[test]
    fn wipe_secret_empties_a_sensitive_payload_in_place() {
        let mut envelope = Envelope::request_message(
            "gnoblin",
            "request.answer",
            7,
            &AnswerArgs {
                request: 1,
                answer: SECRET.as_bytes().to_vec(),
            },
        );
        envelope.wipe_secret();
        assert_eq!(envelope.payload(), Some(&b""[..]));
        assert!(envelope.is_sensitive(), "the frame stays marked after it is wiped");
    }

    #[test]
    fn wipe_secret_leaves_a_plain_payload_alone() {
        let mut envelope = Envelope::event("gnoblin", "ready", vec![1, 2, 3]);
        envelope.wipe_secret();
        assert_eq!(envelope.payload(), Some(&[1, 2, 3][..]));
    }
}
