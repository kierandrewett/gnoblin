//! Hot reload: stage the config from disk, hand it to the compositor, then swap it in.
//!
//! [`JsReloader`] owns the [`JsHost`] and the entry module. A reload stages a fresh generation, passes
//! it to the caller's `commit` closure (the compositor commit of design/js-research/quickjs-embedding.md,
//! section 5, step 5), and only when that accepts does it make the generation current and drop the old
//! one. Any failure on the way, a parse error, a throw, a runaway loop, a rejected commit, leaves the
//! running generation exactly as it was and comes back as a [`ReloadReport`] with the failing file,
//! line and column in the author's `.ts` coordinates.
//!
//! Three things start a reload: a change the [`ConfigWatcher`] saw ([`JsReloader::poll`]), the user
//! ([`JsReloader::reload_now`], for `gnoblinctl reload`), and a fault in the running generation
//! ([`JsReloader::recover`], which consults the [`JsSupervisor`]). The first two are deliberate retries
//! and clear the supervisor's strikes.

use std::path::PathBuf;
use std::time::Duration;

use crate::error::{JsError, JsErrorKind};
use crate::generation::{JsConfig, JsGeneration};
use crate::host::{JsHost, JsPrepare};
use crate::supervisor::{JsClock, JsFault, JsFaultAction, JsHealth, JsSupervisor, JsSystemClock};
use crate::watch::{ConfigWatcher, ReloadRequest};

/// One failure in a reload, at the position the author wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsDiagnostic {
    /// The file the failure is in. The entry module when the engine reported no position.
    pub file: PathBuf,
    /// The 1-based line, or 0 when the engine reported no position.
    pub line: u32,
    /// The 1-based column, or 0 when the engine reported no position.
    pub column: u32,
    /// What went wrong.
    pub message: String,
    /// The engine's kind of failure, or `None` when the commit closure rejected the generation.
    pub kind: Option<JsErrorKind>,
}

impl std::fmt::Display for JsDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            self.file.display(),
            self.line,
            self.column,
            self.message
        )
    }
}

/// What a reload did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReloadReport {
    /// Whether a new generation is now current.
    pub ok: bool,
    /// The id of the generation that is current after the reload, `None` when there is none.
    pub generation: Option<u64>,
    /// Why the reload failed. Empty when `ok`.
    pub errors: Vec<JsDiagnostic>,
    /// Problems that did not fail the reload, such as a directory the watcher could not watch.
    pub warnings: Vec<String>,
    /// The module files whose change triggered the reload; empty for a manual or recovery reload.
    pub changed: Vec<PathBuf>,
    /// Whether the supervisor still allows restaging after a fault.
    pub health: JsHealth,
}

/// Why a stage did not become the current generation.
#[derive(Debug)]
enum StageFailure {
    /// The config failed to load or evaluate.
    Js(JsError),
    /// It evaluated, and the commit closure refused it.
    Rejected(String),
}

impl StageFailure {
    /// The supervisor fault this failure is, if any: only an interrupt.
    fn fault(&self) -> Option<JsFault> {
        match self {
            Self::Js(error) => JsFault::from_error(error),
            Self::Rejected(_) => None,
        }
    }
}

/// Stages, commits and recovers the shell generation.
///
/// `commit` runs on a staged generation that evaluated cleanly. It returns `Ok` once the compositor
/// accepted the generation's transaction, or the reason it refused, in which case nothing changes.
pub struct JsReloader<F, C = JsSystemClock> {
    host: JsHost,
    entry: PathBuf,
    entry_path: PathBuf,
    commit: F,
    supervisor: JsSupervisor<C>,
    /// Files named by the last failed stage. A failing module is in no generation's graph, and the
    /// edit that fixes it must still be seen.
    failed_files: Vec<PathBuf>,
}

impl<F, C> std::fmt::Debug for JsReloader<F, C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsReloader")
            .field("entry", &self.entry)
            .field("host", &self.host)
            .finish_non_exhaustive()
    }
}

impl<F, C> JsReloader<F, C>
where
    F: FnMut(&JsGeneration) -> Result<(), String>,
    C: JsClock,
{
    /// A reloader with no generation yet. `entry` is the config's entry module, relative to the config
    /// root or absolute. Call [`reload_now`](Self::reload_now) to load it the first time.
    pub fn new(config: JsConfig, entry: impl Into<PathBuf>, commit: F, supervisor: JsSupervisor<C>) -> Self {
        let entry = entry.into();
        let entry_path = if entry.is_absolute() {
            entry.clone()
        } else {
            config.root.join(&entry)
        };
        Self {
            host: JsHost::new(config),
            entry,
            entry_path,
            commit,
            supervisor,
            failed_files: Vec::new(),
        }
    }

    /// Runs `prepare` on every generation this reloader stages, before the entry module is evaluated (see
    /// [`JsHost::set_prepare`]).
    pub fn set_prepare(&mut self, prepare: JsPrepare) {
        self.host.set_prepare(prepare);
    }

    /// The generation that is running, if any.
    pub fn current(&self) -> Option<&JsGeneration> {
        self.host.current()
    }

    /// Whether the supervisor still allows restaging after a fault.
    pub fn health(&self) -> JsHealth {
        self.supervisor.health()
    }

    /// The files a watcher should observe: the module graph of the current generation and any file the
    /// last failed stage named.
    pub fn watch_set(&self) -> Vec<PathBuf> {
        let mut files = self.host.current().map(JsGeneration::module_paths).unwrap_or_default();
        files.extend(self.failed_files.iter().cloned());
        files.sort();
        files.dedup();
        files
    }

    /// Reloads because the user asked: `gnoblinctl reload`, or the first load at startup.
    pub fn reload_now(&mut self) -> ReloadReport {
        self.reload(Vec::new())
    }

    /// Reloads because `request` came from the watcher.
    pub fn reload_for(&mut self, request: &ReloadRequest) -> ReloadReport {
        self.reload(request.paths.clone())
    }

    /// Waits up to `wait` for the watcher to report a change, reloads, and re-arms the watcher from the
    /// new module graph. `None` when nothing changed in that time.
    pub fn poll(&mut self, watcher: &mut ConfigWatcher, wait: Duration) -> Option<ReloadReport> {
        let request = watcher.next_request(wait)?;
        let mut report = self.reload_for(&request);
        if let Err(error) = watcher.watch_modules(&self.watch_set()) {
            report.warnings.push(error.to_string());
        }
        Some(report)
    }

    /// Reloads after the running generation was interrupted or panicked.
    ///
    /// The supervisor counts the fault. While it allows a retry, a fresh generation is staged from disk
    /// and replaces the suspect one; a restage that is itself interrupted is another strike. When the
    /// strikes run out the suspect generation is left running as the last good one, if there is one,
    /// and the report says so. A restage that fails any other way is not retried: the files on disk
    /// would fail the same way.
    pub fn recover(&mut self, fault: JsFault) -> ReloadReport {
        let mut fault = fault;
        loop {
            match self.supervisor.record_fault(fault) {
                JsFaultAction::GiveUp => return self.gave_up(),
                JsFaultAction::Restage => match self.stage_and_commit() {
                    Ok(()) => return self.report(Vec::new(), Vec::new()),
                    Err(failure) => match failure.fault() {
                        Some(next) => fault = next,
                        None => return self.report(vec![self.diagnostic(&failure)], Vec::new()),
                    },
                },
            }
        }
    }

    fn reload(&mut self, changed: Vec<PathBuf>) -> ReloadReport {
        self.supervisor.user_reload();
        let errors = match self.stage_and_commit() {
            Ok(()) => Vec::new(),
            Err(failure) => vec![self.diagnostic(&failure)],
        };
        self.report(errors, changed)
    }

    /// Stages from disk and, when it evaluates and the commit closure accepts, makes it current.
    fn stage_and_commit(&mut self) -> Result<(), StageFailure> {
        let generation = match self.host.stage(&self.entry) {
            Ok(generation) => generation,
            Err(error) => {
                self.failed_files = error.location.iter().map(|at| at.file.clone()).collect();
                return Err(StageFailure::Js(error));
            }
        };
        if let Err(reason) = (self.commit)(&generation) {
            return Err(StageFailure::Rejected(reason));
        }
        self.failed_files.clear();
        self.host.commit(generation);
        Ok(())
    }

    fn diagnostic(&self, failure: &StageFailure) -> JsDiagnostic {
        let error = match failure {
            StageFailure::Js(error) => error,
            StageFailure::Rejected(reason) => {
                return JsDiagnostic {
                    file: self.entry_path.clone(),
                    line: 0,
                    column: 0,
                    message: format!("the compositor rejected the generation: {reason}"),
                    kind: None,
                };
            }
        };
        match &error.location {
            Some(at) => JsDiagnostic {
                file: at.file.clone(),
                line: at.line,
                column: at.column,
                message: error.message.clone(),
                kind: Some(error.kind),
            },
            None => JsDiagnostic {
                file: self.entry_path.clone(),
                line: 0,
                column: 0,
                message: error.message.clone(),
                kind: Some(error.kind),
            },
        }
    }

    fn gave_up(&self) -> ReloadReport {
        let kept = match self.host.current() {
            Some(generation) => format!("generation {} stays as the last good one", generation.id()),
            None => "no generation is running".to_owned(),
        };
        let diagnostic = JsDiagnostic {
            file: self.entry_path.clone(),
            line: 0,
            column: 0,
            message: format!(
                "the config was interrupted too many times; not restaging until it is edited, \
                 `gnoblinctl reload` runs, or the cool-down passes: {kept}"
            ),
            kind: Some(JsErrorKind::Budget),
        };
        self.report(vec![diagnostic], Vec::new())
    }

    fn report(&self, errors: Vec<JsDiagnostic>, changed: Vec<PathBuf>) -> ReloadReport {
        ReloadReport {
            ok: errors.is_empty(),
            generation: self.host.current().map(JsGeneration::id),
            errors,
            warnings: Vec::new(),
            changed,
            health: self.supervisor.health(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::time::Duration;

    use tempfile::TempDir;

    use super::JsReloader;
    use crate::error::JsErrorKind;
    use crate::generation::{JsConfig, JsGeneration};
    use crate::supervisor::{JsClock, JsFault, JsHealth, JsStrikePolicy, JsSupervisor};
    use crate::watch::ConfigWatcher;

    const DEBOUNCE: Duration = Duration::from_millis(80);
    const PATIENCE: Duration = Duration::from_secs(5);
    const QUIET: Duration = Duration::from_millis(600);

    #[derive(Clone, Default)]
    struct FakeClock(Rc<Cell<Duration>>);

    impl JsClock for FakeClock {
        fn now(&self) -> Duration {
            self.0.get()
        }
    }

    type Commits = Rc<RefCell<Vec<u64>>>;
    type TestCommit = Box<dyn FnMut(&JsGeneration) -> Result<(), String>>;
    type TestReloader = JsReloader<TestCommit, FakeClock>;

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

    fn fixture_with(files: &[(&str, &str)], accept: impl Fn(u64) -> Result<(), String> + 'static) -> Fixture {
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
        // Short enough that a runaway loop in a test is cheap.
        config.stage_budget = Duration::from_millis(100);
        let supervisor = JsSupervisor::new(JsStrikePolicy::default(), FakeClock::default());
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
        let mut fx = fixture(&[("init.ts", V1)]);
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
        let mut fx = fixture(&[("init.ts", V1)]);
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
        let mut fx = fixture(&[("init.ts", "while (true) {}\n")]);
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
}
