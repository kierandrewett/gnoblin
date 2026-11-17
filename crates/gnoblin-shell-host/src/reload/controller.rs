//! The handle that asks for a reload and reports what the reloads did.

use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::generation::GenerationId;
use crate::log;
use crate::retire::RetainedStats;

use super::build::BuildFailure;

/// What asked for a reload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReloadTrigger {
    /// The running shell asked (`request_reload`).
    Shell,
    /// The user asked (`gnoblinctl shell reload`), or the host did on a restart.
    User,
    /// The watcher saw these files change. Empty when it may have lost events and cannot say which.
    Files(Vec<PathBuf>),
}

impl ReloadTrigger {
    /// The trigger two requests that arrive before the reload runs become: one reload, for the stronger reason. A user's
    /// request outranks the shell's, and the files of two watcher requests are joined.
    fn merged(self, later: ReloadTrigger) -> ReloadTrigger {
        match (self, later) {
            (ReloadTrigger::Files(mut earlier), ReloadTrigger::Files(more)) => {
                earlier.extend(more);
                earlier.sort();
                earlier.dedup();
                ReloadTrigger::Files(earlier)
            }
            (ReloadTrigger::User, _) | (_, ReloadTrigger::User) => ReloadTrigger::User,
            (ReloadTrigger::Files(files), ReloadTrigger::Shell)
            | (ReloadTrigger::Shell, ReloadTrigger::Files(files)) => ReloadTrigger::Files(files),
            (ReloadTrigger::Shell, ReloadTrigger::Shell) => ReloadTrigger::Shell,
        }
    }
}

impl fmt::Display for ReloadTrigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReloadTrigger::Shell => f.write_str("the shell asked for it"),
            ReloadTrigger::User => f.write_str("it was requested"),
            ReloadTrigger::Files(files) if files.is_empty() => f.write_str("the watcher may have lost events"),
            ReloadTrigger::Files(files) => {
                f.write_str("changed:")?;
                for file in files {
                    write!(f, " {}", file.display())?;
                }
                Ok(())
            }
        }
    }
}

/// What the last reload did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReloadRecord {
    /// What asked for it.
    pub trigger: ReloadTrigger,
    /// The generation that became current, or the failure text (`reload of /x/libshell.so failed, still running
    /// generation 3: ...`).
    pub outcome: Result<GenerationId, String>,
}

/// What `gnoblinctl shell status` shows about reloading.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReloadStatus {
    /// The running generation, `None` before the first load.
    pub generation: Option<GenerationId>,
    /// Reloads that committed.
    pub reloads: u64,
    /// Reloads that failed and left the running generation serving.
    pub failures: u64,
    /// Builds that failed (reported by the build step) since the last one that succeeded.
    pub build_failures: u64,
    /// A request that no reload has taken yet.
    pub pending: Option<ReloadTrigger>,
    /// The last reload, whatever came of it.
    pub last: Option<ReloadRecord>,
    /// The failed build whose output is the reason the shell has not changed, until a build succeeds.
    pub last_build_failure: Option<BuildFailure>,
    /// Why the first load failed, while no generation runs because of it. A shell in this state serves nothing, the
    /// compositor's own window management has the windows, and a reload that loads clears it.
    pub failed: Option<String>,
    /// The retire list, as of the last reload.
    pub retained: Option<RetainedStats>,
}

impl fmt::Display for ReloadStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(error) = &self.failed {
            writeln!(f, "state: failed")?;
            writeln!(f, "the shell did not start: {error}")?;
        }
        match self.generation {
            Some(generation) => writeln!(f, "generation {generation} is running")?,
            None => writeln!(f, "no generation is running")?,
        }
        writeln!(f, "{} reload(s) committed, {} failed", self.reloads, self.failures)?;
        if let Some(retained) = &self.retained {
            writeln!(f, "{retained}")?;
        }
        if let Some(pending) = &self.pending {
            writeln!(f, "a reload is waiting ({pending})")?;
        }
        match &self.last {
            Some(ReloadRecord {
                outcome: Ok(generation),
                trigger,
            }) => {
                writeln!(f, "last reload ({trigger}): generation {generation} took over")?;
            }
            Some(ReloadRecord {
                outcome: Err(message),
                trigger,
            }) => {
                writeln!(f, "last reload ({trigger}) failed: {message}")?;
            }
            None => {}
        }
        if let Some(build) = &self.last_build_failure {
            writeln!(
                f,
                "the last build failed ({} since the last good one): {}",
                self.build_failures,
                build.summary()
            )?;
        }
        Ok(())
    }
}

#[derive(Default)]
struct ControllerState {
    status: ReloadStatus,
}

/// Asks the shell thread to reload, and tells what the reloads did.
///
/// A cheap handle: clones share one state, and every method may be called from any thread. [`request`](Self::request) only
/// records the wish; the shell thread's loop takes it with [`take_request`](Self::take_request) between dispatches, as
/// [`NativeShell::serve_requests`](super::NativeShell::serve_requests) does. Requests that arrive before it looks are one
/// reload.
#[derive(Clone, Default)]
pub struct ReloadController {
    shared: Arc<Mutex<ControllerState>>,
}

impl fmt::Debug for ReloadController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReloadController")
            .field("status", &self.status())
            .finish()
    }
}

impl ReloadController {
    /// A controller with nothing pending.
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, ControllerState> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Asks for a reload. Returns whether one was already waiting, in which case the two are one reload.
    pub fn request(&self, trigger: ReloadTrigger) -> bool {
        let mut shared = self.lock();
        let waiting = shared.status.pending.take();
        let already = waiting.is_some();
        shared.status.pending = Some(match waiting {
            Some(earlier) => earlier.merged(trigger),
            None => trigger,
        });
        already
    }

    /// The request nobody has taken, once. The shell thread calls this between dispatches.
    pub fn take_request(&self) -> Option<ReloadTrigger> {
        self.lock().status.pending.take()
    }

    /// Whether a request waits.
    pub fn has_request(&self) -> bool {
        self.lock().status.pending.is_some()
    }

    /// What `gnoblinctl shell status` shows.
    pub fn status(&self) -> ReloadStatus {
        self.lock().status.clone()
    }

    /// A build step failed: logs the reason with the generation that keeps serving, and keeps it for the status. The
    /// running generation is untouched; nothing is requested, because there is no new library to load.
    pub fn report_build_failure(&self, failure: BuildFailure) {
        let generation = {
            let mut shared = self.lock();
            shared.status.build_failures += 1;
            shared.status.last_build_failure = Some(failure.clone());
            shared.status.generation
        };
        let kept = match generation {
            Some(generation) => format!("still running generation {generation}"),
            None => "no generation is running".to_owned(),
        };
        log::error!(
            "build of {} failed, {kept}: {}",
            failure.project.display(),
            failure.summary()
        );
        for line in failure.stderr.lines() {
            log::error!("    {line}");
        }
    }

    /// A build step succeeded: the failure it replaces is no longer the reason the shell has not changed.
    pub fn report_build_success(&self) {
        let mut shared = self.lock();
        shared.status.last_build_failure = None;
        shared.status.build_failures = 0;
    }

    /// The reload that just ran committed: `generation` is current.
    pub(super) fn record_committed(&self, trigger: ReloadTrigger, generation: GenerationId, retained: RetainedStats) {
        let mut shared = self.lock();
        shared.status.generation = Some(generation);
        shared.status.failed = None;
        shared.status.reloads += 1;
        shared.status.retained = Some(retained);
        shared.status.last = Some(ReloadRecord {
            trigger,
            outcome: Ok(generation),
        });
    }

    /// The first load succeeded: `generation` is current. Not a reload.
    pub(super) fn record_started(&self, generation: GenerationId, retained: RetainedStats) {
        let mut shared = self.lock();
        shared.status.generation = Some(generation);
        shared.status.failed = None;
        shared.status.retained = Some(retained);
    }

    /// A reload failed and the running generation (if any) serves on.
    pub(super) fn record_failed(&self, trigger: ReloadTrigger, message: String, retained: RetainedStats) {
        let mut shared = self.lock();
        shared.status.failures += 1;
        shared.status.retained = Some(retained);
        shared.status.last = Some(ReloadRecord {
            trigger,
            outcome: Err(message.clone()),
        });
        // A reload that fails while the shell has not started is the reason it has not.
        if shared.status.failed.is_some() {
            shared.status.failed = Some(message);
        }
    }

    /// The shell ended: no generation is running.
    pub(super) fn record_stopped(&self) {
        self.lock().status.generation = None;
    }

    /// The first load failed: no generation runs, and `error` says why until one does. The shell host calls this for the
    /// state it holds a failed start in, waiting for a reload; it is not a reload, so no counter moves.
    pub fn record_start_failed(&self, error: String) {
        self.lock().status.failed = Some(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(names: &[&str]) -> ReloadTrigger {
        ReloadTrigger::Files(names.iter().map(PathBuf::from).collect())
    }

    #[test]
    fn requests_before_the_shell_thread_looks_are_one_reload() {
        let controller = ReloadController::new();
        assert!(!controller.has_request());
        assert!(!controller.request(ReloadTrigger::Shell));
        assert!(
            controller.request(ReloadTrigger::Shell),
            "the second is told one was waiting"
        );
        assert_eq!(controller.take_request(), Some(ReloadTrigger::Shell));
        assert_eq!(controller.take_request(), None, "a request is taken once");
        assert!(!controller.has_request());
    }

    #[test]
    fn a_user_request_outranks_the_shell_and_file_requests_join() {
        let controller = ReloadController::new();
        controller.request(ReloadTrigger::Shell);
        controller.request(ReloadTrigger::User);
        controller.request(ReloadTrigger::Shell);
        assert_eq!(controller.take_request(), Some(ReloadTrigger::User));

        controller.request(files(&["/b.so"]));
        controller.request(files(&["/a.so", "/b.so"]));
        assert_eq!(controller.take_request(), Some(files(&["/a.so", "/b.so"])));

        controller.request(ReloadTrigger::Shell);
        controller.request(files(&["/a.so"]));
        assert_eq!(
            controller.take_request(),
            Some(files(&["/a.so"])),
            "a change names the cause better than the shell"
        );
    }

    #[test]
    fn clones_share_one_request_from_any_thread() {
        let controller = ReloadController::new();
        let other = controller.clone();
        std::thread::spawn(move || {
            other.request(ReloadTrigger::User);
        })
        .join()
        .expect("the requesting thread ends");
        assert_eq!(controller.take_request(), Some(ReloadTrigger::User));
    }

    #[test]
    fn a_failed_build_is_kept_for_the_status_until_one_succeeds() {
        let controller = ReloadController::new();
        let failure = BuildFailure {
            project: PathBuf::from("/work/shell"),
            command: "cargo build".to_owned(),
            status: Some(101),
            stderr: "error[E0425]: cannot find value `x` in this scope\n".to_owned(),
        };
        controller.report_build_failure(failure.clone());
        controller.report_build_failure(failure.clone());
        let status = controller.status();
        assert_eq!(status.build_failures, 2);
        assert_eq!(status.last_build_failure.as_ref(), Some(&failure));
        assert!(
            status
                .to_string()
                .contains("the last build failed (2 since the last good one)"),
            "{status}"
        );
        controller.report_build_success();
        let status = controller.status();
        assert_eq!(status.build_failures, 0);
        assert_eq!(status.last_build_failure, None);
    }

    #[test]
    fn the_status_says_what_the_last_reload_did() {
        let controller = ReloadController::new();
        controller.record_started(1, RetainedStats::default());
        controller.record_committed(files(&["/a.so"]), 2, RetainedStats::default());
        controller.record_failed(
            ReloadTrigger::User,
            "reload of /a.so failed, still running generation 2: x".to_owned(),
            RetainedStats::default(),
        );
        let status = controller.status();
        assert_eq!((status.reloads, status.failures, status.generation), (1, 1, Some(2)));
        let text = status.to_string();
        assert!(text.contains("generation 2 is running"), "{text}");
        assert!(text.contains("1 reload(s) committed, 1 failed"), "{text}");
        assert!(
            text.contains(
                "last reload (it was requested) failed: reload of /a.so failed, still running generation 2: x"
            ),
            "{text}"
        );
    }

    #[test]
    fn a_failed_start_is_the_status_until_a_generation_runs() {
        let controller = ReloadController::new();
        controller.record_start_failed("shell.ts:3:26: unexpected token".to_owned());
        let status = controller.status();
        assert_eq!(status.failed.as_deref(), Some("shell.ts:3:26: unexpected token"));
        assert_eq!(
            (status.reloads, status.failures),
            (0, 0),
            "a failed start is not a reload"
        );
        let text = status.to_string();
        assert!(text.contains("state: failed"), "{text}");
        assert!(
            text.contains("the shell did not start: shell.ts:3:26: unexpected token"),
            "{text}"
        );
        assert!(text.contains("no generation is running"), "{text}");

        // A reload that fails too is the reason now, and counts as a failure.
        controller.record_failed(
            ReloadTrigger::User,
            "reload of /x/shell.ts failed, no generation is running: still broken".to_owned(),
            RetainedStats::default(),
        );
        let status = controller.status();
        assert_eq!(status.failures, 1);
        assert!(
            status.failed.as_deref().is_some_and(|why| why.contains("still broken")),
            "{status:?}"
        );

        // One that loads clears it.
        controller.record_committed(ReloadTrigger::User, 1, RetainedStats::default());
        let status = controller.status();
        assert_eq!((status.failed, status.generation, status.reloads), (None, Some(1), 1));
        assert!(!controller.status().to_string().contains("state: failed"));
    }
}
