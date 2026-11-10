//! `Debug` for the tree messages that can carry a call's text, written by hand.
//!
//! A `call` op's `text` is the chord of a seat claim or, for `authenticate`, a password.
//! prost's derived `Debug` would print it, so `build.rs` switches that derive off for the
//! op and the transaction that carries it, and both are formatted here. The text is shown
//! by length only, so no `{:?}` or `{:#?}` of a frame, a transaction or an op shows it.

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

#[cfg(test)]
mod tests {
    use crate::generated::tree;

    const PASSWORD: &str = "correct-horse-battery";

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
