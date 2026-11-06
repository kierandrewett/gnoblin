//! The neutral value tree a configuration document is built from.
//!
//! The Lua host evaluates `init.lua` and hands over a tree of tables, lists and
//! scalars; this crate never sees Lua. That boundary is why the tree is a plain
//! enum instead of a Serde model: a Serde model would force the host to produce
//! this crate's field names, and a tree keeps the document layer free to report
//! problems about settings it does not model yet.
//!
//! Tables are [`BTreeMap`]s, so iteration order is the sorted key order. Two
//! consequences are deliberate: a document always renders the same way, which
//! makes the stored last-good file stable and diffable, and diagnostics arrive in
//! a predictable order, which makes a test assert on the first problem without
//! depending on hash order.

use std::collections::BTreeMap;

use crate::path::Path;
use crate::problem::{Fault, Problem};

/// The deepest nesting the parser will follow.
///
/// The C configuration core rejects Lua tables deeper than 64 levels, because a
/// recursive walk of an adversarially nested table would exhaust the stack. The
/// same limit applies here for the same reason: the tree can arrive from a
/// script, and a stack overflow kills the session where a diagnostic does not.
pub const MAX_DEPTH: usize = 64;

/// A configuration value: what a Lua table, list or scalar becomes.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// Lua `nil`, which a table never stores but a list entry could hold.
    Null,
    /// Lua `true` or `false`.
    Bool(bool),
    /// A Lua integer, stored exactly.
    Integer(i64),
    /// A Lua number with a fractional part.
    Float(f64),
    /// Lua `string`.
    String(String),
    /// A Lua array, in order. Order is significant: workspaces and gestures are
    /// lists because position is part of their meaning.
    List(Vec<Value>),
    /// A Lua table, by key.
    Map(BTreeMap<String, Value>),
}

impl Value {
    /// A table built from pairs.
    pub fn table<const N: usize>(entries: [(&str, Value); N]) -> Value {
        Value::Map(
            entries
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect(),
        )
    }

    /// A string value.
    pub fn string(text: impl Into<String>) -> Value {
        Value::String(text.into())
    }

    /// The word used in a diagnostic for the shape of this value.
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Null => "nothing",
            Value::Bool(_) => "a boolean",
            Value::Integer(_) => "an integer",
            Value::Float(_) => "a number",
            Value::String(_) => "a string",
            Value::List(_) => "a list",
            Value::Map(_) => "a table",
        }
    }

    /// The table if this is one.
    pub fn as_table(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Value::Map(entries) => Some(entries),
            _ => None,
        }
    }

    /// The list if this is one.
    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(items) => Some(items),
            _ => None,
        }
    }

    /// The string if this is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(text) => Some(text),
            _ => None,
        }
    }

    /// The boolean if this is one.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(flag) => Some(*flag),
            _ => None,
        }
    }

    /// The integer if this is one. A float is not an integer even when it has no
    /// fractional part: `size = 24.0` is a mistake worth reporting, and silently
    /// truncating it would hide a value the writer did not mean.
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Value::Integer(number) => Some(*number),
            _ => None,
        }
    }

    /// The value as a finite float, accepting an integer.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Integer(number) => Some(*number as f64),
            Value::Float(number) if number.is_finite() => Some(*number),
            _ => None,
        }
    }

    /// Every problem that makes the whole tree unusable: a branch nested past
    /// [`MAX_DEPTH`] and a float that is not finite.
    ///
    /// These are checked before any section, because a tree this broken cannot be
    /// walked safely or stored: the last-good file is JSON, and JSON has no
    /// spelling for infinity.
    pub fn tree_problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        visit(self, &Path::root(), 0, &mut problems);
        problems
    }

    /// The plain JSON form, for the last-good store.
    ///
    /// A document that passed [`Value::tree_problems`] converts without loss:
    /// JSON distinguishes an integer from a float with a fractional part, and
    /// every float is finite.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Value::Null => serde_json::Value::Null,
            Value::Bool(flag) => serde_json::Value::Bool(*flag),
            Value::Integer(number) => serde_json::Value::from(*number),
            Value::Float(number) => {
                serde_json::Number::from_f64(*number).map_or(serde_json::Value::Null, serde_json::Value::Number)
            }
            Value::String(text) => serde_json::Value::String(text.clone()),
            Value::List(items) => serde_json::Value::Array(items.iter().map(Value::to_json).collect()),
            Value::Map(entries) => serde_json::Value::Object(
                entries
                    .iter()
                    .map(|(key, value)| (key.clone(), value.to_json()))
                    .collect(),
            ),
        }
    }

    /// Read a document back from its plain JSON form.
    ///
    /// `None` when the JSON holds something a configuration document cannot: a
    /// float that is not finite, or an unsigned integer outside `i64`.
    pub fn from_json(json: &serde_json::Value) -> Option<Value> {
        let value = match json {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(flag) => Value::Bool(*flag),
            serde_json::Value::Number(number) => {
                if let Some(integer) = number.as_i64() {
                    Value::Integer(integer)
                } else {
                    // Either a float, or an unsigned integer beyond `i64::MAX`,
                    // which the document layer never writes.
                    Value::Float(number.as_f64().filter(|number| number.is_finite())?)
                }
            }
            serde_json::Value::String(text) => Value::String(text.clone()),
            serde_json::Value::Array(items) => {
                Value::List(items.iter().map(Value::from_json).collect::<Option<Vec<Value>>>()?)
            }
            serde_json::Value::Object(entries) => Value::Map(
                entries
                    .iter()
                    .map(|(key, value)| Value::from_json(value).map(|value| (key.clone(), value)))
                    .collect::<Option<BTreeMap<String, Value>>>()?,
            ),
        };
        Some(value)
    }
}

/// Report a too-deep branch once and a non-finite float wherever it is.
fn visit(value: &Value, path: &Path, depth: usize, problems: &mut Vec<Problem>) {
    if depth > MAX_DEPTH {
        problems.push(Problem::new(path.clone(), Fault::TooDeep { maximum: MAX_DEPTH }));
        return;
    }
    match value {
        Value::Float(number) if !number.is_finite() => {
            problems.push(Problem::new(path.clone(), Fault::NotFinite));
        }
        Value::List(items) => {
            for (index, item) in items.iter().enumerate() {
                visit(item, &path.entry(index), depth + 1, problems);
            }
        }
        Value::Map(entries) => {
            for (key, item) in entries {
                visit(item, &path.key(key), depth + 1, problems);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested(depth: usize) -> Value {
        let mut value = Value::Integer(1);
        for _ in 0..depth {
            value = Value::table([("k", value)]);
        }
        value
    }

    #[test]
    fn a_plain_tree_has_no_tree_problems() {
        let value = Value::table([
            ("compositor", Value::table([("debug", Value::Bool(true))])),
            ("workspaces", Value::List(vec![Value::string("main")])),
        ]);
        assert!(value.tree_problems().is_empty());
    }

    #[test]
    fn an_infinite_float_is_reported_where_it_is() {
        let value = Value::table([("compositor", Value::table([("blur", Value::Float(f64::INFINITY))]))]);
        let problems = value.tree_problems();
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].path.to_string(), "compositor.blur");
        assert_eq!(problems[0].fault, Fault::NotFinite);
    }

    #[test]
    fn the_deepest_accepted_tree_passes_and_one_more_level_fails() {
        assert!(nested(MAX_DEPTH).tree_problems().is_empty());
        let problems = nested(MAX_DEPTH + 1).tree_problems();
        assert_eq!(problems.len(), 1);
        assert!(matches!(problems[0].fault, Fault::TooDeep { maximum } if maximum == MAX_DEPTH));
    }

    #[test]
    fn integers_and_floats_survive_the_json_round_trip_apart() {
        let value = Value::table([
            ("integer", Value::Integer(24)),
            ("float", Value::Float(1.5)),
            ("text", Value::string("on")),
            ("list", Value::List(vec![Value::Bool(false), Value::Null])),
        ]);
        let json = serde_json::to_string(&value.to_json()).expect("serialise");
        let restored = Value::from_json(&serde_json::from_str(&json).expect("parse")).expect("restore");
        assert_eq!(restored, value);
        assert!(matches!(
            restored.as_table().expect("table").get("float"),
            Some(Value::Float(_))
        ));
    }
}
