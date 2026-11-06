//! Targets: the ways a scenario obtains its observations.
//!
//! A target is "a way to start a headless session with a fixed virtual output
//! layout and to stop it" (design/rust-compositor.md). The Rust candidate is live
//! ([`crate::candidate::CandidateTarget`]). Stock Mutter 51 and Gnoblin on the
//! patched Mutter need a Mutter session this crate does not start, so
//! [`TargetSpec::build_target`] rejects them with the lane that will bring them,
//! and a scenario that asks for one is reported as failing rather than passing on
//! a stand-in.
//!
//! Two targets are real today and make the runner testable end to end without a
//! compositor:
//!
//! - [`RecordedTarget`] replays observations from a checked-in JSON recording.
//!   This is how a Mutter recording is kept in the tree and compared against a
//!   candidate run without starting Mutter in CI.
//! - [`CommandTarget`] runs a program per step and captures its output. This is
//!   how a self-contained check (a codec, a config loader, a client against a
//!   real session) is expressed as the same kind of scenario.

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::Path;
use std::process::Command;

use serde::Deserialize;
use serde_json::Value;

use crate::candidate::CandidateTarget;
use crate::scenario::{CommandStep, OutputFormat, Scenario, TargetSpec};

/// One observation per step, keyed by step id.
///
/// Ids that a target produces but the scenario does not ask for are dropped by
/// the target, so one recording can serve several scenarios that share a probe.
pub type Observations = BTreeMap<String, Value>;

/// A way to obtain the observations a scenario asks for.
///
/// Implementations must be usable without a session: a live target starts one
/// inside `observe` and stops it before returning, so the runner never has to
/// know about sessions.
pub trait Target {
    /// The target name printed in the report.
    fn name(&self) -> &str;

    /// Run the scenario's steps and return one observation per step, keyed by
    /// step id.
    fn observe(&self, scenario: &Scenario) -> Result<Observations, TargetError>;
}

/// Replay observations from a checked-in JSON recording.
///
/// The recording was produced by a live target and checked in, so a reference
/// observation is reviewable in the same diff as the scenario that uses it.
#[derive(Debug)]
pub struct RecordedTarget {
    name: String,
    producer: String,
    observations: Observations,
}

impl RecordedTarget {
    /// Load a recording from disk. The report names it by its path, so a failure
    /// points at the file to fix.
    pub fn load(path: &Path) -> Result<Self, TargetError> {
        let source = path.display().to_string();
        let text = std::fs::read_to_string(path).map_err(|error| TargetError::Recording {
            source: source.clone(),
            detail: error.to_string(),
        })?;
        Self::from_json(format!("recorded:{source}"), &text)
    }

    /// Which live target produced the recording, as the file declares it. The
    /// report names the recording by path; this is for a reader deciding whether
    /// a checked-in reference is still the one they think it is.
    pub fn producer(&self) -> &str {
        &self.producer
    }

    /// Replay a recording held in memory, for a test of the runner itself.
    pub fn from_json(name: impl Into<String>, json: &str) -> Result<Self, TargetError> {
        let name = name.into();
        let recording: Recording = serde_json::from_str(json).map_err(|error| TargetError::Recording {
            source: name.clone(),
            detail: error.to_string(),
        })?;
        Ok(Self {
            name,
            producer: recording.producer,
            observations: recording.observations,
        })
    }
}

impl Target for RecordedTarget {
    fn name(&self) -> &str {
        &self.name
    }

    fn observe(&self, scenario: &Scenario) -> Result<Observations, TargetError> {
        let mut observations = Observations::new();
        for step in &scenario.steps {
            let value = self
                .observations
                .get(&step.id)
                .ok_or_else(|| TargetError::MissingObservation {
                    step: step.id.clone(),
                    target: self.name.clone(),
                })?;
            observations.insert(step.id.clone(), value.clone());
        }
        Ok(observations)
    }
}

/// The documents this crate reads from disk.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Recording {
    /// Which target produced the recording. Not compared: the same recording is
    /// replayed against whatever the scenario asks for.
    #[serde(default, rename = "target")]
    producer: String,
    /// Observations by step id.
    observations: Observations,
}

/// Run each step's program and capture its standard output.
#[derive(Debug)]
pub struct CommandTarget {
    name: String,
}

impl CommandTarget {
    /// The target itself needs no configuration, so it is a unit.
    pub fn new() -> Self {
        Self {
            name: "command".to_owned(),
        }
    }
}

impl Default for CommandTarget {
    fn default() -> Self {
        Self::new()
    }
}

impl Target for CommandTarget {
    fn name(&self) -> &str {
        &self.name
    }

    fn observe(&self, scenario: &Scenario) -> Result<Observations, TargetError> {
        let mut observations = Observations::new();
        for step in &scenario.steps {
            let command = step
                .command
                .as_ref()
                .ok_or_else(|| TargetError::NoCommand { step: step.id.clone() })?;
            let value = run_step(&step.id, command)?;
            observations.insert(step.id.clone(), value);
        }
        Ok(observations)
    }
}

/// Run one step's program and read its output as the step's observation.
fn run_step(step: &str, command: &CommandStep) -> Result<Value, TargetError> {
    let output = Command::new(&command.program)
        .args(&command.args)
        .output()
        .map_err(|error| TargetError::Spawn {
            step: step.to_owned(),
            program: command.program.clone(),
            source: error,
        })?;
    if !output.status.success() {
        return Err(TargetError::Status {
            step: step.to_owned(),
            program: command.program.clone(),
            status: output.status.to_string(),
            stderr: trim_output(&String::from_utf8_lossy(&output.stderr)).to_owned(),
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let text = trim_output(&stdout);
    match command.format {
        OutputFormat::Text => Ok(Value::String(text.to_owned())),
        OutputFormat::Json => serde_json::from_str(text).map_err(|error| TargetError::NotJson {
            step: step.to_owned(),
            detail: error.to_string(),
        }),
    }
}

/// Strip the newlines a program ends its output with, and nothing else: leading
/// space and inner blank lines are part of what a text observation reports.
fn trim_output(text: &str) -> &str {
    text.trim_end_matches(['\n', '\r'])
}

impl TargetSpec {
    /// Build the target a scenario named.
    ///
    /// `base` is the scenario's own directory, which a recording path is
    /// relative to so a scenario and its fixture move together.
    pub fn build_target(&self, base: &Path) -> Result<Box<dyn Target>, TargetError> {
        match self {
            Self::Recorded(path) => {
                let path = if path.is_absolute() {
                    path.clone()
                } else {
                    base.join(path)
                };
                Ok(Box::new(RecordedTarget::load(&path)?))
            }
            Self::Command(_) => Ok(Box::new(CommandTarget::new())),
            Self::Mutter51(_) | Self::MutterGnoblin(_) => Err(TargetError::NotImplemented {
                target: self.name(),
                hint: "a reference run needs a Mutter session with the headless backend, which this crate does not start yet (the Mutter target lane)"
                    .to_owned(),
            }),
            Self::Candidate(_) => Ok(Box::new(CandidateTarget::new())),
        }
    }
}

/// Why a target could not produce the observations a scenario asks for.
#[derive(Debug)]
pub enum TargetError {
    /// A recording could not be read or does not match the recording shape.
    Recording { source: String, detail: String },
    /// The recording has no observation for a step.
    MissingObservation { step: String, target: String },
    /// A step has no `command`, so the command target cannot run it.
    NoCommand { step: String },
    /// The program could not be started.
    Spawn {
        step: String,
        program: String,
        source: io::Error,
    },
    /// The program ran and exited unsuccessfully.
    Status {
        step: String,
        program: String,
        status: String,
        stderr: String,
    },
    /// The program's output is not JSON though the step asked for JSON.
    NotJson { step: String, detail: String },
    /// A live target has no implementation yet.
    NotImplemented { target: String, hint: String },
    /// A live target failed while it was running the scenario's steps.
    Live { target: String, detail: String },
}

impl fmt::Display for TargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Recording { source, detail } => write!(formatter, "recording {source}: {detail}"),
            Self::MissingObservation { step, target } => {
                write!(formatter, "{target} has no observation for step `{step}`")
            }
            Self::NoCommand { step } => write!(formatter, "step `{step}` declares no command"),
            Self::Spawn { step, program, source } => {
                write!(formatter, "step `{step}`: cannot start `{program}`: {source}")
            }
            Self::Status {
                step,
                program,
                status,
                stderr,
            } => {
                if stderr.is_empty() {
                    write!(formatter, "step `{step}`: `{program}` failed with {status}")
                } else {
                    write!(formatter, "step `{step}`: `{program}` failed with {status}: {stderr}")
                }
            }
            Self::NotJson { step, detail } => {
                write!(formatter, "step `{step}`: output is not JSON: {detail}")
            }
            Self::NotImplemented { target, hint } => {
                write!(formatter, "target `{target}` is not implemented: {hint}")
            }
            Self::Live { target, detail } => write!(formatter, "target `{target}` failed: {detail}"),
        }
    }
}

impl std::error::Error for TargetError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::{LiveOptions, Scenario};

    fn command_scenario(text: &str) -> Scenario {
        Scenario::parse(text, Path::new("<memory>")).expect("scenario parses")
    }

    #[test]
    fn a_recorded_target_replays_the_observations_the_scenario_asks_for() {
        let recording = r#"
        {
            "target": "gnoblin-on-mutter",
            "observations": {
                "windows.list": [{ "id": 3, "title": "term" }],
                "focus.trace": ["term"],
                "unused.probe": 1
            }
        }
        "#;
        let target = RecordedTarget::from_json("recorded:test.json", recording).expect("recording parses");
        let scenario = Scenario::parse(
            r#"
name = "replay"
target = { recorded = "test.json" }

[[steps]]
id = "windows.list"

[[steps]]
id = "focus.trace"
"#,
            Path::new("<memory>"),
        )
        .expect("scenario parses");
        let observations = target.observe(&scenario).expect("observations");
        assert_eq!(observations.len(), 2, "only the requested probes are returned");
        assert_eq!(
            observations["windows.list"],
            serde_json::json!([{ "id": 3, "title": "term" }])
        );
        assert_eq!(observations["focus.trace"], serde_json::json!(["term"]));
        assert_eq!(target.name(), "recorded:test.json");
    }

    #[test]
    fn a_recorded_target_fails_when_the_recording_lacks_a_step() {
        let target =
            RecordedTarget::from_json("recorded:test.json", r#"{ "observations": {} }"#).expect("recording parses");
        let scenario = command_scenario(
            r#"
name = "replay"
target = { recorded = "test.json" }

[[steps]]
id = "windows.list"
"#,
        );
        let error = target.observe(&scenario).expect_err("missing observation fails");
        assert!(
            error.to_string().contains("no observation for step `windows.list`"),
            "{error}"
        );
    }

    #[test]
    fn a_recording_with_an_unknown_key_is_rejected() {
        let error =
            RecordedTarget::from_json("recorded:test.json", r#"{ "observation": {} }"#).expect_err("unknown key fails");
        assert!(error.to_string().contains("recording recorded:test.json"), "{error}");
    }

    #[test]
    fn the_command_target_captures_text_output_without_its_newline() {
        let scenario = command_scenario(
            r#"
name = "text"
target = { command = {} }

[[steps]]
id = "monitors.list"
command = { program = "printf", args = ["%s\n", "  eDP-1\n"] }
"#,
        );
        let observations = CommandTarget::new().observe(&scenario).expect("observations");
        assert_eq!(observations["monitors.list"], serde_json::json!("  eDP-1"));
    }

    #[test]
    fn the_command_target_parses_json_output() {
        let scenario = command_scenario(
            r#"
name = "json"
target = { command = {} }

[[steps]]
id = "configure.size"
command = { program = "printf", args = ["%s", "{\"width\": 320}"], format = "json" }
"#,
        );
        let observations = CommandTarget::new().observe(&scenario).expect("observations");
        assert_eq!(observations["configure.size"], serde_json::json!({ "width": 320 }));
    }

    #[test]
    fn a_failing_program_reports_its_status_and_stderr() {
        let scenario = command_scenario(
            r#"
name = "failing"
target = { command = {} }

[[steps]]
id = "broken"
command = { program = "sh", args = ["-c", "echo out of order >&2; exit 3"] }
"#,
        );
        let error = CommandTarget::new().observe(&scenario).expect_err("a bad exit fails");
        let message = error.to_string();
        assert!(message.contains("step `broken`"), "{message}");
        assert!(message.contains("exit status: 3"), "{message}");
        assert!(message.contains("out of order"), "{message}");
    }

    #[test]
    fn output_that_is_not_json_is_reported_against_the_step() {
        let scenario = command_scenario(
            r#"
name = "not-json"
target = { command = {} }

[[steps]]
id = "broken"
command = { program = "printf", args = ["%s", "not json"], format = "json" }
"#,
        );
        let error = CommandTarget::new().observe(&scenario).expect_err("invalid JSON fails");
        assert!(
            error.to_string().contains("step `broken`: output is not JSON"),
            "{error}"
        );
    }

    #[test]
    fn a_live_target_is_rejected_with_the_lane_that_brings_it() {
        let spec = TargetSpec::Mutter51(LiveOptions {});
        let Err(error) = spec.build_target(Path::new(".")) else {
            panic!("a live target must not build until its lane lands");
        };
        assert!(
            error.to_string().contains("target `mutter51` is not implemented"),
            "{error}"
        );
    }
}
