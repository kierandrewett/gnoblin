//! Removing values that two runs of the same behaviour never agree on.
//!
//! A protocol trace carries object ids, serials and absolute times. Two targets
//! that behave identically still disagree on all three: the ids are allocated in
//! the order objects were created, the serials count requests, and a timestamp is
//! a wall-clock instant. Comparing raw observations would therefore report a
//! difference for every scenario, and a real difference would be invisible in the
//! noise. The normaliser removes what the contract does not define and keeps
//! everything it does (design/rust-compositor.md, "Normalisation").
//!
//! Two rules, both keyed on the key rather than on the value, so a value that
//! happens to look like an id is not touched:
//!
//! - A key naming an object (`id`, `window_id`, `surface_id`, ...) has its value
//!   replaced by `<id:N>`, where `N` counts distinct raw values in the order they
//!   were first seen. Two sides that create objects in the same order agree, and
//!   a reference to the same object in two places still agrees with itself.
//! - A key naming a serial, a timestamp or a process id is removed, key and all,
//!   because no comparable property of it survives.
//!
//! One normaliser instance covers one side of one observation; the runner builds
//! a fresh one for the expected value and another for the actual value of each
//! observation. A shared instance across a whole scenario would make the labels
//! depend on how many steps the scenario compares, which is not a property of the
//! behaviour under test.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

/// Keys whose value is an absolute time, a serial or a process id: removed
/// outright. The list is short on purpose; a key is added here only when the plan
/// or a scenario shows that two conforming targets disagree on it.
const VOLATILE_KEYS: &[&str] = &[
    "client_pid",
    "cpu",
    "created_at",
    "frame_time",
    "monotonic",
    "monotonic_time",
    "pid",
    "serial",
    "started_at",
    "time",
    "timestamp",
    "uptime",
    "wall_time",
];

/// Suffixes that mark a value as a serial, a timestamp or a process id wherever
/// the key appears, so `configure_serial`, `commit_msec` and `owner_pid` do not
/// each need an entry above.
const VOLATILE_SUFFIXES: &[&str] = &["_msec", "_nsec", "_pid", "_serial", "_timestamp", "_usec"];

/// Rewrites an observation into the form two targets can be compared in.
#[derive(Debug, Default)]
pub struct Normaliser {
    /// Raw id to label, keyed by `(key, raw value)` so the same object keeps one
    /// label and two objects never share one.
    labels: BTreeMap<(String, String), String>,
    volatile_keys: BTreeSet<String>,
    volatile_suffixes: Vec<String>,
}

impl Normaliser {
    /// A normaliser with the plan's default rules.
    pub fn new() -> Self {
        Self {
            labels: BTreeMap::new(),
            volatile_keys: VOLATILE_KEYS.iter().map(|key| (*key).to_owned()).collect(),
            volatile_suffixes: VOLATILE_SUFFIXES.iter().map(|suffix| (*suffix).to_owned()).collect(),
        }
    }

    /// Normalise one value.
    ///
    /// Keys are visited in sorted order (`serde_json` keeps object keys sorted),
    /// so the label numbering is reproducible even though JSON objects carry no
    /// order of their own. Array order is preserved: the contract defines it
    /// wherever a scenario relies on it.
    pub fn normalise(&mut self, value: &Value) -> Value {
        match value {
            Value::Object(object) => Value::Object(self.normalise_object(object)),
            Value::Array(items) => Value::Array(items.iter().map(|item| self.normalise(item)).collect()),
            other => other.clone(),
        }
    }

    fn normalise_object(&mut self, object: &Map<String, Value>) -> Map<String, Value> {
        let mut normalised = Map::new();
        for (key, value) in object {
            if self.is_volatile(key) {
                continue;
            }
            if is_object_id(key) {
                let label = self.label(key, value);
                normalised.insert(key.clone(), Value::String(label));
                continue;
            }
            normalised.insert(key.clone(), self.normalise(value));
        }
        normalised
    }

    fn is_volatile(&self, key: &str) -> bool {
        self.volatile_keys.contains(key)
            || self
                .volatile_suffixes
                .iter()
                .any(|suffix| key.ends_with(suffix.as_str()))
    }

    fn label(&mut self, key: &str, raw: &Value) -> String {
        let identity = (key.to_owned(), raw.to_string());
        if let Some(label) = self.labels.get(&identity) {
            return label.clone();
        }
        let label = format!("<id:{}>", self.labels.len() + 1);
        self.labels.insert(identity, label.clone());
        label
    }
}

/// Whether a key names an object whose numeric or string value is an allocation
/// order artefact.
fn is_object_id(key: &str) -> bool {
    key == "id" || key.ends_with("_id")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn normalised(value: Value) -> Value {
        Normaliser::new().normalise(&value)
    }

    #[test]
    fn ids_are_remapped_by_first_seen_order() {
        let value = json!({
            "windows": [
                { "id": 42, "title": "term" },
                { "id": 7, "title": "files" }
            ]
        });
        assert_eq!(
            normalised(value),
            json!({
                "windows": [
                    { "id": "<id:1>", "title": "term" },
                    { "id": "<id:2>", "title": "files" }
                ]
            })
        );
    }

    #[test]
    fn two_recordings_that_differ_only_in_volatile_values_are_equal() {
        let reference = json!({
            "windows": [{ "id": 3, "serial": 91, "timestamp": 1767225600123_u64, "pid": 4211, "title": "term" }],
            "output": { "name": "eDP-1", "configure_serial": 12 }
        });
        let candidate = json!({
            "windows": [{ "id": 51, "serial": 4, "timestamp": 1767225699999_u64, "pid": 8, "title": "term" }],
            "output": { "name": "eDP-1", "configure_serial": 99 }
        });
        assert_eq!(normalised(reference), normalised(candidate));
    }

    #[test]
    fn the_same_object_in_two_places_keeps_one_label() {
        // A trace mentions the focused window by id in two records; the second
        // mention must not become a new object.
        let value = json!({
            "window": { "id": 5 },
            "focus": { "id": 5 }
        });
        assert_eq!(
            normalised(value),
            json!({
                "window": { "id": "<id:1>" },
                "focus": { "id": "<id:1>" }
            })
        );
    }

    #[test]
    fn the_same_key_and_raw_value_always_gets_the_same_label() {
        let mut normaliser = Normaliser::new();
        let first = normaliser.normalise(&json!({ "window_id": 5 }));
        let second = normaliser.normalise(&json!({ "window_id": 5 }));
        assert_eq!(first, second);
        assert_eq!(first["window_id"], json!("<id:1>"));
    }

    #[test]
    fn a_real_difference_survives_normalisation() {
        let reference = json!({ "windows": [{ "id": 3, "title": "term", "state": "maximized" }] });
        let candidate = json!({ "windows": [{ "id": 3, "title": "term", "state": "normal" }] });
        assert_ne!(normalised(reference), normalised(candidate));
    }

    #[test]
    fn a_value_that_looks_like_an_id_is_not_touched() {
        let value = json!({ "title": "<id:1>", "note": "serial 12" });
        assert_eq!(normalised(value.clone()), value);
    }

    #[test]
    fn volatile_suffixes_remove_the_whole_key() {
        let value = json!({ "commit_msec": 8, "owner_pid": 4, "kind": "keep" });
        assert_eq!(normalised(value), json!({ "kind": "keep" }));
    }

    #[test]
    fn array_order_is_preserved() {
        let value = json!(["b", "a"]);
        assert_eq!(normalised(value), json!(["b", "a"]));
    }
}
