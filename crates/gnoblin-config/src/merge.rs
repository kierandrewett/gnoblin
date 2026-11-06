//! Layered documents: how several files become one.
//!
//! Gnoblin loads the built-in defaults, then system drop-ins, then the user's own
//! `init.lua` and everything it includes, and whatever loads later replaces what
//! loaded earlier. The rule is the one the reference documents: tables merge by
//! key, lists and scalars are replaced whole. A list is replaced because its
//! order and length are part of its meaning: appending a gesture from a drop-in
//! to a user's list would silently change which gesture fires, and half of a
//! workspace declaration is not a workspace declaration.
//!
//! Precedence is decided by position in the slice, never by the content: the
//! caller passes layers in load order and the last one wins. There is exactly one
//! place in the compositor that builds that order ([`crate::locate`] names the
//! files; the Lua host loads them), so two files can never disagree about which
//! one is later.

use crate::value::Value;

/// Merge `higher` over `lower`: tables merge by key, everything else is replaced.
#[must_use]
pub fn merge(lower: Value, higher: Value) -> Value {
    match (lower, higher) {
        (Value::Map(mut lower), Value::Map(higher)) => {
            for (key, value) in higher {
                let merged = match lower.remove(&key) {
                    Some(existing) => merge(existing, value),
                    None => value,
                };
                lower.insert(key, merged);
            }
            Value::Map(lower)
        }
        (_, higher) => higher,
    }
}

/// Merge layers in load order, so the last layer wins.
#[must_use]
pub fn merge_layers(layers: Vec<Value>) -> Value {
    layers
        .into_iter()
        .reduce(merge)
        .unwrap_or(Value::Map(Default::default()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn table<const N: usize>(entries: [(&str, Value); N]) -> Value {
        Value::table(entries)
    }

    #[test]
    fn tables_merge_by_key() {
        let lower = table([
            (
                "compositor",
                table([("debug", Value::Bool(false)), ("blur", Value::string("off"))]),
            ),
            ("cursor", table([("size", Value::Integer(24))])),
        ]);
        let higher = table([("compositor", table([("blur", Value::string("on"))]))]);
        let merged = merge(lower, higher);
        let compositor = merged
            .as_table()
            .and_then(|root| root.get("compositor"))
            .and_then(Value::as_table)
            .expect("compositor");
        assert_eq!(compositor.get("blur"), Some(&Value::string("on")));
        assert_eq!(compositor.get("debug"), Some(&Value::Bool(false)));
        assert!(merged.as_table().expect("root").contains_key("cursor"));
    }

    #[test]
    fn a_later_list_replaces_an_earlier_one() {
        let lower = table([("workspaces", Value::List(vec![Value::string("a"), Value::string("b")]))]);
        let higher = table([("workspaces", Value::List(vec![Value::string("c")]))]);
        let merged = merge(lower, higher);
        assert_eq!(
            merged.as_table().expect("root").get("workspaces"),
            Some(&Value::List(vec![Value::string("c")]))
        );
    }

    #[test]
    fn a_scalar_replaces_a_table_and_the_other_way_round() {
        let merged = merge(
            table([("cursor", table([("size", Value::Integer(24))]))]),
            table([("cursor", Value::Integer(32))]),
        );
        assert_eq!(
            merged.as_table().expect("root").get("cursor"),
            Some(&Value::Integer(32))
        );
    }

    #[test]
    fn the_last_layer_wins_regardless_of_key_order() {
        let first = table([(
            "cursor",
            table([("size", Value::Integer(24)), ("theme", Value::string("a"))]),
        )]);
        let second = table([("cursor", table([("size", Value::Integer(48))]))]);
        let merged = merge_layers(vec![first.clone(), second]);
        let cursor = merged
            .as_table()
            .and_then(|root| root.get("cursor"))
            .and_then(Value::as_table)
            .expect("cursor");
        assert_eq!(cursor.get("size"), Some(&Value::Integer(48)));
        assert_eq!(cursor.get("theme"), Some(&Value::string("a")));

        let reversed = merge_layers(vec![table([("cursor", table([("size", Value::Integer(48))]))]), first]);
        let cursor = reversed
            .as_table()
            .and_then(|root| root.get("cursor"))
            .and_then(Value::as_table)
            .expect("cursor");
        assert_eq!(
            cursor.get("size"),
            Some(&Value::Integer(24)),
            "the layer order decides precedence, so reversing it changes the result"
        );
    }

    #[test]
    fn no_layers_is_an_empty_document() {
        assert_eq!(merge_layers(Vec::new()), Value::Map(BTreeMap::new()));
    }
}
