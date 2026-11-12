//! `Debug` for the tree messages that can carry secret text, written by hand.
//!
//! A `call` op's `text` is the chord of a seat claim or, for `authenticate`, a password.
//! A key event's `key` and `text` name what was typed, which in a passphrase entry is the
//! passphrase. prost's derived `Debug` would print them, so `build.rs` switches that derive off
//! for the op, the transaction that carries it and the event, and all three are formatted here.
//! The text is shown by length only, so no `{:?}` or `{:#?}` of a frame, a transaction, an op or
//! an event shows it.

use std::fmt;

use crate::generated::tree;

impl fmt::Debug for tree::op::Properties {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Properties")
            .field("kind", &self.kind)
            .field("node", &self.node)
            .field("class", &self.class)
            .field("id", &self.id)
            .field("key", &self.key)
            .field("value_kind", &self.value_kind)
            .field("value_bool", &self.value_bool)
            .field("value_int", &self.value_int)
            .field("value_uint", &self.value_uint)
            .field("value_double", &self.value_double)
            .field("value_text", &self.value_text)
            .field("value_node", &self.value_node)
            .field("value_list", &self.value_list)
            .field("seconds", &self.seconds)
            .field("samples", &self.samples)
            .field("parent", &self.parent)
            .field("index", &self.index)
            .field("method", &self.method)
            .field("args", &format_args!("<{} bytes>", self.args.len()))
            .field("value_texts", &self.value_texts)
            .field("value_effects", &self.value_effects)
            .field("value_bytes", &format_args!("<{} bytes>", self.value_bytes.len()))
            .field("seat", &self.seat)
            .field("input_kind", &self.input_kind)
            .field("x", &self.x)
            .field("y", &self.y)
            .field("button", &self.button)
            .field("state", &self.state)
            .field("dx", &self.dx)
            .field("dy", &self.dy)
            .field("text", &format_args!("<redacted {} bytes>", self.text.len()))
            .finish()
    }
}

impl fmt::Debug for tree::tree::TransactArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransactArgs")
            .field("base_serial", &self.base_serial)
            .field("serial", &self.serial)
            .field("ops", &self.ops)
            .finish()
    }
}

impl fmt::Debug for tree::tree::EventPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventPayload")
            .field("frame", &self.frame)
            .field("seq", &self.seq)
            .field("node", &self.node)
            .field("kind", &self.kind)
            .field("x", &self.x)
            .field("y", &self.y)
            .field("button", &self.button)
            .field("key", &format_args!("<redacted {} bytes>", self.key.len()))
            .field("text", &format_args!("<redacted {} bytes>", self.text.len()))
            .field("dx", &self.dx)
            .field("dy", &self.dy)
            .field("request", &self.request)
            .field("pressed", &self.pressed)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use crate::generated::tree;

    const PASSWORD: &str = "correct-horse-battery";

    #[test]
    fn debug_of_a_key_event_never_shows_what_was_typed() {
        // `Z9` is a key name and a typed character at once: the shell reads the same letter as `key` and `text`.
        let event = tree::tree::EventPayload {
            frame: 3,
            seq: 4,
            node: 9,
            kind: "key".to_owned(),
            key: "Zq9".to_owned(),
            text: "Z".to_owned(),
            pressed: true,
            request: "activate".to_owned(),
            ..Default::default()
        };
        for shown in [format!("{event:?}"), format!("{event:#?}")] {
            assert!(!shown.contains("Zq9"), "{shown}");
            assert!(!shown.contains('Z'), "{shown}");
            assert!(shown.contains("<redacted 3 bytes>"), "{shown}");
            assert!(shown.contains("<redacted 1 bytes>"), "{shown}");
            assert!(shown.contains("kind: \"key\""), "{shown}");
            assert!(shown.contains("activate"), "{shown}");
        }
    }

    #[test]
    fn debug_of_an_op_and_a_transaction_never_shows_call_text() {
        let op = tree::op::Properties {
            kind: "call".to_owned(),
            node: 7,
            method: "authenticate".to_owned(),
            text: PASSWORD.to_owned(),
            ..Default::default()
        };
        let args = tree::tree::TransactArgs {
            base_serial: 1,
            serial: 2,
            ops: vec![op.clone()],
        };
        for shown in [
            format!("{op:?}"),
            format!("{op:#?}"),
            format!("{args:?}"),
            format!("{args:#?}"),
        ] {
            assert!(!shown.contains(PASSWORD), "{shown}");
            assert!(shown.contains("authenticate"), "{shown}");
            assert!(shown.contains("<redacted 21 bytes>"), "{shown}");
        }
    }
}
