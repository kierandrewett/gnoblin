//! Tests of this module.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use tempfile::TempDir;

use super::JsReloader;
use crate::error::JsErrorKind;
use crate::generation::{JsConfig, JsGeneration};
use crate::supervisor::{JsFault, JsHealth, JsManualClock, JsStrikePolicy, JsSupervisor};
use crate::watch::ConfigWatcher;

/// The quiet time that ends a burst of saves. A test writes its burst in microseconds, but a loaded machine can
/// stall the writing thread for far longer than a short debounce, and the stalled burst would split in two. The
/// debounce is longer than any stall a test machine is likely to see.
const DEBOUNCE: Duration = Duration::from_secs(1);
const PATIENCE: Duration = Duration::from_secs(10);
/// Long enough that an event already on its way has arrived; it never decides whether a burst was one.
const QUIET: Duration = Duration::from_millis(600);
/// What a stage of an honest module may use. A test asserts what a stage does, never how fast it was.
const HONEST_BUDGET: Duration = Duration::from_secs(5);
/// The budget of a fixture whose module loops for ever, and of the honest module loaded beside it. Short
/// enough that a runaway stage is cheap, long enough that the honest stage cannot trip it. A test of a runaway
/// stage asserts that it was interrupted, never when.
const RUNAWAY_BUDGET: Duration = Duration::from_millis(500);

type Commits = Rc<RefCell<Vec<u64>>>;
type TestCommit = Box<dyn FnMut(&JsGeneration) -> Result<(), String>>;
type TestReloader = JsReloader<TestCommit, JsManualClock>;

/// A reloader over a temp root that records the id of every generation it was asked to commit.
struct Fixture {
    _dir: TempDir,
    root: PathBuf,
    commits: Commits,
    reloader: TestReloader,
}

fn fixture(files: &[(&str, &str)]) -> Fixture {
    fixture_with(files, |_| Ok(()))
}

/// Like [`fixture`], for modules that loop: every stage has the short [`RUNAWAY_BUDGET`].
fn runaway_fixture(files: &[(&str, &str)]) -> Fixture {
    build(files, RUNAWAY_BUDGET, |_| Ok(()))
}

fn fixture_with(files: &[(&str, &str)], accept: impl Fn(u64) -> Result<(), String> + 'static) -> Fixture {
    build(files, HONEST_BUDGET, accept)
}

fn build(files: &[(&str, &str)], budget: Duration, accept: impl Fn(u64) -> Result<(), String> + 'static) -> Fixture {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fs::canonicalize(dir.path()).expect("the temp dir exists");
    for (name, text) in files {
        fs::write(root.join(name), text).expect("the module is written");
    }
    let commits: Commits = Rc::default();
    let log = Rc::clone(&commits);
    let commit: TestCommit = Box::new(move |generation| {
        accept(generation.id())?;
        log.borrow_mut().push(generation.id());
        Ok(())
    });
    let mut config = JsConfig::new(&root);
    config.stage_budget = budget;
    let supervisor = JsSupervisor::new(JsStrikePolicy::default(), JsManualClock::default());
    let reloader = JsReloader::new(config, "init.ts", commit, supervisor);
    Fixture {
        _dir: dir,
        root,
        commits,
        reloader,
    }
}

fn version(reloader: &TestReloader) -> i32 {
    reloader
        .current()
        .expect("a current generation")
        .call_budgeted(Duration::from_secs(5), |ctx| ctx.eval::<i32, _>("version()"))
        .expect("version() is callable")
}

fn write(root: &Path, name: &str, text: &str) {
    fs::write(root.join(name), text).expect("the module is written");
}

const V1: &str = "globalThis.version = (): number => 1;\n";
const V2: &str = "globalThis.version = (): number => 2;\n";

#[test]
fn a_syntax_error_keeps_the_old_generation_and_names_file_line_and_column() {
    let mut fx = fixture(&[("init.ts", V1)]);
    let first = fx.reloader.reload_now();
    assert!(first.ok, "{first:?}");
    assert_eq!(first.generation, Some(1));

    write(&fx.root, "init.ts", "const ok = 1;\nconst broken = ;\n");
    let report = fx.reloader.reload_now();
    assert!(!report.ok);
    assert_eq!(report.generation, Some(1), "the old generation is still current");
    assert_eq!(version(&fx.reloader), 1, "and still callable");
    let [error] = report.errors.as_slice() else {
        panic!("one error expected: {report:?}");
    };
    assert_eq!(error.file, fx.root.join("init.ts"));
    assert_eq!((error.line, error.column), (2, 16));
    assert_eq!(error.kind, Some(JsErrorKind::Ts));
    assert_eq!(*fx.commits.borrow(), vec![1], "the broken stage was not committed");

    write(&fx.root, "init.ts", V2);
    let fixed = fx.reloader.reload_now();
    assert!(fixed.ok, "{fixed:?}");
    assert_eq!(version(&fx.reloader), 2);
    assert!(fixed.generation > Some(1));
    assert_eq!(fx.commits.borrow().len(), 2);
}

#[test]
fn a_throw_in_an_imported_module_reports_that_module() {
    let mut fx = fixture(&[
        ("init.ts", "import './x.ts';\nglobalThis.version = (): number => 1;\n"),
        ("x.ts", "export const a: number = 1;\n"),
    ]);
    assert!(fx.reloader.reload_now().ok);
    write(
        &fx.root,
        "x.ts",
        "const n: number = 1;\n\nthrow new Error('boom ' + n);\n",
    );
    let report = fx.reloader.reload_now();
    assert!(!report.ok);
    let [error] = report.errors.as_slice() else {
        panic!("one error expected: {report:?}");
    };
    assert_eq!(error.file, fx.root.join("x.ts"));
    assert_eq!(error.line, 3);
    assert!(error.message.contains("boom"), "{error}");
    assert_eq!(version(&fx.reloader), 1);
}

#[test]
fn a_rejected_commit_keeps_the_old_generation() {
    let mut fx = fixture_with(&[("init.ts", V1)], |id| {
        if id == 1 {
            Ok(())
        } else {
            Err("duplicate workspace id".to_owned())
        }
    });
    assert!(fx.reloader.reload_now().ok);
    write(&fx.root, "init.ts", V2);
    let report = fx.reloader.reload_now();
    assert!(!report.ok);
    assert_eq!(report.errors[0].kind, None);
    assert!(report.errors[0].message.contains("duplicate workspace id"));
    assert_eq!(report.generation, Some(1));
    assert_eq!(version(&fx.reloader), 1);
}

#[test]
fn a_runaway_config_is_reported_as_a_budget_failure_and_is_not_a_strike() {
    let mut fx = runaway_fixture(&[("init.ts", V1)]);
    assert!(fx.reloader.reload_now().ok);
    write(&fx.root, "init.ts", "while (true) {}\n");
    for _ in 0..5 {
        let report = fx.reloader.reload_now();
        assert!(!report.ok);
        assert_eq!(report.errors[0].kind, Some(JsErrorKind::Budget));
        assert_eq!(
            report.health,
            JsHealth::Healthy,
            "a deliberate reload is never degraded"
        );
    }
    assert_eq!(version(&fx.reloader), 1);
}

#[test]
fn a_save_of_an_imported_module_reloads_through_the_watcher() {
    let mut fx = fixture(&[
        (
            "init.ts",
            "import { n } from './x.ts';\nglobalThis.version = (): number => n;\n",
        ),
        ("x.ts", "export const n: number = 1;\n"),
    ]);
    assert!(fx.reloader.reload_now().ok);
    assert_eq!(
        fx.reloader.watch_set(),
        vec![fx.root.join("init.ts"), fx.root.join("x.ts")]
    );
    let mut watcher = ConfigWatcher::new(&fx.root, DEBOUNCE).expect("a watcher");
    watcher
        .watch_modules(&fx.reloader.watch_set())
        .expect("modules register");

    write(&fx.root, "x.ts", "export const n: number = 7;\n");
    let report = fx.reloader.poll(&mut watcher, PATIENCE).expect("the watcher fired");
    assert!(report.ok, "{report:?}");
    assert_eq!(report.changed, vec![fx.root.join("x.ts")]);
    assert_eq!(version(&fx.reloader), 7);
}

#[test]
fn fifty_rapid_writes_cause_one_reload() {
    let mut fx = fixture(&[("init.ts", V1)]);
    assert!(fx.reloader.reload_now().ok);
    let mut watcher = ConfigWatcher::new(&fx.root, DEBOUNCE).expect("a watcher");
    watcher
        .watch_modules(&fx.reloader.watch_set())
        .expect("modules register");

    for n in 0..50 {
        write(
            &fx.root,
            "init.ts",
            &format!("globalThis.version = (): number => {};\n", 100 + n),
        );
    }
    let report = fx.reloader.poll(&mut watcher, PATIENCE).expect("the watcher fired");
    assert!(report.ok, "{report:?}");
    assert_eq!(version(&fx.reloader), 149, "the last write won");
    assert!(fx.reloader.poll(&mut watcher, QUIET).is_none(), "no second reload");
    assert_eq!(fx.commits.borrow().len(), 2, "the initial load and one reload");
}

#[test]
fn a_broken_new_import_is_watched_so_fixing_it_reloads() {
    let mut fx = fixture(&[("init.ts", V1)]);
    assert!(fx.reloader.reload_now().ok);
    let mut watcher = ConfigWatcher::new(&fx.root, DEBOUNCE).expect("a watcher");
    watcher
        .watch_modules(&fx.reloader.watch_set())
        .expect("modules register");

    fs::create_dir(fx.root.join("lib")).expect("lib");
    write(&fx.root.join("lib"), "y.ts", "export const ok = ;\n");
    write(
        &fx.root,
        "init.ts",
        "import './lib/y.ts';\nglobalThis.version = (): number => 2;\n",
    );
    let broken = fx.reloader.poll(&mut watcher, PATIENCE).expect("init.ts changed");
    assert!(!broken.ok);
    assert_eq!(broken.errors[0].file, fx.root.join("lib/y.ts"));
    assert!(
        watcher.watched_dirs().contains(&fx.root.join("lib")),
        "lib is now watched"
    );

    write(&fx.root.join("lib"), "y.ts", "export const ok = 1;\n");
    let fixed = fx.reloader.poll(&mut watcher, PATIENCE).expect("y.ts changed");
    assert!(fixed.ok, "{fixed:?}");
    assert_eq!(version(&fx.reloader), 2);
}

#[test]
fn recovery_restages_from_disk_after_a_fault() {
    let mut fx = fixture(&[("init.ts", V1)]);
    assert!(fx.reloader.reload_now().ok);
    let report = fx.reloader.recover(JsFault::Budget);
    assert!(report.ok, "{report:?}");
    assert_eq!(
        report.generation,
        Some(2),
        "a fresh generation replaced the suspect one"
    );
    assert_eq!(*fx.commits.borrow(), vec![1, 2]);
    assert_eq!(version(&fx.reloader), 1);
}

#[test]
fn three_interrupted_restages_degrade_and_keep_the_last_good_generation() {
    let mut fx = runaway_fixture(&[("init.ts", V1)]);
    assert!(fx.reloader.reload_now().ok);
    // The file on disk now loops, so every restage is interrupted.
    write(&fx.root, "init.ts", "while (true) {}\n");
    let report = fx.reloader.recover(JsFault::Budget);
    assert!(!report.ok);
    assert!(matches!(report.health, JsHealth::Degraded { .. }), "{report:?}");
    assert_eq!(report.generation, Some(1), "the last good generation is kept");
    assert!(report.errors[0].message.contains("generation 1 stays"), "{report:?}");
    assert_eq!(version(&fx.reloader), 1);
    assert_eq!(*fx.commits.borrow(), vec![1]);

    // Still degraded: another fault is refused at once, without staging.
    let again = fx.reloader.recover(JsFault::Panic);
    assert!(!again.ok);

    // Fixing the file and reloading on purpose clears the state.
    write(&fx.root, "init.ts", V2);
    let fixed = fx.reloader.reload_now();
    assert!(fixed.ok);
    assert_eq!(fixed.health, JsHealth::Healthy);
    assert_eq!(version(&fx.reloader), 2);
}

#[test]
fn degraded_with_no_good_generation_runs_none() {
    let mut fx = runaway_fixture(&[("init.ts", "while (true) {}\n")]);
    let first = fx.reloader.reload_now();
    assert!(!first.ok);
    assert!(fx.reloader.current().is_none());
    let report = fx.reloader.recover(JsFault::Budget);
    assert!(!report.ok);
    assert_eq!(report.generation, None);
    assert!(
        report.errors[0].message.contains("no generation is running"),
        "{report:?}"
    );
    assert!(fx.reloader.current().is_none());
}

#[test]
fn a_restage_that_fails_without_an_interrupt_is_not_retried() {
    let mut fx = fixture(&[("init.ts", V1)]);
    assert!(fx.reloader.reload_now().ok);
    write(&fx.root, "init.ts", "throw new Error('nope');\n");
    let report = fx.reloader.recover(JsFault::Budget);
    assert!(!report.ok);
    assert_eq!(report.errors[0].kind, Some(JsErrorKind::Exception));
    assert_eq!(report.health, JsHealth::Healthy, "one strike only");
    assert_eq!(version(&fx.reloader), 1);
}
