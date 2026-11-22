//! Hot reload of the JavaScript shell: stage the config from disk, hand it to the compositor, then swap it in.
//!
//! The protocol is the shell host's (`gnoblin_shell_host::stage::Reloader`): stage a fresh generation, pass it to the
//! caller's `commit` closure (the compositor commit of design/js/research/quickjs-embedding.md, section 5, step 5), and
//! only when that accepts make it current and drop the old one. Any failure on the way, a parse error, a throw, a runaway
//! loop, a rejected commit, leaves the running generation exactly as it was and comes back as a [`ReloadReport`] with the
//! failing file, line and column in the author's `.ts` coordinates.
//!
//! This module is what is specific to QuickJS: [`JsStager`] stages a generation through the [`JsHost`] and describes a
//! [`JsError`] as a located failure, and [`JsReloader`] is the reloader over it that owns the [`JsHost`] and the entry
//! module. Three things start a reload: a change the watcher saw ([`JsReloader::poll`]), the user
//! ([`JsReloader::reload_now`], for `gnoblinctl reload`), and a fault in the running generation
//! ([`JsReloader::recover`], which consults the [`StrikeSupervisor`]). The first two are deliberate retries and clear the
//! supervisor's strikes.
//!
//! The tests are in `tests.rs`.

use std::path::PathBuf;
use std::time::Duration;

use gnoblin_shell_host::stage::{Diagnostic, Located, Reloader, Stager};
use gnoblin_shell_host::supervisor::{Clock, GenerationFault, Health, StrikeSupervisor, SystemClock};
use gnoblin_shell_host::watch::{ConfigWatcher, ReloadRequest};

use crate::error::{JsError, JsErrorKind};
use crate::generation::{JsConfig, JsGeneration};
use crate::host::{JsHost, JsPrepare};

/// The extensions of the files the JavaScript runtime loads: what the watcher treats as a change to the shell.
pub(crate) const SCRIPT_EXTENSIONS: &[&str] = &["ts", "mts", "js", "mjs"];

/// One failure in a reload, at the position the author wrote.
pub type JsDiagnostic = Diagnostic<JsErrorKind>;

/// What a reload did.
pub type ReloadReport = gnoblin_shell_host::stage::ReloadReport<JsErrorKind>;

/// The supervisor fault `error` is, if it is one. Only an interrupt counts: a thrown exception is the script reporting its
/// own failure and leaves the generation consistent, and a memory error is catchable.
pub(crate) fn fault_of(error: &JsError) -> Option<GenerationFault> {
    (error.kind == JsErrorKind::Budget).then_some(GenerationFault::Budget)
}

/// Stages generations from the entry module through the [`JsHost`].
pub struct JsStager {
    host: JsHost,
    entry: PathBuf,
}

impl Stager for JsStager {
    type Generation = JsGeneration;
    type Failure = JsError;
    type Kind = JsErrorKind;

    fn stage(&mut self) -> Result<JsGeneration, JsError> {
        self.host.stage(&self.entry)
    }

    fn commit(&mut self, generation: JsGeneration) {
        self.host.commit(generation);
    }

    fn current_id(&self) -> Option<u64> {
        self.host.current().map(JsGeneration::id)
    }

    fn current_files(&self) -> Vec<PathBuf> {
        self.host.current().map(JsGeneration::module_paths).unwrap_or_default()
    }

    fn fault(failure: &JsError) -> Option<GenerationFault> {
        fault_of(failure)
    }

    fn locate(failure: &JsError) -> Located<JsErrorKind> {
        Located {
            at: failure
                .location
                .as_ref()
                .map(|at| (at.file.clone(), at.line, at.column)),
            message: failure.message.clone(),
            kind: failure.kind,
        }
    }

    fn budget_kind() -> JsErrorKind {
        JsErrorKind::Budget
    }
}

/// Stages, commits and recovers the shell generation.
///
/// `commit` runs on a staged generation that evaluated cleanly. It returns `Ok` once the compositor accepted the
/// generation's transaction, or the reason it refused, in which case nothing changes.
pub struct JsReloader<F, C = SystemClock> {
    inner: Reloader<JsStager, F, C>,
}

impl<F, C> std::fmt::Debug for JsReloader<F, C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsReloader")
            .field("entry", &self.inner.stager().entry)
            .field("host", &self.inner.stager().host)
            .finish_non_exhaustive()
    }
}

impl<F, C> JsReloader<F, C>
where
    F: FnMut(&JsGeneration) -> Result<(), String>,
    C: Clock,
{
    /// A reloader with no generation yet. `entry` is the config's entry module, relative to the config
    /// root or absolute. Call [`reload_now`](Self::reload_now) to load it the first time.
    pub fn new(config: JsConfig, entry: impl Into<PathBuf>, commit: F, supervisor: StrikeSupervisor<C>) -> Self {
        let entry = entry.into();
        let entry_path = if entry.is_absolute() {
            entry.clone()
        } else {
            config.root.join(&entry)
        };
        let stager = JsStager {
            host: JsHost::new(config),
            entry,
        };
        Self {
            inner: Reloader::new(stager, entry_path, commit, supervisor),
        }
    }

    /// Runs `prepare` on every generation this reloader stages, before the entry module is evaluated (see
    /// [`JsHost::set_prepare`]).
    pub fn set_prepare(&mut self, prepare: JsPrepare) {
        self.inner.stager_mut().host.set_prepare(prepare);
    }

    /// The generation that is running, if any.
    pub fn current(&self) -> Option<&JsGeneration> {
        self.inner.stager().host.current()
    }

    /// Whether the supervisor still allows restaging after a fault.
    pub fn health(&self) -> Health {
        self.inner.health()
    }

    /// The files a watcher should observe: the module graph of the current generation and any file the
    /// last failed stage named.
    pub fn watch_set(&self) -> Vec<PathBuf> {
        self.inner.watch_set()
    }

    /// Reloads because the user asked: `gnoblinctl reload`, or the first load at startup.
    pub fn reload_now(&mut self) -> ReloadReport {
        self.inner.reload_now()
    }

    /// Reloads because `request` came from the watcher.
    pub fn reload_for(&mut self, request: &ReloadRequest) -> ReloadReport {
        self.inner.reload_for(request)
    }

    /// Waits up to `wait` for the watcher to report a change, reloads, and re-arms the watcher from the
    /// new module graph. `None` when nothing changed in that time.
    pub fn poll(&mut self, watcher: &mut ConfigWatcher, wait: Duration) -> Option<ReloadReport> {
        self.inner.poll(watcher, wait)
    }

    /// Reloads after the running generation was interrupted or panicked: the supervisor counts the fault and decides
    /// whether to restage from disk or to keep the last good generation (`Reloader::recover`).
    pub fn recover(&mut self, fault: GenerationFault) -> ReloadReport {
        self.inner.recover(fault)
    }
}

#[cfg(test)]
mod tests;
