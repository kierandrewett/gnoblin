//! Language dispatch in the loader (`design/modular-framework.md` sections 5.10, 6 and 8.2; M6.1).
//!
//! A script is run by a runtime library that exports `gnoruntime_entry_v1`. The loader picks it by the script's extension, opens it
//! once, and drives its generations through the same stage, commit and abandon protocol as a native shell. The runtime here is the
//! test fixture `gnoblin-fixture-runtime-test`, which runs `.gnotest` scripts whose first line says what it does, so every path of
//! the loader has a script that takes it: dispatch, a reload that commits, a failed start, a panic in a runtime, a generation that
//! cannot activate, no runtime for an extension, a library that is not a runtime, and a native shell still loading as one.
//!
//! The scenarios are the reload driver of the core's test launcher (`gnoblin-fixture-launcher reload MODULE STEP...`): the module is
//! a script here, and every line of the runtime's own is printed as `[runtime-test] ...`, so the tests read the order the loader
//! called the entry points in.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

mod support;
use support::*;

struct Stage {
    dir: PathBuf,
    /// The directory `GNOBLIN_RUNTIME_DIR` names: the test runtime, and a library that is not a runtime.
    runtimes: PathBuf,
}

fn build_stage() -> Result<Stage, String> {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("runtime-stage");
    let _ = fs::remove_dir_all(&base);
    let target = base.join("target-main");
    let dir = base.join("core10");
    cargo_build(
        &target,
        &[
            "-p",
            "gnoblin-core-impl",
            "-p",
            "gnoblin-fixture-launcher",
            "-p",
            "gnoblin-fixture-runtime-test",
            "-p",
            "gnoblin-fixture-shell-min",
            "--features",
            "gnoblin-core-impl/test-driver",
        ],
        &[],
    )?;
    let debug = target.join("debug");
    copy_to(&debug.join("libgnoblin_core_impl.so"), &dir.join(CORE_FILE));
    copy_to(&debug.join(LAUNCHER), &dir.join(LAUNCHER));
    let runtimes = dir.join("runtimes");
    copy_to(
        &debug.join("libruntime_test.so"),
        &runtimes.join("libgnoblin-runtime-gnotest.so"),
    );
    // A library in a runtime directory that exports a shell and no runtime.
    copy_to(
        &debug.join("libshell_min.so"),
        &runtimes.join("libgnoblin-runtime-notruntime.so"),
    );
    copy_to(&debug.join("libshell_min.so"), &dir.join("shells/libshell_min.so"));
    Ok(Stage { dir, runtimes })
}

fn stage() -> &'static Stage {
    static STAGE: OnceLock<Result<Stage, String>> = OnceLock::new();
    match STAGE.get_or_init(build_stage) {
        Ok(stage) => stage,
        Err(message) => panic!("cannot build the fixtures: {message}"),
    }
}

struct Run {
    stdout: String,
    stderr: String,
    code: i32,
}

impl Run {
    fn dump(&self) -> String {
        format!(
            "exit {}\n--- stdout\n{}\n--- stderr\n{}",
            self.code, self.stdout, self.stderr
        )
    }

    /// The position of the first line that contains `needle`, at or after the line `after`.
    fn at(&self, needle: &str, after: usize) -> usize {
        self.stdout
            .lines()
            .enumerate()
            .skip(after)
            .find(|(_, line)| line.contains(needle))
            .map(|(index, _)| index)
            .unwrap_or_else(|| panic!("no line with `{needle}` from line {after}\n{}", self.dump()))
    }

    fn lines_with(&self, needle: &str) -> Vec<&str> {
        self.stdout.lines().filter(|line| line.contains(needle)).collect()
    }
}

/// A script `name` holding `text` in a fresh place, and the scenario run over it with `steps`.
fn scripted(name: &str, text: &str, steps: &[&str]) -> (PathBuf, Run) {
    let stage = stage();
    let script = stage.dir.join("scripts").join(name);
    fs::create_dir_all(script.parent().unwrap()).unwrap();
    fs::write(&script, text).unwrap();
    let run = drive(&script, steps);
    (script, run)
}

fn drive(module: &Path, steps: &[&str]) -> Run {
    let stage = stage();
    let mut all: Vec<PathBuf> = vec![PathBuf::from("reload"), module.to_owned()];
    all.extend(steps.iter().map(PathBuf::from));
    let refs: Vec<&Path> = all.iter().map(PathBuf::as_path).collect();
    let output = scenario_with_env(&stage.dir, &refs, &[("GNOBLIN_RUNTIME_DIR", stage.runtimes.as_path())]);
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code().unwrap_or(-1),
    }
}

/// A file with `text` that a `copy:` step replaces the script with.
fn source(name: &str, text: &str) -> String {
    let path = stage().dir.join("sources").join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, text).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn a_script_is_run_by_the_runtime_its_extension_names_and_ends_in_the_order_a_shell_does() {
    let (script, run) = scripted("dispatch.gnotest", "ok\n", &["pump", "stop"]);
    assert_eq!(run.code, 0, "{}", run.dump());
    // The context says what language it is: no constant names this one, so it is the generic script language (5).
    let start = run.lines_with("start generation=1");
    assert_eq!(start.len(), 1, "{}", run.dump());
    assert!(start[0].contains("language=5"), "{}", run.dump());
    assert!(start[0].contains("reload_count=0"), "{}", run.dump());
    assert!(
        start[0].contains(&format!("path={}", script.display())),
        "{}",
        run.dump()
    );
    assert!(run.stdout.contains("started generation=1"), "{}", run.dump());
    assert!(
        !run.lines_with("turn mode=ok").is_empty(),
        "the runtime's own loop ran\n{}",
        run.dump()
    );
    // Ending: the shutdown hook runs and is released, and then the runtime's shutdown runs. Nothing is left to remove.
    let start = run.at("start generation=1", 0);
    let activate = run.at("activate mode=ok", start);
    let hook = run.at("hook of generation 1", activate);
    let release = run.at("drop hook of generation 1", hook);
    let shutdown = run.at("shutdown mode=ok", release);
    assert!(
        start < activate && activate < hook && hook < release && release < shutdown,
        "{}",
        run.dump()
    );
    assert!(run.stdout.contains("stopped removed=0"), "{}", run.dump());
}

#[test]
fn a_reload_stages_beside_the_serving_generation_commits_and_retires_it_and_maps_nothing_new() {
    let second = source("second.gnotest", "ok\nsecond\n");
    let (_, run) = scripted(
        "reload.gnotest",
        "ok\nfirst\n",
        &["pump", &format!("copy:{second}"), "reload", "reload", "stats"],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
    // The first reload: the new generation starts before the old one ends, and the old one has ended before the new one is
    // activated.
    let staged = run.at("start generation=2", 0);
    assert!(
        run.lines_with("start generation=2")[0].contains("reload_count=1"),
        "{}",
        run.dump()
    );
    let hook = run.at("hook of generation 1", staged);
    let release = run.at("drop hook of generation 1", hook);
    let shutdown = run.at("shutdown mode=ok", release);
    let activate = run.at("activate mode=ok", shutdown);
    assert!(
        staged < hook && hook < release && release < shutdown && shutdown < activate,
        "{}",
        run.dump()
    );
    let reloads = run.lines_with("reload ok");
    assert_eq!(reloads.len(), 2, "{}", run.dump());
    assert!(
        reloads[0].contains("generation=2 previous=1 removed=0"),
        "{}",
        run.dump()
    );
    assert!(
        reloads[1].contains("generation=3 previous=2 removed=0"),
        "{}",
        run.dump()
    );
    // A script maps nothing of its own: however often it reloads, the retire list stays empty.
    assert!(
        reloads[0].contains("retained=0") && reloads[1].contains("retained=0"),
        "{}",
        run.dump()
    );
    assert!(run.stdout.contains("retained=0\n"), "{}", run.dump());
}

#[test]
fn a_start_that_fails_leaves_the_serving_generation_and_says_why_and_a_later_good_script_loads() {
    let bad = source("bad.gnotest", "fail\n");
    let good = source("good.gnotest", "ok\ngood\n");
    let (path, run) = scripted(
        "failing.gnotest",
        "ok\n",
        &[
            "pump",
            &format!("copy:{bad}"),
            "reload",
            "dump",
            &format!("copy:{good}"),
            "reload",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
    let failed = run.lines_with("reload failed");
    assert_eq!(failed.len(), 1, "{}", run.dump());
    assert_eq!(
        failed[0],
        format!(
            "reload failed reload of {} failed, still running generation 1: {}: init failed (E_ERR): the script says fail",
            path.display(),
            path.display()
        ),
        "{}",
        run.dump()
    );
    // The serving generation was never ended, and nothing of the failed one is left to release.
    assert!(run.lines_with("shutdown mode=ok").len() == 1, "{}", run.dump());
    let after_failure = run.at("reload failed", 0);
    assert!(
        run.at("shutdown mode=ok", after_failure) > run.at("start generation=3", after_failure),
        "the generation that served ended only when the next good one took over\n{}",
        run.dump()
    );
    // The failed generation used number 2; the good one is 3 and replaces 1.
    assert!(
        run.lines_with("reload ok")[0].contains("generation=3 previous=1"),
        "{}",
        run.dump()
    );
}

#[test]
fn a_panic_in_a_runtime_is_contained_and_reported_and_the_serving_generation_stays() {
    let bad = source("panic.gnotest", "panic\n");
    let (_, run) = scripted(
        "panics.gnotest",
        "ok\n",
        &["pump", &format!("copy:{bad}"), "reload", "pump", "stats"],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
    let failed = run.lines_with("reload failed");
    assert_eq!(failed.len(), 1, "{}", run.dump());
    assert!(
        failed[0].contains("still running generation 1") && failed[0].contains("init failed (E_PANIC)"),
        "{}",
        run.dump()
    );
    assert!(failed[0].contains("boom in the runtime's start"), "{}", run.dump());
    // The first generation keeps getting its turns after the failure.
    let failure = run.at("reload failed", 0);
    run.at("turn mode=ok", failure);
}

#[test]
fn a_generation_that_cannot_activate_is_abandoned_without_ending_the_one_that_serves() {
    let stubborn = source("stubborn.gnotest", "noactivate\n");
    let good = source("again.gnotest", "ok\nagain\n");
    let (_, run) = scripted(
        "activation.gnotest",
        "ok\n",
        &[
            "pump",
            &format!("copy:{stubborn}"),
            "reload",
            "pump",
            &format!("copy:{good}"),
            "reload",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.dump());
    let failed = run.lines_with("reload failed");
    assert_eq!(failed.len(), 1, "{}", run.dump());
    assert!(
        failed[0].contains("the staged generation could not take over: cannot activate this generation"),
        "{}",
        run.dump()
    );
    assert!(failed[0].contains("still running generation 1"), "{}", run.dump());
    // The loader told the runtime to throw the staged generation away, after trying to activate it.
    let staged = run.at("start generation=2", 0);
    let activate = run.at("activate mode=noactivate", staged);
    run.at("abandon mode=noactivate", activate);
    run.at("reload ok generation=3 previous=1", 0);
}

#[test]
fn no_runtime_for_an_extension_says_what_was_looked_for_where_and_what_is_known() {
    let (path, run) = scripted("shell.unknownlang", "x\n", &[]);
    assert_eq!(run.code, 1, "{}", run.dump());
    let line = run.lines_with("start_failed")[0].to_owned();
    let runtimes = stage().runtimes.display().to_string();
    assert!(line.contains(&path.display().to_string()), "{line}");
    assert!(
        line.contains("no runtime for `unknownlang` scripts: `libgnoblin-runtime-unknownlang.so` was not found in"),
        "{line}"
    );
    assert!(line.contains(&runtimes), "{line}");
    assert!(line.contains("Known extensions: .so .py .js .mjs .ts .mts"), "{line}");
    assert!(line.contains("language = \""), "{line}");
}

#[test]
fn a_known_extension_with_no_runtime_installed_names_the_runtime_library_it_needs() {
    let (_, run) = scripted("shell.ts", "export {};\n", &[]);
    assert_eq!(run.code, 1, "{}", run.dump());
    let line = run.lines_with("start_failed")[0].to_owned();
    assert!(
        line.contains("no runtime for `js` scripts: `libgnoblin-runtime-js.so` was not found in"),
        "{line}"
    );
}

#[test]
fn a_library_in_a_runtime_directory_that_is_not_a_runtime_is_refused_by_name() {
    let (_, run) = scripted("shell.notruntime", "x\n", &[]);
    assert_eq!(run.code, 1, "{}", run.dump());
    let line = run.lines_with("start_failed")[0].to_owned();
    assert!(
        line.contains(
            "libgnoblin-runtime-notruntime.so: not a gnoblin runtime: the library exports no `gnoruntime_entry_v1`"
        ),
        "{line}"
    );
}

#[test]
fn a_file_that_is_neither_a_library_nor_has_an_extension_cannot_be_run() {
    let (_, run) = scripted("shell", "x\n", &[]);
    assert_eq!(run.code, 1, "{}", run.dump());
    let line = run.lines_with("start_failed")[0].to_owned();
    assert!(line.contains("cannot tell how to run this file"), "{line}");
    assert!(line.contains("Known extensions: .so .py .js .mjs .ts .mts"), "{line}");
}

#[test]
fn a_script_that_is_not_there_is_a_missing_file_not_a_missing_runtime() {
    let stage = stage();
    let run = drive(&stage.dir.join("scripts/not-there.gnotest"), &[]);
    assert_eq!(run.code, 1, "{}", run.dump());
    let line = run.lines_with("start_failed")[0].to_owned();
    assert!(line.contains("the shell library does not exist"), "{line}");
}

#[test]
fn a_native_shell_is_still_a_native_shell_with_runtimes_installed() {
    let stage = stage();
    let run = drive(&stage.dir.join("shells/libshell_min.so"), &["stop"]);
    assert_eq!(run.code, 0, "{}", run.dump());
    assert!(run.stdout.contains("started generation=1"), "{}", run.dump());
    assert!(!run.stdout.contains("[runtime-test]"), "{}", run.dump());
}
