//! The running native shell and the reload protocol (`design/modular-framework.md` section 8.1).

use std::fmt;
use std::path::{Path, PathBuf};

use crate::generation::{GenerationId, GenerationState};
use crate::log;
use crate::native::{Copying, LoadError, NativeLoader, NativeModule};
use crate::retire::RetainedStats;

use super::{ReloadController, ReloadTrigger};

/// Why a reload did not take over.
#[derive(Debug)]
pub enum ReloadError {
    /// The new generation could not be staged: no file, not a shell, a wrong ABI, a failing or panicking `init`, or the
    /// retire list was full.
    Load(LoadError),
    /// The staged generation could not take over. The generation was staged a moment before, on this thread, so this is a
    /// host bug rather than a property of the shell; the old generation was not touched.
    Commit(String),
}

impl fmt::Display for ReloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReloadError::Load(error) => error.fmt(f),
            ReloadError::Commit(message) => write!(f, "the staged generation could not take over: {message}"),
        }
    }
}

impl std::error::Error for ReloadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ReloadError::Load(error) => Some(error),
            ReloadError::Commit(_) => None,
        }
    }
}

/// A reload that left the running generation serving.
///
/// Its text is the line the log carries, and the one `gnoblinctl shell reload` prints:
/// `reload of /x/libshell.so failed, still running generation 3: <reason>`.
#[derive(Debug)]
pub struct ReloadFailure {
    /// The module path.
    pub path: PathBuf,
    /// The generation that keeps serving, `None` when none was running.
    pub running: Option<GenerationId>,
    /// Why.
    pub error: ReloadError,
}

impl fmt::Display for ReloadFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.running {
            Some(generation) => write!(
                f,
                "reload of {} failed, still running generation {generation}: {}",
                self.path.display(),
                self.error
            ),
            None => write!(
                f,
                "reload of {} failed, no generation is running: {}",
                self.path.display(),
                self.error
            ),
        }
    }
}

impl std::error::Error for ReloadFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// A reload that committed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reloaded {
    /// The generation that is current now.
    pub generation: GenerationId,
    /// The generation it replaced, `None` when no generation was running.
    pub previous: Option<GenerationId>,
    /// How many registrations the old generation had when it retired (each one's release has run).
    pub removed: usize,
    /// Generations that have retired, in the host's count.
    pub retired_generations: u64,
    /// Calls from retired generations that were refused (`E_RETIRED`), in the host's count.
    pub stale_calls: u64,
    /// The retire list after the commit.
    pub retained: RetainedStats,
}

/// A native shell on the shell thread: the running generation, and what replaces it.
///
/// Not `Send`: it lives on the thread that loaded the shell, which is the shell thread, and every method runs there.
pub struct NativeShell<E: 'static> {
    loader: &'static NativeLoader<E>,
    path: PathBuf,
    current: Option<NativeModule<E>>,
    /// The generations that retired, oldest first. Their libraries are mapped for good, so keeping the handles costs nothing,
    /// and a test or `gnoblinctl shell status` can read each one's last counters.
    retired: Vec<NativeModule<E>>,
    /// Reloads that committed: the new generation's `reload_count` is this plus one.
    committed: u32,
    controller: ReloadController,
}

impl<E: 'static> NativeShell<E> {
    /// A shell that will load `path` through `loader`, reporting to `controller`. Nothing is loaded until
    /// [`start`](Self::start).
    pub fn new(loader: &'static NativeLoader<E>, path: impl Into<PathBuf>, controller: ReloadController) -> Self {
        Self {
            loader,
            path: path.into(),
            current: None,
            retired: Vec::new(),
            committed: 0,
            controller,
        }
    }

    /// The module path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The controller reloads are requested through and reported to.
    pub fn controller(&self) -> &ReloadController {
        &self.controller
    }

    /// The running generation, if the shell has started and not stopped.
    pub fn current(&self) -> Option<&NativeModule<E>> {
        self.current.as_ref()
    }

    /// The generations this shell retired, oldest first. Each is refused every call (`E_RETIRED`); its counters are readable.
    pub fn retired(&self) -> &[NativeModule<E>] {
        &self.retired
    }

    /// Runs one dispatch turn of the running generation, waiting up to `timeout_ms` for work. `None` when nothing runs.
    pub fn dispatch(&self, timeout_ms: u32) -> Option<i32> {
        self.current.as_ref().map(|module| module.dispatch(timeout_ms))
    }

    /// Loads the shell for the first time: stage, `init`, activate. A shell that is running already stays as it is.
    ///
    /// # Errors
    ///
    /// [`LoadError`]: nothing was loaded and nothing is left behind.
    pub fn start(&mut self) -> Result<GenerationId, LoadError> {
        if let Some(current) = &self.current {
            return Ok(current.generation());
        }
        let module = self.loader.load(&self.path, Copying::Unique, 0).inspect_err(|error| {
            log::error!("{}: the shell did not start: {error}", self.path.display());
        })?;
        if let Err(message) = module.activate() {
            module.abandon();
            let error = LoadError::Init {
                path: self.path.clone(),
                code: gnoblin_core_abi::E_ERR,
                message: format!("the staged generation could not take over: {message}"),
            };
            log::error!("{}: the shell did not start: {error}", self.path.display());
            return Err(error);
        }
        let generation = module.generation();
        log::info!(
            generation = generation,
            "{}: generation {generation} ({}) is running",
            self.path.display(),
            module.label()
        );
        self.controller.record_started(generation, self.loader.retained_stats());
        self.current = Some(module);
        Ok(generation)
    }

    /// Takes the request waiting in the controller, if there is one, and reloads for it. The shell thread's loop calls this
    /// between dispatches.
    pub fn serve_requests(&mut self) -> Option<Result<Reloaded, ReloadFailure>> {
        let trigger = self.controller.take_request()?;
        Some(self.reload(trigger))
    }

    /// Replaces the running generation with a fresh load of the module file.
    ///
    /// **Stage.** The file is copied to a private path and opened, and the new generation's `init` runs while the old one
    /// still serves: its registrations receive nothing, its claims and programs wait, and what it writes to the tree is
    /// held in a rollback mark. **Commit.** The old generation's shutdown hooks and `shutdown` run; the new generation takes
    /// over, which releases the old claims and sends the new ones behind the releases in one transaction, starts the programs
    /// the new `init` asked for (adopting the ones the old generation ran with the same arguments), and activates its
    /// registrations; then the old generation retires: its registrations go, each release runs once, and its library joins the
    /// retire list without being unmapped.
    ///
    /// Any failure before the commit is an **abandon**: the old generation is untouched, the new one leaves nothing behind
    /// (what its `init` registered is removed without a callback, what it wrote is rolled back), and the failure comes back
    /// as a [`ReloadFailure`] whose text is also logged and kept for the status. Past the retire limit the reload is refused
    /// before the library is copied.
    ///
    /// # Errors
    ///
    /// [`ReloadFailure`]: the running generation (if any) is still the running generation.
    pub fn reload(&mut self, trigger: ReloadTrigger) -> Result<Reloaded, ReloadFailure> {
        let running = self.current.as_ref().map(NativeModule::generation);
        log::info!("reloading {} ({trigger})", self.path.display());
        let staged = match self.loader.load(&self.path, Copying::Unique, self.committed + 1) {
            Ok(staged) => staged,
            Err(error) => return Err(self.failed(trigger, running, ReloadError::Load(error))),
        };
        if staged.state() != GenerationState::Staged {
            let state = staged.state();
            staged.abandon();
            return Err(self.failed(
                trigger,
                running,
                ReloadError::Commit(format!("generation {} is {state:?}, not staged", staged.generation())),
            ));
        }
        // The old generation finishes first: its hooks may write what its successor reads, and it must not be called again
        // once the new one has its chords.
        if let Some(old) = &self.current {
            old.end();
        }
        if let Err(message) = staged.activate() {
            staged.abandon();
            return Err(self.failed(trigger, running, ReloadError::Commit(message)));
        }
        let (previous, removed) = match self.current.take() {
            Some(old) => {
                let removed = old.retire();
                let id = old.generation();
                self.retired.push(old);
                (Some(id), removed)
            }
            None => (None, 0),
        };
        let generation = staged.generation();
        self.committed += 1;
        let label = staged.label().to_owned();
        self.current = Some(staged);
        let host = self.loader.host().stats();
        let retained = self.loader.retained_stats();
        log::info!(
            generation = generation,
            "generation {generation} ({label}) active ({} retired, {} stale calls rejected); {retained}",
            host.retired_generations,
            host.stale_calls
        );
        self.controller.record_committed(trigger, generation, retained);
        Ok(Reloaded {
            generation,
            previous,
            removed,
            retired_generations: host.retired_generations,
            stale_calls: host.stale_calls,
            retained,
        })
    }

    /// Ends the shell: the running generation's shutdown hooks run and it retires. Returns how many registrations it had.
    pub fn stop(&mut self) -> usize {
        let removed = self.current.take().map_or(0, |module| {
            let removed = module.retire();
            self.retired.push(module);
            removed
        });
        self.controller.record_stopped();
        removed
    }

    /// Logs a failed reload, keeps it for the status, and builds the error the caller gets.
    fn failed(&self, trigger: ReloadTrigger, running: Option<GenerationId>, error: ReloadError) -> ReloadFailure {
        let failure = ReloadFailure {
            path: self.path.clone(),
            running,
            error,
        };
        log::error!("{failure}");
        if matches!(failure.error, ReloadError::Load(LoadError::Init { .. })) {
            log::warn!(
                "what the failed init did outside the ABI (a program it started, a file it wrote) is not undone; \
                 what it registered, claimed and wrote through the ABI was dropped"
            );
        }
        self.controller
            .record_failed(trigger, failure.to_string(), self.loader.retained_stats());
        failure
    }
}
