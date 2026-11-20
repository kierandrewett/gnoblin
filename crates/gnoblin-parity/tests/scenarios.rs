//! The scenarios checked into `tests/parity/scenarios`, run as CI runs them.
//!
//! This test is the gate for the runner itself: it asserts that the checked-in
//! scenarios report the expected verdicts, that the Rust candidate passes its own
//! recorded expectations, and that a difference is named in words. A change that
//! makes the runner silently pass everything, or that loses the normaliser, fails
//! here before it can hide a candidate defect.

use std::path::{Path, PathBuf};

use gnoblin_parity::{Report, RunEnvironment, ScenarioReport, Verdict, run_directory, run_directory_in};

fn scenario_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/parity/scenarios")
}

/// The scenarios that can never pass, which exist to prove the runner names a difference in words.
fn known_failing_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/parity/known-failing")
}

fn run() -> Report {
    run_directory_in(&scenario_dir(), &RunEnvironment::default()).expect("the checked-in scenario directory loads")
}

fn run_known_failing() -> Report {
    run_directory_in(&known_failing_dir(), &RunEnvironment::default()).expect("the known-failing directory loads")
}

fn find<'report>(report: &'report Report, name: &str) -> &'report ScenarioReport {
    report
        .scenarios()
        .iter()
        .find(|scenario| scenario.name() == name)
        .unwrap_or_else(|| panic!("scenario `{name}` is missing from the report:\n{report}"))
}

#[test]
fn the_checked_in_scenarios_report_their_verdicts() {
    let report = run();
    // Without a Mutter launcher in the environment the `mutter51` and `mutter-gnoblin` reference rows are skipped; the
    // candidate rows pass or diverge on record. The directory is a gate: no failure.
    assert_eq!(report.scenarios().len(), 19, "unexpected scenario count:\n{report}");
    assert_eq!(
        (report.passed(), report.divergent(), report.failed(), report.skipped()),
        (5, 3, 0, 11),
        "unexpected verdicts:\n{report}"
    );
    assert!(
        report.is_ok(),
        "the scenario directory is a gate and must not fail:\n{report}"
    );

    let refused = run_known_failing();
    assert_eq!(
        (refused.passed(), refused.failed()),
        (0, 1),
        "unexpected verdicts:\n{refused}"
    );
    assert!(!refused.is_ok(), "a reported difference must fail the run");
}

#[test]
fn the_command_target_scenario_passes_on_its_expectations() {
    let report = run();
    let scenario = find(&report, "toplevel-configure");
    assert_eq!(scenario.target(), "command");
    match scenario.verdict() {
        Verdict::Pass { compared } => assert_eq!(*compared, 2, "both steps carry an expectation"),
        other => panic!("expected a pass, got:\n{report}\n{other:?}"),
    }
}

#[test]
fn the_candidate_scenarios_match_the_legacy_session_or_record_why_not() {
    let report = run();
    // The window lifecycle reads the same on the candidate and on the legacy session; the other two have a reviewed
    // divergence, and what the candidate answers is still compared, step by step.
    match row(&report, "candidate-window-lifecycle", "candidate").verdict() {
        Verdict::Pass { compared } => assert_eq!(*compared, 2, "every step of the lifecycle carries an expectation"),
        other => panic!("the lifecycle must pass against the candidate, got:\n{report}\n{other:?}"),
    }
    for (name, compared, reason) in [
        (
            "candidate-workspace-list",
            1,
            "workspace.list: the candidate answers in the IPC 0.2.0 object model",
        ),
        (
            "candidate-kill-unknown-window",
            1,
            "window.kill.unknown: the refusal text is the candidate's",
        ),
    ] {
        match row(&report, name, "candidate").verdict() {
            Verdict::Divergent {
                compared: count,
                reasons,
            } => {
                assert_eq!(*count, compared, "every step of `{name}` carries an expectation");
                assert_eq!(reasons.len(), 1, "one step of `{name}` diverges:\n{report}");
                assert!(reasons[0].starts_with(reason), "{reasons:?}");
            }
            other => panic!("`{name}` must diverge on record against the candidate, got:\n{report}\n{other:?}"),
        }
    }
}

#[test]
fn the_eis_input_scenario_passes_against_the_rust_compositor() {
    let report = run();
    match row(&report, "input-eis", "candidate").verdict() {
        Verdict::Pass { compared } => assert_eq!(*compared, 4, "key, motion, button and scroll are all compared"),
        other => panic!("`input-eis` must pass against the candidate, got:\n{report}\n{other:?}"),
    }
}

/// The report row of `name` for `target`.
fn row<'report>(report: &'report Report, name: &str, target: &str) -> &'report ScenarioReport {
    report
        .scenarios()
        .iter()
        .find(|scenario| scenario.name() == name && scenario.target() == target)
        .unwrap_or_else(|| panic!("no `{name}` row for `{target}`:\n{report}"))
}

#[test]
fn the_popup_scenarios_pass_against_the_rust_compositor() {
    let report = run();
    for (name, compared) in [("popup-placement", 5), ("popup-lifecycle", 3)] {
        match row(&report, name, "candidate").verdict() {
            Verdict::Pass { compared: count } => assert_eq!(*count, compared, "every step of `{name}` is compared"),
            other => panic!("`{name}` must pass against the candidate, got:\n{report}\n{other:?}"),
        }
    }
}

#[test]
fn the_candidate_diverges_from_mutter_on_focus_for_a_recorded_reason() {
    let report = run();
    match row(&report, "focus-toplevels", "candidate").verdict() {
        Verdict::Divergent { compared, reasons } => {
            assert_eq!(*compared, 3);
            assert_eq!(reasons.len(), 1, "one step diverges:\n{report}");
            assert!(
                reasons[0].starts_with("focus.second-toplevel: focus_new_windows defaults to prevent"),
                "{reasons:?}"
            );
        }
        other => panic!("expected a recorded divergence, got:\n{report}\n{other:?}"),
    }
}

#[test]
fn the_mutter_reference_rows_skip_and_name_the_launcher_variable_when_none_is_configured() {
    let report = run();
    for name in ["focus-toplevels", "popup-placement", "popup-lifecycle", "input-eis"] {
        match row(&report, name, "mutter51").verdict() {
            Verdict::Skipped { reason } => assert!(reason.contains("GNOBLIN_PARITY_MUTTER51"), "{reason}"),
            other => panic!("`{name}` must skip its reference without a launcher, got:\n{report}\n{other:?}"),
        }
    }
    for name in [
        "focus-toplevels",
        "popup-placement",
        "popup-lifecycle",
        "input-eis",
        "candidate-workspace-list",
        "candidate-window-lifecycle",
        "candidate-kill-unknown-window",
    ] {
        match row(&report, name, "mutter-gnoblin").verdict() {
            Verdict::Skipped { reason } => assert!(reason.contains("GNOBLIN_PARITY_MUTTER_GNOBLIN"), "{reason}"),
            other => panic!("`{name}` must skip its legacy reference without a launcher, got:\n{report}\n{other:?}"),
        }
    }
}

#[test]
fn the_recorded_scenarios_volatile_values_are_normalised_away() {
    let report = run_known_failing();
    let scenario = find(&report, "layer-surface-stacking");
    let Verdict::Fail { diff } = scenario.verdict() else {
        panic!("expected a failure, got:\n{report}");
    };
    let text = diff.to_string();
    for volatile in ["serial", "timestamp", "client_pid", "<id:2>", "<id:1>"] {
        assert!(
            !text.contains(volatile),
            "`{volatile}` cannot decide a verdict and must not be reported:\n{text}"
        );
    }
}

#[test]
fn one_broken_scenario_file_does_not_hide_the_others() {
    // A directory with one scenario that does not parse still reports the
    // scenarios beside it, with the parse error on the broken one.
    let directory = std::env::temp_dir().join(format!("gnoblin-parity-files-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("create scenario directory");
    let keep = directory.join("keep.toml");
    std::fs::copy(scenario_dir().join("toplevel-configure.toml"), &keep).expect("copy a good scenario");
    std::fs::write(directory.join("broken.toml"), "name = \"broken\ntarget = ").expect("write a broken scenario");

    let report = run_directory(&directory).expect("the directory loads");
    std::fs::remove_dir_all(&directory).expect("remove the scenario directory");

    assert_eq!(report.scenarios().len(), 2, "unexpected report:\n{report}");
    assert_eq!(report.passed(), 1);
    assert_eq!(report.failed(), 1);
    assert!(
        report.scenarios()[0].name() == "broken",
        "files are reported in name order"
    );
}
