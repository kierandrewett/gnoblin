//! What a run says about each scenario.
//!
//! The report is the artifact of a run: CI stores it, and a person reads it to
//! decide whether the candidate is done. It therefore has three states and no
//! more. A scenario either produced every expected observation, differed in a way
//! the diff spells out, or did not run and said why. Silence is never a pass: a
//! scenario that could not be run is a `skip` with a reason on disk, not a line
//! that is missing.

use std::fmt;

use crate::diff::Diff;

/// The result of a whole run.
#[derive(Debug, Default)]
pub struct Report {
    scenarios: Vec<ScenarioReport>,
}

/// The result of one scenario.
#[derive(Debug)]
pub struct ScenarioReport {
    name: String,
    target: String,
    verdict: Verdict,
}

/// What happened to one scenario.
#[derive(Debug)]
pub enum Verdict {
    /// Every compared observation matched.
    Pass {
        /// How many observations were compared (steps without an expectation are
        /// recorded but not counted).
        compared: usize,
    },
    /// At least one observation differed, or the target failed.
    Fail { diff: Diff },
    /// The scenario declares a reason not to run it yet.
    Skipped { reason: String },
}

impl Report {
    /// An empty report, with no scenarios recorded yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one scenario's outcome.
    pub fn push(&mut self, scenario: ScenarioReport) {
        self.scenarios.push(scenario);
    }

    /// The per-scenario results, in run order.
    pub fn scenarios(&self) -> &[ScenarioReport] {
        &self.scenarios
    }

    /// Number of scenarios whose verdict is a pass.
    pub fn passed(&self) -> usize {
        self.count(|verdict| matches!(verdict, Verdict::Pass { .. }))
    }

    /// Number of scenarios whose verdict is a failure.
    pub fn failed(&self) -> usize {
        self.count(|verdict| matches!(verdict, Verdict::Fail { .. }))
    }

    /// Number of scenarios that were skipped.
    pub fn skipped(&self) -> usize {
        self.count(|verdict| matches!(verdict, Verdict::Skipped { .. }))
    }

    /// Whether the run found no difference. A skip does not fail the run: the
    /// scenario carries its own reason, and holding CI red for a lane that has not
    /// landed would hide the failures that matter.
    pub fn is_ok(&self) -> bool {
        self.failed() == 0
    }

    fn count(&self, matches: impl Fn(&Verdict) -> bool) -> usize {
        self.scenarios.iter().filter(|report| matches(&report.verdict)).count()
    }
}

impl ScenarioReport {
    /// Build one scenario's result. `name` and `target` are stored as owned strings.
    pub fn new(name: impl Into<String>, target: impl Into<String>, verdict: Verdict) -> Self {
        Self {
            name: name.into(),
            target: target.into(),
            verdict,
        }
    }

    /// The scenario's name, as printed in the report.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The target the scenario ran against.
    pub fn target(&self) -> &str {
        &self.target
    }

    /// The outcome of the scenario.
    pub fn verdict(&self) -> &Verdict {
        &self.verdict
    }
}

impl Verdict {
    /// The word the report prints.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pass { .. } => "pass",
            Self::Fail { .. } => "fail",
            Self::Skipped { .. } => "skip",
        }
    }
}

impl fmt::Display for Report {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            formatter,
            "[parity] {} scenarios: {} pass, {} fail, {} skip",
            self.scenarios.len(),
            self.passed(),
            self.failed(),
            self.skipped()
        )?;
        for report in &self.scenarios {
            let label = report.verdict.label();
            let name = &report.name;
            let target = &report.target;
            write!(formatter, "  {label:<4} {name} ({target})")?;
            match &report.verdict {
                Verdict::Pass { compared } => writeln!(formatter, ", {compared} observation(s) matched")?,
                Verdict::Fail { diff } => {
                    writeln!(formatter)?;
                    for line in diff.to_string().lines() {
                        writeln!(formatter, "       {line}")?;
                    }
                }
                Verdict::Skipped { reason } => writeln!(formatter, ": {reason}")?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report_with(verdicts: Vec<Verdict>) -> Report {
        let mut report = Report::new();
        for (index, verdict) in verdicts.into_iter().enumerate() {
            report.push(ScenarioReport::new(format!("scenario-{index}"), "command", verdict));
        }
        report
    }

    #[test]
    fn an_empty_report_prints_a_zero_count_and_is_ok() {
        let report = Report::new();
        assert_eq!(report.to_string(), "[parity] 0 scenarios: 0 pass, 0 fail, 0 skip\n");
        assert!(report.is_ok());
    }

    #[test]
    fn the_counts_add_up_and_a_failure_fails_the_run() {
        let report = report_with(vec![
            Verdict::Pass { compared: 2 },
            Verdict::Fail {
                diff: Diff::failed("target `mutter51` is not implemented"),
            },
            Verdict::Skipped {
                reason: "needs a session".to_owned(),
            },
        ]);
        assert_eq!((report.passed(), report.failed(), report.skipped()), (1, 1, 1));
        assert!(!report.is_ok());
    }

    #[test]
    fn a_skip_alone_does_not_fail_the_run() {
        let report = report_with(vec![Verdict::Skipped {
            reason: "needs a live headless Mutter 51 session".to_owned(),
        }]);
        assert!(report.is_ok());
    }

    #[test]
    fn a_pass_names_the_observation_count() {
        let report = report_with(vec![Verdict::Pass { compared: 3 }]);
        assert_eq!(
            report.to_string(),
            "[parity] 1 scenarios: 1 pass, 0 fail, 0 skip\n  pass scenario-0 (command), 3 observation(s) matched\n"
        );
    }

    #[test]
    fn a_failure_indents_its_diff_under_the_scenario() {
        let report = report_with(vec![Verdict::Fail {
            diff: Diff::failed("step `broken`: `printf` failed with exit status: 3"),
        }]);
        assert_eq!(
            report.to_string(),
            "[parity] 1 scenarios: 0 pass, 1 fail, 0 skip\n  fail scenario-0 (command)\n       step `broken`: `printf` failed with exit status: 3\n"
        );
    }

    #[test]
    fn a_skip_prints_its_reason() {
        let report = report_with(vec![Verdict::Skipped {
            reason: "needs a live session".to_owned(),
        }]);
        assert_eq!(
            report.to_string(),
            "[parity] 1 scenarios: 0 pass, 0 fail, 1 skip\n  skip scenario-0 (command): needs a live session\n"
        );
    }
}
