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
/// `secret` (for example `bsm.greeter.auth_response`). Logs, the parity recorder
/// and `gnoblinctl ipc dump` call [`Envelope::redact`] before they show anything,
/// so none of them needs the extension's schema to keep the secret out.
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

#[cfg(test)]
mod tests {
    use prost::Message as _;

    use super::{Envelope, Kind, UNKNOWN_MEMBER};
    use crate::generated::bsm_greeter::greeter::AuthResponseArgs;
    use crate::generated::gnoblin::window::Properties;

    const SECRET: &str = "hunter2";

    #[test]
    fn a_schema_secret_member_is_always_flagged() {
        // Built through the public API with no extra call: the schema decides.
        let envelope = Envelope::request_message(
            "bsm.greeter",
            "auth_response",
            7,
            &AuthResponseArgs {
                response: SECRET.to_owned(),
            },
        );
        assert!(envelope.is_sensitive());
        assert_eq!(
            envelope.payload_as::<AuthResponseArgs>().expect("payload").response,
            SECRET
        );

        // Every constructor, not only requests.
        for envelope in [
            Envelope::request("bsm.greeter", "auth_response", 1, Vec::new()),
            Envelope::reply("bsm.greeter", "auth_response", 1, Vec::new()),
            Envelope::event("bsm.greeter", "auth_response", Vec::new()),
            Envelope::error("bsm.greeter", "auth_response", 1, UNKNOWN_MEMBER, "no"),
        ] {
            assert!(envelope.is_sensitive(), "{envelope:?}");
        }
    }

    #[test]
    fn a_plain_member_is_not_flagged() {
        let envelope = Envelope::request_message(
            "gnoblin",
            "focus",
            1,
            &Properties {
                id: 12,
                title: "editor".to_owned(),
                app_id: "code".to_owned(),
                focused: true,
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
            "bsm.greeter",
            "auth_response",
            7,
            &AuthResponseArgs {
                response: SECRET.to_owned(),
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
        let envelope = Envelope::error("bsm.greeter", "auth_response", 3, UNKNOWN_MEMBER, "no such member");
        let redacted = envelope.redact();
        assert_eq!(redacted.error_code(), Some("redacted"));
        assert!(matches!(redacted.kind, Some(Kind::Error(_))));
        assert_eq!(envelope.error_code(), Some(UNKNOWN_MEMBER));
    }
}
