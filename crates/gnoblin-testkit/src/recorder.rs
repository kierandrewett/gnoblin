//! Recording observations as JSON, with secrets dropped before they are stored.
//!
//! A parity trace is checked in, pasted into issues and read by people who are
//! not the session owner, so a value that is secret must never reach the file in
//! the first place. Redaction after the fact would leave the secret in a buffer
//! and in any intermediate copy; this recorder makes the drop structural: the
//! only way to record a sensitive observation is a call that discards the value
//! before the observation is stored, and the stored observation then has no value
//! field at all.

use std::fmt;
use std::io;
use std::path::Path;

use serde_json::{Map, Value, json};

use crate::clock::Clock;

/// The observations of one target run, in the order they were recorded.
#[derive(Debug)]
pub struct Recorder {
    clock: Clock,
    observations: Vec<Observation>,
}

/// One recorded observation.
///
/// `value` is `None` exactly when the observation was flagged sensitive, so the
/// stored shape cannot carry a secret even by accident.
#[derive(Debug)]
struct Observation {
    name: String,
    at_millis: u64,
    value: Option<Value>,
}

impl Recorder {
    /// A recorder on the default deterministic clock.
    pub fn new() -> Self {
        Self::with_clock(Clock::new())
    }

    /// A recorder whose stamps come from `clock`. A scenario that models a
    /// timeout passes a clock with a wide step so the observation tells the
    /// reader how much virtual time passed.
    pub fn with_clock(clock: Clock) -> Self {
        Self {
            clock,
            observations: Vec::new(),
        }
    }

    /// Record a value that is safe to check in.
    pub fn record(&mut self, name: &str, value: impl Into<Value>) {
        let at_millis = self.clock.tick();
        self.observations.push(Observation {
            name: name.to_owned(),
            at_millis,
            value: Some(value.into()),
        });
    }

    /// Record that a probe ran, discarding its value because the value is
    /// secret.
    ///
    /// The value is taken rather than omitted so the call site stays one line:
    /// there is no branch in which a caller could accidentally use the plain
    /// `record`. Only the name and the sensitive marker are stored, which is
    /// enough to prove the probe ran without saying what it returned.
    pub fn record_secret(&mut self, name: &str, secret: impl Into<Value>) {
        // The value is converted and left to drop unread at the end of this
        // call. Nothing keeps a copy, so no later code can write it out.
        let _secret: Value = secret.into();
        let at_millis = self.clock.tick();
        self.observations.push(Observation {
            name: name.to_owned(),
            at_millis,
            value: None,
        });
    }

    /// How many observations have been recorded.
    pub fn len(&self) -> usize {
        self.observations.len()
    }

    /// Whether nothing has been recorded yet.
    pub fn is_empty(&self) -> bool {
        self.observations.is_empty()
    }

    /// The recorded observations as a JSON document.
    ///
    /// A `serde_json::Value` cannot hold a NaN or an infinite float, so
    /// serialising one cannot fail and the result is not a `Result`.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&self.to_value()).expect("a serde_json::Value always serialises")
    }

    /// Write the document to `path`, with the trailing newline text files carry.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        let mut document = self.to_json();
        document.push('\n');
        std::fs::write(path, document)
    }

    fn to_value(&self) -> Value {
        let observations: Vec<Value> = self.observations.iter().map(Observation::to_value).collect();
        json!({ "observations": observations })
    }
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}

impl Observation {
    fn to_value(&self) -> Value {
        let mut object = Map::new();
        object.insert("name".to_owned(), Value::String(self.name.clone()));
        object.insert("at".to_owned(), json!(self.at_millis));
        match &self.value {
            Some(value) => {
                object.insert("value".to_owned(), value.clone());
            }
            None => {
                object.insert("sensitive".to_owned(), Value::Bool(true));
            }
        }
        Value::Object(object)
    }
}

impl fmt::Display for Recorder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.to_json())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::START_MILLIS;

    const SECRET: &str = "correct-horse-battery-staple";

    #[test]
    fn values_are_recorded_in_order_with_deterministic_stamps() {
        let mut recorder = Recorder::new();
        recorder.record("windows.list", json!([{ "id": 7 }]));
        recorder.record("monitors.list", "one");
        let document: Value = serde_json::from_str(&recorder.to_json()).expect("recorder output is JSON");
        let observations = document["observations"].as_array().expect("observations array");
        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0]["name"], json!("windows.list"));
        assert_eq!(observations[0]["at"], json!(START_MILLIS + 1));
        assert_eq!(observations[0]["value"], json!([{ "id": 7 }]));
        assert_eq!(observations[1]["at"], json!(START_MILLIS + 2));
        assert_eq!(observations[1]["value"], json!("one"));
    }

    #[test]
    fn a_secret_value_never_reaches_the_output() {
        let mut recorder = Recorder::new();
        recorder.record("prompt.before", "visible");
        recorder.record_secret("prompt.answer", SECRET);
        recorder.record("prompt.after", "visible");

        let document = recorder.to_json();
        assert!(
            !document.contains(SECRET),
            "the secret leaked into the recording:\n{document}"
        );
        let parsed: Value = serde_json::from_str(&document).expect("recorder output is JSON");
        let observations = parsed["observations"].as_array().expect("observations array");
        assert_eq!(observations.len(), 3, "a redacted observation is still recorded");
        assert_eq!(observations[1]["name"], json!("prompt.answer"));
        assert_eq!(observations[1]["sensitive"], json!(true));
        assert!(observations[1].get("value").is_none(), "no value may be stored");
    }

    #[test]
    fn a_secret_inside_a_structure_never_reaches_the_output() {
        let mut recorder = Recorder::new();
        recorder.record_secret("auth.response", json!({ "answer": SECRET, "attempt": 2 }));
        assert!(!recorder.to_json().contains(SECRET));
        assert!(!recorder.to_json().contains("attempt"));
    }

    #[test]
    fn writing_a_recording_ends_with_a_newline() {
        let mut recorder = Recorder::new();
        recorder.record("monitors.list", "one");
        let path = std::env::temp_dir().join(format!("gnoblin-recorder-{}.json", std::process::id()));
        recorder.write(&path).expect("write recording");
        let text = std::fs::read_to_string(&path).expect("read recording");
        assert!(text.ends_with("}\n"), "a text file ends with exactly one newline");
        std::fs::remove_file(&path).expect("remove recording");
    }

    #[test]
    fn an_empty_recorder_still_writes_a_document() {
        let recorder = Recorder::new();
        assert!(recorder.is_empty());
        assert_eq!(recorder.len(), 0);
        let parsed: Value = serde_json::from_str(&recorder.to_json()).expect("recorder output is JSON");
        assert_eq!(parsed["observations"], json!([]));
    }
}
