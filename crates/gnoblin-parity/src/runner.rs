//! Running a directory of scenarios and comparing each target against its
//! expectation.
//!
//! The runner is deliberately dull: load, build the target the scenario names,
//! collect observations, normalise, diff. Everything that decides a verdict is
//! visible in the report, and a scenario that cannot be run is a failure or a
//! declared skip rather than a silent omission.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::diff::{Diff, DiffLine};
use crate::normaliser::Normaliser;
use crate::report::{Report, ScenarioReport, Verdict};
use crate::scenario::{Scenario, TargetSpec};
use crate::target::{Observations, RunEnvironment, TargetError};

/// Run every `*.toml` scenario in `directory`, in file-name order, with the Mutter reference the process
/// environment configures.
pub fn run_directory(directory: &Path) -> Result<Report, RunError> {
    run_directory_in(directory, &RunEnvironment::from_process())
}

/// Run every `*.toml` scenario in `directory`, in file-name order, with the given environment.
pub fn run_directory_in(directory: &Path, environment: &RunEnvironment) -> Result<Report, RunError> {
    let mut report = Report::new();
    for path in scenario_files(directory)? {
        for scenario in run_scenario_file(&path, environment) {
            report.push(scenario);
        }
    }
    Ok(report)
}

/// The scenario files in a directory, sorted so two runs report in one order.
pub fn scenario_files(directory: &Path) -> Result<Vec<PathBuf>, RunError> {
    let entries = std::fs::read_dir(directory).map_err(|source| RunError::Read {
        path: directory.to_path_buf(),
        source,
    })?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| RunError::Read {
            path: directory.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension == "toml") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

/// Run one scenario file: one report row for its target and one for each reference. A file that cannot be loaded
/// becomes a failing scenario report rather than an error out of the whole run, so one broken file does not hide the
/// result of the others.
pub fn run_scenario_file(path: &Path, environment: &RunEnvironment) -> Vec<ScenarioReport> {
    let label = path.file_stem().map_or_else(
        || path.display().to_string(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    let scenario = match Scenario::load(path) {
        Ok(scenario) => scenario,
        Err(error) => {
            return vec![ScenarioReport::new(
                label,
                "unknown",
                Verdict::Fail {
                    diff: Diff::failed(error.to_string()),
                },
            )];
        }
    };
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let mut reports = vec![run_scenario(&scenario, base, environment)];
    if scenario.skip.is_none() {
        for reference in &scenario.references {
            reports.push(run_against(&scenario, reference, base, environment));
        }
    }
    reports
}

/// Run an already-loaded scenario against its own target. `base` is the directory a relative recording path is
/// resolved against.
pub fn run_scenario(scenario: &Scenario, base: &Path, environment: &RunEnvironment) -> ScenarioReport {
    if let Some(reason) = &scenario.skip {
        return ScenarioReport::new(
            &scenario.name,
            scenario.target.name(),
            Verdict::Skipped { reason: reason.clone() },
        );
    }
    run_against(scenario, &scenario.target, base, environment)
}

/// Run the scenario's steps against `spec` and compare them with the expectations.
fn run_against(scenario: &Scenario, spec: &TargetSpec, base: &Path, environment: &RunEnvironment) -> ScenarioReport {
    let target = spec.name();
    let failed = |detail: String| {
        ScenarioReport::new(
            &scenario.name,
            &target,
            Verdict::Fail {
                diff: Diff::failed(detail),
            },
        )
    };
    let unavailable = |reason: String| ScenarioReport::new(&scenario.name, &target, Verdict::Skipped { reason });

    let target_impl = match spec.build_target(base, environment) {
        Ok(target) => target,
        Err(TargetError::Unavailable { reason, .. }) => return unavailable(reason),
        Err(error) => return failed(error.to_string()),
    };
    let observations = match target_impl.observe(scenario) {
        Ok(observations) => observations,
        Err(error) => return failed(error.to_string()),
    };

    let (compared, reasons, diff) = compare(scenario, &target, &observations);
    let verdict = if !diff.is_empty() {
        Verdict::Fail { diff }
    } else if reasons.is_empty() {
        Verdict::Pass { compared }
    } else {
        Verdict::Divergent { compared, reasons }
    };
    ScenarioReport::new(&scenario.name, &target, verdict)
}

/// Compare every expected observation, in step order, and report observations the
/// scenario did not ask for. A step that records a divergence for `target` is compared against the divergence's
/// expectation, and its reason is returned beside the count so the verdict can say `divergent`.
fn compare(scenario: &Scenario, target: &str, observations: &Observations) -> (usize, Vec<String>, Diff) {
    let mut diff = Diff::new();
    let mut reasons = Vec::new();
    let mut compared = 0;
    for step in &scenario.steps {
        let Some(actual) = observations.get(&step.id) else {
            diff.push(DiffLine::Missing {
                observation: step.id.clone(),
            });
            continue;
        };
        let Some(reference) = &step.expect else {
            // Record-only step: the observation is collected but there is no
            // reference to compare it with yet.
            continue;
        };
        let expected = match step.divergence.get(target) {
            Some(divergence) => {
                reasons.push(format!("{}: {}", step.id, divergence.reason));
                &divergence.expect
            }
            None => reference,
        };
        // A fresh normaliser per observation and per side keeps the id labels
        // independent of how many other steps the scenario compares.
        let normalised_expected = Normaliser::new().normalise(expected);
        let normalised_actual = Normaliser::new().normalise(actual);
        compared += 1;
        diff.compare_observation(&step.id, &normalised_expected, &normalised_actual);
    }

    let asked: BTreeSet<&str> = scenario.steps.iter().map(|step| step.id.as_str()).collect();
    for name in observations.keys() {
        if !asked.contains(name.as_str()) {
            diff.push(DiffLine::Unexpected {
                observation: name.clone(),
            });
        }
    }
    (compared, reasons, diff)
}

/// Why a run could not start at all. A problem with one scenario is reported
/// inside the [`Report`] instead.
#[derive(Debug)]
pub enum RunError {
    Read { path: PathBuf, source: std::io::Error },
}

impl fmt::Display for RunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => write!(formatter, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for RunError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::{RecordedTarget, Target};

    fn scenario_text(target: &str, expectation: &str) -> String {
        format!(
            r#"
name = "replay"
target = {{ {target} }}

[[steps]]
id = "windows.list"
expect = {expectation}
"#
        )
    }

    #[test]
    fn a_recording_that_agrees_with_the_expectation_passes() {
        // The recording and the expectation differ in ids and serials only,
        // which the normaliser removes.
        let json = r#"{ "observations": { "windows.list": [{ "id": 91, "serial": 5, "title": "term" }] } }"#;
        let target = RecordedTarget::from_json("recorded:test.json", json).expect("recording parses");
        let scenario = Scenario::parse(
            &scenario_text("recorded = \"test.json\"", "[{ id = 4, title = \"term\" }]"),
            Path::new("<memory>"),
        )
        .expect("scenario parses");
        let observations = target.observe(&scenario).expect("observations");
        let (compared, _, diff) = compare(&scenario, "recorded:test.json", &observations);
        assert_eq!(compared, 1);
        assert!(diff.is_empty(), "unexpected differences:\n{diff}");
    }

    #[test]
    fn a_different_stacking_order_is_reported() {
        let json = r#"{
            "observations": {
                "windows.list": [
                    { "id": 12, "title": "top" },
                    { "id": 13, "title": "bottom" }
                ]
            }
        }"#;
        let target = RecordedTarget::from_json("recorded:test.json", json).expect("recording parses");
        let scenario = Scenario::parse(
            &scenario_text(
                "recorded = \"test.json\"",
                "[{ id = 1, title = \"bottom\" }, { id = 2, title = \"top\" }]",
            ),
            Path::new("<memory>"),
        )
        .expect("scenario parses");
        let observations = target.observe(&scenario).expect("observations");
        let (_, _, diff) = compare(&scenario, "recorded:test.json", &observations);
        assert_eq!(
            diff.to_string(),
            "windows.list [0].title: expected \"bottom\", got \"top\"\nwindows.list [1].title: expected \"top\", got \"bottom\"\n"
        );
    }

    #[test]
    fn an_observation_without_an_expectation_is_not_counted() {
        let json = r#"{ "observations": { "windows.list": [], "layers.list": [] } }"#;
        let target = RecordedTarget::from_json("recorded:test.json", json).expect("recording parses");
        let scenario = Scenario::parse(
            r#"
name = "record-only"
target = { recorded = "test.json" }

[[steps]]
id = "windows.list"

[[steps]]
id = "layers.list"
"#,
            Path::new("<memory>"),
        )
        .expect("scenario parses");
        let observations = target.observe(&scenario).expect("observations");
        let (compared, _, diff) = compare(&scenario, "recorded:test.json", &observations);
        assert_eq!(compared, 0);
        assert!(diff.is_empty(), "unexpected differences:\n{diff}");
    }

    #[test]
    fn an_observation_the_scenario_did_not_ask_for_is_reported() {
        let mut observations = Observations::new();
        observations.insert("windows.list".to_owned(), serde_json::json!([]));
        observations.insert("layers.list".to_owned(), serde_json::json!([]));
        let scenario = Scenario::parse(
            r#"
name = "unexpected"
target = { recorded = "test.json" }

[[steps]]
id = "windows.list"
"#,
            Path::new("<memory>"),
        )
        .expect("scenario parses");
        let (_, _, diff) = compare(&scenario, "recorded:test.json", &observations);
        assert_eq!(
            diff.to_string(),
            "layers.list: the target produced an observation the scenario does not ask for\n"
        );
    }

    #[test]
    fn a_skipped_scenario_never_builds_its_target() {
        let scenario = Scenario::parse(
            r#"
name = "skipped"
target = { mutter51 = {} }
skip = "needs a live headless Mutter 51 session"

[[steps]]
id = "windows.list"
"#,
            Path::new("<memory>"),
        )
        .expect("scenario parses");
        let report = run_scenario(&scenario, Path::new("."), &RunEnvironment::default());
        match report.verdict() {
            Verdict::Skipped { reason } => assert!(reason.contains("Mutter 51")),
            other => panic!("expected a skip, got {other:?}"),
        }
    }

    #[test]
    fn a_target_with_no_implementation_fails_loudly() {
        let scenario = Scenario::parse(
            r#"
name = "not-implemented"
target = { mutter-gnoblin = {} }

[[steps]]
id = "windows.list"
"#,
            Path::new("<memory>"),
        )
        .expect("scenario parses");
        let report = run_scenario(&scenario, Path::new("."), &RunEnvironment::default());
        match report.verdict() {
            Verdict::Fail { diff } => {
                assert!(
                    diff.to_string().contains("target `mutter-gnoblin` is not implemented"),
                    "{diff}"
                );
            }
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn a_reference_without_a_launcher_is_a_skip_row_beside_the_target_row() {
        let directory = std::env::temp_dir().join(format!("gnoblin-parity-reference-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("create the scenario directory");
        std::fs::write(
            directory.join("both.toml"),
            r#"
name = "both"
target = { recorded = "both.json" }
references = [{ mutter51 = {} }]

[[steps]]
id = "windows.list"
expect = []
"#,
        )
        .expect("write the scenario");
        std::fs::write(
            directory.join("both.json"),
            r#"{ "observations": { "windows.list": [] } }"#,
        )
        .expect("write the recording");
        let report = run_directory_in(&directory, &RunEnvironment::default()).expect("the directory loads");
        std::fs::remove_dir_all(&directory).expect("remove the scenario directory");

        let rows: Vec<(&str, &str)> = report
            .scenarios()
            .iter()
            .map(|row| (row.name(), row.target()))
            .collect();
        assert_eq!(rows, [("both", "recorded:both.json"), ("both", "mutter51")], "{report}");
        assert_eq!(
            (report.passed(), report.failed(), report.skipped()),
            (1, 0, 1),
            "{report}"
        );
        let Verdict::Skipped { reason } = report.scenarios()[1].verdict() else {
            panic!("the reference must be skipped:\n{report}");
        };
        assert!(reason.contains("GNOBLIN_PARITY_MUTTER51"), "{reason}");
    }

    fn diverging_scenario() -> Scenario {
        Scenario::parse(
            r#"
name = "diverging"
target = { candidate = {} }

[[steps]]
id = "focus.second-toplevel"
expect = { b_activated = true }

[steps.divergence.candidate]
expect = { b_activated = false }
reason = "a new window does not take focus from the focused one"
"#,
            Path::new("<memory>"),
        )
        .expect("scenario parses")
    }

    #[test]
    fn a_recorded_divergence_is_compared_for_its_target_only() {
        let scenario = diverging_scenario();
        let mut observations = Observations::new();

        observations.insert(
            "focus.second-toplevel".to_owned(),
            serde_json::json!({ "b_activated": false }),
        );
        let (compared, reasons, diff) = compare(&scenario, "candidate", &observations);
        assert_eq!(compared, 1);
        assert!(diff.is_empty(), "the target does what its divergence records:\n{diff}");
        assert_eq!(
            reasons,
            ["focus.second-toplevel: a new window does not take focus from the focused one"]
        );

        let (_, reasons, diff) = compare(&scenario, "mutter51", &observations);
        assert!(reasons.is_empty(), "another target has no divergence");
        assert_eq!(
            diff.to_string(),
            "focus.second-toplevel b_activated: expected true, got false\n",
            "another target is held to the reference"
        );
    }

    #[test]
    fn a_target_that_stops_diverging_fails_against_its_divergence() {
        let scenario = diverging_scenario();
        let mut observations = Observations::new();
        observations.insert(
            "focus.second-toplevel".to_owned(),
            serde_json::json!({ "b_activated": true }),
        );
        let (_, _, diff) = compare(&scenario, "candidate", &observations);
        assert_eq!(
            diff.to_string(),
            "focus.second-toplevel b_activated: expected false, got true\n"
        );
    }

    #[test]
    fn a_missing_scenario_directory_is_a_run_error() {
        let error = run_directory(Path::new("tests/parity/does-not-exist")).expect_err("missing directory fails");
        assert!(error.to_string().contains("does-not-exist"), "{error}");
    }
}
