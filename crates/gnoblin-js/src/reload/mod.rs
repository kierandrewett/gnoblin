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
//!
//! The tests are in `tests.rs`.

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
mod tests;
