//! The readable account of how a target differed from the scenario.
//!
//! A parity failure is read by whoever is fixing the candidate, often on a CI log
//! rather than in a debugger. A boolean or a serialised blob of two whole traces
//! makes them diff two hundred lines by eye. This module flattens both values to
//! dotted leaf paths, so the report names the one field that differs and prints
//! only the two values that disagree.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::Value;

/// Every way one scenario's observations differed from its expectation.
#[derive(Debug, Default)]
pub struct Diff {
    lines: Vec<DiffLine>,
}

/// One difference, in a form that reads as a sentence.
#[derive(Debug)]
pub enum DiffLine {
    /// The target produced nothing under a name the scenario asks for.
    Missing { observation: String },
    /// The target produced an observation the scenario does not ask for.
    Unexpected { observation: String },
    /// A leaf value differs. `path` is empty when the whole observation differs
    /// as a single value.
    Changed {
        observation: String,
        path: String,
        expected: Option<String>,
        actual: Option<String>,
    },
    /// The target itself failed, before any comparison was possible.
    Failed { reason: String },
}

impl Diff {
    /// An empty diff, which is a passing scenario.
    pub fn new() -> Self {
        Self::default()
    }

    /// A diff that holds one failure and no comparison.
    pub fn failed(reason: impl Into<String>) -> Self {
        let mut diff = Self::new();
        diff.push(DiffLine::Failed { reason: reason.into() });
        diff
    }

    /// Whether nothing differed.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// How many differences were reported.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// The differences, in report order.
    pub fn lines(&self) -> &[DiffLine] {
        &self.lines
    }

    pub fn push(&mut self, line: DiffLine) {
        self.lines.push(line);
    }

    /// Compare one already-normalised observation and record every leaf that
    /// differs.
    ///
    /// Paths are visited in sorted order so the report is reproducible; `[10]`
    /// sorting before `[2]` is an accepted cost of that, since a report is read to
    /// find the first difference, not to replay the document.
    pub fn compare_observation(&mut self, observation: &str, expected: &Value, actual: &Value) {
        let mut expected_leaves = BTreeMap::new();
        flatten("", expected, &mut expected_leaves);
        let mut actual_leaves = BTreeMap::new();
        flatten("", actual, &mut actual_leaves);

        let mut paths: Vec<&String> = expected_leaves.keys().collect();
        for path in actual_leaves.keys() {
            if !expected_leaves.contains_key(path) {
                paths.push(path);
            }
        }

        for path in paths {
            let expected = expected_leaves.get(path);
            let actual = actual_leaves.get(path);
            if expected == actual {
                continue;
            }
            self.lines.push(DiffLine::Changed {
                observation: observation.to_owned(),
                path: path.clone(),
                expected: expected.cloned(),
                actual: actual.cloned(),
            });
        }
    }
}

impl fmt::Display for Diff {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for line in &self.lines {
            writeln!(formatter, "{line}")?;
        }
        Ok(())
    }
}

impl fmt::Display for DiffLine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { observation } => {
                write!(formatter, "{observation}: the target produced no observation")
            }
            Self::Unexpected { observation } => {
                write!(
                    formatter,
                    "{observation}: the target produced an observation the scenario does not ask for"
                )
            }
            Self::Changed {
                observation,
                path,
                expected,
                actual,
            } => {
                let location = if path.is_empty() {
                    observation.clone()
                } else {
                    format!("{observation} {path}")
                };
                write!(
                    formatter,
                    "{location}: expected {}, got {}",
                    render_present(expected.as_deref()),
                    render_present(actual.as_deref())
                )
            }
            Self::Failed { reason } => write!(formatter, "{reason}"),
        }
    }
}

/// A leaf value, quoted when it is a string so a difference in trailing space is
/// visible.
fn render_leaf(value: &Value) -> String {
    match value {
        Value::String(text) => format!("\"{text}\""),
        other => other.to_string(),
    }
}

fn render_present(value: Option<&str>) -> String {
    value.map_or_else(|| "<absent>".to_owned(), ToOwned::to_owned)
}

/// Flatten a value to `path -> rendered leaf`.
///
/// An empty object or array becomes one leaf of its own, so "the target produced
/// an empty list" stays distinguishable from "the target produced nothing".
fn flatten(prefix: &str, value: &Value, leaves: &mut BTreeMap<String, String>) {
    match value {
        Value::Object(object) if !object.is_empty() => {
            for (key, value) in object {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(&path, value, leaves);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            for (index, value) in items.iter().enumerate() {
                flatten(&format!("{prefix}[{index}]"), value, leaves);
            }
        }
        other => {
            leaves.insert(prefix.to_owned(), render_leaf(other));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn diff_for(observation: &str, expected: Value, actual: Value) -> Diff {
        let mut diff = Diff::new();
        diff.compare_observation(observation, &expected, &actual);
        diff
    }

    #[test]
    fn an_identical_observation_produces_no_difference() {
        let diff = diff_for(
            "windows.list",
            json!([{ "id": "a", "title": "t" }]),
            json!([{ "id": "a", "title": "t" }]),
        );
        assert!(diff.is_empty(), "unexpected differences:\n{diff}");
    }

    #[test]
    fn a_differing_leaf_names_the_field_and_both_values() {
        let diff = diff_for(
            "windows.list",
            json!([{ "id": "<id:1>", "title": "term" }]),
            json!([{ "id": "<id:1>", "title": "files" }]),
        );
        assert_eq!(
            diff.to_string(),
            "windows.list [0].title: expected \"term\", got \"files\"\n"
        );
    }

    #[test]
    fn a_missing_leaf_is_reported_as_absent() {
        let diff = diff_for(
            "windows.list",
            json!([{ "title": "term", "state": "normal" }]),
            json!([{ "title": "term" }]),
        );
        assert_eq!(
            diff.to_string(),
            "windows.list [0].state: expected \"normal\", got <absent>\n"
        );
    }

    #[test]
    fn an_extra_leaf_is_reported_as_absent_on_the_expected_side() {
        let diff = diff_for(
            "windows.list",
            json!([{ "title": "term" }]),
            json!([{ "title": "term", "state": "normal" }]),
        );
        assert_eq!(
            diff.to_string(),
            "windows.list [0].state: expected <absent>, got \"normal\"\n"
        );
    }

    #[test]
    fn a_scalar_observation_differs_as_one_line() {
        let diff = diff_for("ready", json!("before"), json!("after"));
        assert_eq!(diff.to_string(), "ready: expected \"before\", got \"after\"\n");
    }

    #[test]
    fn an_empty_container_stays_distinguishable_from_no_value() {
        let diff = diff_for("windows.list", json!([]), json!({}));
        assert_eq!(diff.to_string(), "windows.list: expected [], got {}\n");
    }

    #[test]
    fn a_target_failure_is_a_diff_of_its_own() {
        let diff = Diff::failed("target `mutter51` is not implemented");
        assert_eq!(diff.len(), 1);
        assert_eq!(diff.to_string(), "target `mutter51` is not implemented\n");
    }

    #[test]
    fn missing_and_unexpected_observations_read_as_sentences() {
        let mut diff = Diff::new();
        diff.push(DiffLine::Missing {
            observation: "windows.list".to_owned(),
        });
        diff.push(DiffLine::Unexpected {
            observation: "layers.list".to_owned(),
        });
        assert_eq!(
            diff.to_string(),
            "windows.list: the target produced no observation\nlayers.list: the target produced an observation the scenario does not ask for\n"
        );
    }
}
