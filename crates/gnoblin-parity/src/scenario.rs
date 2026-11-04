//! The declarative scenario format.
//!
//! A scenario is data, not code: the same file is run against every target, and
//! the file records which behaviour it means to prove without knowing how any
//! target produces it. That is what lets the Mutter reference and the Rust
//! candidate be compared with one harness instead of two test suites
//! (design/rust-compositor.md, "The parity harness").
//!
//! The format is TOML so comments survive, unknown keys are rejected by
//! `deny_unknown_fields`, and a typo in a checked-in scenario fails the run
//! instead of silently skipping a check.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

/// A parity scenario: one target, the ordered stimuli to apply to it, and the
/// observation each stimulus must produce.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    /// Unique name, printed in the report.
    pub name: String,
    /// What the scenario proves, in one sentence, for the reader of a failure.
    #[serde(default)]
    pub description: String,
    /// The target the scenario runs against.
    pub target: TargetSpec,
    /// Why the scenario does not run yet. When present the report says `skip`
    /// and the target is never built or started, so a scenario can be checked in
    /// ahead of the lane that makes it runnable and still show up in the report.
    #[serde(default)]
    pub skip: Option<String>,
    /// Ordered stimuli. Each one also names the observation it produces.
    #[serde(default)]
    pub steps: Vec<Step>,
}

/// One stimulus and the observation it must produce.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// Observation name, unique inside its scenario. A live target treats it as
    /// the probe to run against the session (`windows.list`); the recorded target
    /// treats it as the key to replay from its recording.
    pub id: String,
    /// What the stimulus does, for the reader of the scenario file.
    #[serde(default)]
    pub description: String,
    /// The reference value the target's observation must match after
    /// normalisation. Absent records the observation without comparing it, which
    /// is how a new scenario is explored before its expectation is known.
    ///
    /// The value is a plain JSON shape: numbers, strings, booleans, arrays and
    /// tables. TOML datetimes are not expectations; they are exactly the kind of
    /// volatile value the normaliser removes.
    #[serde(default)]
    pub expect: Option<Value>,
    /// How the `command` target realises the stimulus. A live target ignores
    /// this, so leaving it out keeps a scenario runnable against a session and
    /// out of reach of the `command` target.
    #[serde(default)]
    pub command: Option<CommandStep>,
}

/// A step the `command` target runs as a program.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandStep {
    /// Program to run. Resolved through `PATH`, never through a shell, so a
    /// scenario cannot smuggle in quoting or a pipeline.
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub format: OutputFormat,
}

/// How a program's standard output becomes an observation.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// The observation is the output as text, without its trailing newlines.
    #[default]
    Text,
    /// The observation is the output parsed as JSON, so a recorded expectation
    /// compares structurally instead of character by character.
    Json,
}

/// Which target a scenario runs against.
///
/// Two variants are implemented: `recorded`, which replays a checked-in JSON
/// recording, and `command`, which runs a program per step. The other three name
/// the live targets from the plan and are rejected until their lane lands, so a
/// scenario that asks for one fails loudly rather than passing on a stand-in.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum TargetSpec {
    /// Replay observations from a checked-in JSON recording, relative to the
    /// scenario file.
    Recorded(PathBuf),
    /// Run each step's `command` on this machine and capture its output.
    Command(CommandTargetOptions),
    /// Stock Mutter 51 with the headless backend: the reference for core
    /// protocol behaviour. Arrives with the Mutter target lane.
    Mutter51(LiveOptions),
    /// Gnoblin on the patched Mutter: the reference for Gnoblin's own API.
    /// Arrives with the Mutter target lane.
    MutterGnoblin(LiveOptions),
    /// The Rust candidate with `gnoblin-backend` headless. Arrives with the first
    /// phase where `gnoblin-compositor` runs a session.
    Candidate(LiveOptions),
}

/// Options for the `command` target. It has none yet; the table exists so that a
/// scenario reads `command = {}` rather than a bare string, and so an option can
/// be added without breaking the files.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandTargetOptions {}

/// Options for a live target. It has none yet: a live target's session layout
/// comes from the plan's fixed virtual output layout, not from the scenario.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveOptions {}

impl TargetSpec {
    /// The target name printed in the report.
    pub fn name(&self) -> String {
        match self {
            Self::Recorded(path) => format!("recorded:{}", path.display()),
            Self::Command(_) => "command".to_owned(),
            Self::Mutter51(_) => "mutter51".to_owned(),
            Self::MutterGnoblin(_) => "mutter-gnoblin".to_owned(),
            Self::Candidate(_) => "candidate".to_owned(),
        }
    }

    /// Whether this target needs a live compositor session.
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Mutter51(_) | Self::MutterGnoblin(_) | Self::Candidate(_))
    }
}

impl Scenario {
    /// Read, parse and validate one scenario file.
    pub fn load(path: &Path) -> Result<Self, ScenarioError> {
        let text = std::fs::read_to_string(path).map_err(|source| ScenarioError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&text, path)
    }

    /// Parse and validate a scenario from text. `origin` names the file in an
    /// error message and is the scenario's own path when loaded from disk.
    pub fn parse(text: &str, origin: &Path) -> Result<Self, ScenarioError> {
        let scenario: Self = toml::from_str(text).map_err(|error| ScenarioError::Parse {
            path: origin.to_path_buf(),
            detail: error.to_string(),
        })?;
        scenario.validate().map_err(|detail| ScenarioError::Invalid {
            path: origin.to_path_buf(),
            detail,
        })?;
        Ok(scenario)
    }

    /// Reject a scenario that cannot mean anything: no steps, a duplicated step
    /// id, an empty name or skip reason, or a `command` target whose steps do not
    /// carry commands.
    fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("the scenario has no name".to_owned());
        }
        if self.steps.is_empty() {
            return Err(format!("scenario `{}` declares no steps", self.name));
        }
        if let Some(reason) = &self.skip
            && reason.trim().is_empty()
        {
            return Err(format!("scenario `{}` has an empty skip reason", self.name));
        }
        let mut seen = BTreeSet::new();
        for step in &self.steps {
            if !seen.insert(step.id.as_str()) {
                return Err(format!("scenario `{}` declares step `{}` twice", self.name, step.id));
            }
            if step.id.trim().is_empty() {
                return Err(format!("scenario `{}` has a step with no id", self.name));
            }
            if matches!(self.target, TargetSpec::Command(_)) && step.command.is_none() {
                return Err(format!(
                    "step `{}` declares no command, which the `command` target needs",
                    step.id
                ));
            }
        }
        Ok(())
    }
}

/// Why a scenario file could not be turned into a scenario.
#[derive(Debug)]
pub enum ScenarioError {
    /// The file could not be read.
    Read { path: PathBuf, source: std::io::Error },
    /// The file is not valid TOML, or does not match the scenario shape.
    Parse { path: PathBuf, detail: String },
    /// The file parses but describes a scenario that cannot run.
    Invalid { path: PathBuf, detail: String },
}

impl fmt::Display for ScenarioError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => write!(formatter, "{}: {source}", path.display()),
            Self::Parse { path, detail } => write!(formatter, "{}: {detail}", path.display()),
            Self::Invalid { path, detail } => write!(formatter, "{}: {detail}", path.display()),
        }
    }
}

impl std::error::Error for ScenarioError {}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
name = "toplevel-configure"
target = { command = {} }

[[steps]]
id = "configure.size"
expect = { width = 320 }
command = { program = "printf", args = ["%s", "{\"width\": 320}"], format = "json" }
"#;

    #[test]
    fn a_minimal_scenario_parses() {
        let scenario = Scenario::parse(MINIMAL, Path::new("<memory>")).expect("scenario parses");
        assert_eq!(scenario.name, "toplevel-configure");
        assert_eq!(scenario.steps.len(), 1);
        assert_eq!(scenario.steps[0].id, "configure.size");
        assert_eq!(scenario.steps[0].expect, Some(serde_json::json!({ "width": 320 })));
        assert_eq!(scenario.target.name(), "command");
        assert!(!scenario.target.is_live());
    }

    #[test]
    fn an_unknown_key_is_rejected() {
        let text = MINIMAL.replace("name = ", "nmae = ");
        let error = Scenario::parse(&text, Path::new("<memory>")).expect_err("unknown key fails");
        assert!(
            matches!(error, ScenarioError::Parse { .. }),
            "expected a parse error, got {error}"
        );
        assert!(error.to_string().contains("nmae"), "the error names the key: {error}");
    }

    #[test]
    fn a_duplicated_step_id_is_rejected() {
        let text = format!("{MINIMAL}\n[[steps]]\nid = \"configure.size\"\ncommand = {{ program = \"true\" }}\n");
        let error = Scenario::parse(&text, Path::new("<memory>")).expect_err("duplicate fails");
        assert!(
            error.to_string().contains("declares step `configure.size` twice"),
            "{error}"
        );
    }

    #[test]
    fn a_command_target_needs_a_command_per_step() {
        let text = r#"
name = "no-command"
target = { command = {} }

[[steps]]
id = "windows.list"
"#;
        let error = Scenario::parse(text, Path::new("<memory>")).expect_err("missing command fails");
        assert!(error.to_string().contains("declares no command"), "{error}");
    }

    #[test]
    fn a_scenario_without_steps_is_rejected() {
        let text = r#"
name = "empty"
target = { recorded = "x.json" }
"#;
        let error = Scenario::parse(text, Path::new("<memory>")).expect_err("no steps fails");
        assert!(error.to_string().contains("declares no steps"), "{error}");
    }

    #[test]
    fn the_live_targets_parse_and_are_marked_live() {
        let text = r#"
name = "live"
target = { mutter-gnoblin = {} }
skip = "needs a session"

[[steps]]
id = "windows.list"
"#;
        let scenario = Scenario::parse(text, Path::new("<memory>")).expect("scenario parses");
        assert!(scenario.target.is_live());
        assert_eq!(scenario.target.name(), "mutter-gnoblin");
        assert_eq!(scenario.skip.as_deref(), Some("needs a session"));
    }

    #[test]
    fn a_recorded_target_names_its_recording() {
        let text = r#"
name = "recorded"
target = { recorded = "../fixtures/x.json" }

[[steps]]
id = "windows.list"
"#;
        let scenario = Scenario::parse(text, Path::new("<memory>")).expect("scenario parses");
        assert_eq!(scenario.target.name(), "recorded:../fixtures/x.json");
        assert!(!scenario.target.is_live());
    }
}
