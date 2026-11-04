//! The scenarios checked into `tests/parity/scenarios`, run as CI runs them.
//!
//! This test is the gate for the runner itself: it asserts that the three checked
//! in scenarios report one pass, one failure with a readable difference, and one
//! skip with its reason. A change that makes the runner silently pass everything,
//! or that loses the normaliser, fails here before it can hide a candidate defect.

use std::path::{Path, PathBuf};

use gnoblin_parity::{Report, ScenarioReport, Verdict, run_directory};

fn scenario_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/parity/scenarios")
}

fn run() -> Report {
    run_directory(&scenario_dir()).expect("the checked-in scenario directory loads")
}

fn find<'report>(report: &'report Report, name: &str) -> &'report ScenarioReport {
    report
        .scenarios()
        .iter()
        .find(|scenario| scenario.name() == name)
        .unwrap_or_else(|| panic!("scenario `{name}` is missing from the report:\n{report}"))
}

#[test]
fn the_checked_in_scenarios_report_one_pass_one_failure_and_one_skip() {
    let report = run();
    assert_eq!(report.scenarios().len(), 3, "unexpected scenario count:\n{report}");
    assert_eq!(
        (report.passed(), report.failed(), report.skipped()),
        (1, 1, 1),
        "unexpected verdicts:\n{report}"
    );
    assert!(!report.is_ok(), "a reported difference must fail the run");
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
fn the_recorded_scenario_reports_the_stacking_difference() {
    let report = run();
    let scenario = find(&report, "layer-surface-stacking");
    assert_eq!(
        scenario.target(),
        "recorded:../fixtures/layer-surface-stacking/candidate.json"
    );
    let Verdict::Fail { diff } = scenario.verdict() else {
        panic!("expected a failure, got:\n{report}\n{:?}", scenario.verdict());
    };
    let text = diff.to_string();
    assert!(
        text.contains("layers.list [0].layer: expected \"overlay\", got \"top\""),
        "the difference must name the field and both values:\n{text}"
    );
    assert!(
        text.contains("layers.list [0].namespace: expected \"osd\", got \"bar\""),
        "the difference must name the surface that moved:\n{text}"
    );
}

#[test]
fn the_recorded_scenarios_volatile_values_are_normalised_away() {
    let report = run();
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
fn the_live_target_scenario_skips_with_its_reason() {
    let report = run();
    let scenario = find(&report, "nested-session-ipc");
    assert_eq!(scenario.target(), "mutter-gnoblin");
    match scenario.verdict() {
        Verdict::Skipped { reason } => {
            assert!(reason.contains("lane P2"), "the reason names what is missing: {reason}");
        }
        other => panic!("expected a skip, got:\n{report}\n{other:?}"),
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
