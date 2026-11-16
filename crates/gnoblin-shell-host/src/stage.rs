//! Hot reload and swap: the protocol every language runtime shares.
//!
//! A reload stages a fresh generation from disk, hands it to the caller's `commit` closure (the compositor commit,
//! `design/js-research/quickjs-embedding.md` section 5, step 5), and only when that accepts does it make the generation
//! current and drop the old one. Any failure on the way (a parse error, a throw, a runaway loop, a failing `init`, a
//! rejected commit) leaves the running generation exactly as it was and comes back as a [`ReloadReport`] with the failing
//! file, line and column in the author's coordinates.
//!
//! The protocol has three parts, each independent of the language that runs the shell:
//!
//! - [`Reloader`] drives stage, commit and recovery over a [`Stager`], which a language runtime implements (QuickJS
//!   stages a fresh `Context`, a native shell loads a unique copy). Three things start a reload: a change the
//!   [`ConfigWatcher`] saw ([`Reloader::poll`]), the user ([`Reloader::reload_now`], for `gnoblinctl reload`), and a fault
//!   in the running generation ([`Reloader::recover`], which consults the [`StrikeSupervisor`]). The first two are deliberate
//!   retries and clear the supervisor's strikes.
//! - [`RollbackSlot`] holds the rollback mark of a stage: what the shell must put back if the stage is abandoned.
//! - [`swap`] is the order a staged generation takes over in, over the steps of a [`SwapHost`]: the old generation's state
//!   is read, the new generation's programs start, the old generation ends, and the new generation's seat claims go out
//!   behind the old one's releases in one transaction, so a chord is never refused for being held and never held by
//!   nobody. A step that fails after the swap is under way is a diagnostic, not an error: the swap has happened.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::supervisor::{Clock, FaultAction, GenerationFault, Health, StrikeSupervisor, SystemClock};
use crate::watch::{ConfigWatcher, ReloadRequest};

// ---- reports -------------------------------------------------------------------------------------------------------

/// One failure in a reload, at the position the author wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic<K> {
    /// The file the failure is in. The entry module when the runtime reported no position.
    pub file: PathBuf,
    /// The 1-based line, or 0 when the runtime reported no position.
    pub line: u32,
    /// The 1-based column, or 0 when the runtime reported no position.
    pub column: u32,
    /// What went wrong.
    pub message: String,
    /// The runtime's kind of failure, or `None` when the commit closure rejected the generation.
    pub kind: Option<K>,
}

impl<K> fmt::Display for Diagnostic<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
pub struct ReloadReport<K> {
    /// Whether a new generation is now current.
    pub ok: bool,
    /// The id of the generation that is current after the reload, `None` when there is none.
    pub generation: Option<u64>,
    /// Why the reload failed. Empty when `ok`.
    pub errors: Vec<Diagnostic<K>>,
    /// Problems that did not fail the reload, such as a directory the watcher could not watch.
    pub warnings: Vec<String>,
    /// The files whose change triggered the reload; empty for a manual or recovery reload.
    pub changed: Vec<PathBuf>,
    /// Whether the supervisor still allows restaging after a fault.
    pub health: Health,
}

/// Where a failed stage went wrong, in the language runtime's own terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located<K> {
    /// The file, line and column of the failure, when the runtime knows them.
    pub at: Option<(PathBuf, u32, u32)>,
    /// What went wrong.
    pub message: String,
    /// The runtime's kind of failure.
    pub kind: K,
}

// ---- the stager ----------------------------------------------------------------------------------------------------

/// What a language runtime provides for a reload: how to stage a generation, how to make it current, and how to describe a
/// failure.
pub trait Stager {
    /// A staged generation: evaluated, not yet current.
    type Generation;
    /// Why a stage failed.
    type Failure;
    /// The runtime's kind of failure, carried on a [`Diagnostic`].
    type Kind: Clone;

    /// Stages a fresh generation from disk. A failure leaves the current generation untouched.
    ///
    /// # Errors
    ///
    /// The runtime's failure: the source did not load, a throw, a budget trip, a failing `init`.
    fn stage(&mut self) -> Result<Self::Generation, Self::Failure>;

    /// Makes `generation` current and drops the old one.
    fn commit(&mut self, generation: Self::Generation);

    /// The id of the generation that is running, if any.
    fn current_id(&self) -> Option<u64>;

    /// The files of the running generation's module graph, for the watcher.
    fn current_files(&self) -> Vec<PathBuf>;

    /// The supervisor fault `failure` is, if any (only an interrupt is one).
    fn fault(failure: &Self::Failure) -> Option<GenerationFault>;

    /// Where `failure` is and what it says.
    fn locate(failure: &Self::Failure) -> Located<Self::Kind>;

    /// The kind carried by the "interrupted too many times" diagnostic.
    fn budget_kind() -> Self::Kind;
}

/// Why a stage did not become the current generation.
enum StageFailure<F> {
    /// The runtime failed to stage it.
    Runtime(F),
    /// It staged, and the commit closure refused it.
    Rejected(String),
}

/// Stages, commits and recovers the shell generation.
///
/// `commit` runs on a staged generation that evaluated cleanly. It returns `Ok` once the compositor accepted the
/// generation's transaction, or the reason it refused, in which case nothing changes.
pub struct Reloader<S: Stager, F, C = SystemClock> {
    stager: S,
    entry_path: PathBuf,
    commit: F,
    supervisor: StrikeSupervisor<C>,
    /// Files named by the last failed stage. A failing module is in no generation's graph, and the edit that fixes it
    /// must still be seen.
    failed_files: Vec<PathBuf>,
}

impl<S: Stager, F, C> fmt::Debug for Reloader<S, F, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reloader")
            .field("entry_path", &self.entry_path)
            .field("generation", &self.stager.current_id())
            .finish_non_exhaustive()
    }
}

impl<S: Stager, F, C> Reloader<S, F, C> {
    /// The language runtime's stager.
    pub fn stager(&self) -> &S {
        &self.stager
    }

    /// The language runtime's stager, mutably (to configure what it stages).
    pub fn stager_mut(&mut self) -> &mut S {
        &mut self.stager
    }
}

impl<S, F, C> Reloader<S, F, C>
where
    S: Stager,
    F: FnMut(&S::Generation) -> Result<(), String>,
    C: Clock,
{
    /// A reloader with no generation yet. `entry_path` is the shell's entry file, absolute, for diagnostics that carry no
    /// position. Call [`reload_now`](Self::reload_now) to load it the first time.
    pub fn new(stager: S, entry_path: impl Into<PathBuf>, commit: F, supervisor: StrikeSupervisor<C>) -> Self {
        Self {
            stager,
            entry_path: entry_path.into(),
            commit,
            supervisor,
            failed_files: Vec::new(),
        }
    }

    /// Whether the supervisor still allows restaging after a fault.
    pub fn health(&self) -> Health {
        self.supervisor.health()
    }

    /// The files a watcher should observe: the module graph of the current generation and any file the last failed stage
    /// named.
    pub fn watch_set(&self) -> Vec<PathBuf> {
        let mut files = self.stager.current_files();
        files.extend(self.failed_files.iter().cloned());
        files.sort();
        files.dedup();
        files
    }

    /// Reloads because the user asked: `gnoblinctl reload`, or the first load at startup.
    pub fn reload_now(&mut self) -> ReloadReport<S::Kind> {
        self.reload(Vec::new())
    }

    /// Reloads because `request` came from the watcher.
    pub fn reload_for(&mut self, request: &ReloadRequest) -> ReloadReport<S::Kind> {
        self.reload(request.paths.clone())
    }

    /// Waits up to `wait` for the watcher to report a change, reloads, and re-arms the watcher from the new module graph.
    /// `None` when nothing changed in that time.
    pub fn poll(&mut self, watcher: &mut ConfigWatcher, wait: Duration) -> Option<ReloadReport<S::Kind>> {
        let request = watcher.next_request(wait)?;
        let mut report = self.reload_for(&request);
        if let Err(error) = watcher.watch_modules(&self.watch_set()) {
            report.warnings.push(error.to_string());
        }
        Some(report)
    }

    /// Reloads after the running generation was interrupted or panicked.
    ///
    /// The supervisor counts the fault. While it allows a retry, a fresh generation is staged from disk and replaces the
    /// suspect one; a restage that is itself interrupted is another strike. When the strikes run out the suspect
    /// generation is left running as the last good one, if there is one, and the report says so. A restage that fails any
    /// other way is not retried: the files on disk would fail the same way.
    pub fn recover(&mut self, fault: GenerationFault) -> ReloadReport<S::Kind> {
        let mut fault = fault;
        loop {
            match self.supervisor.record_fault(fault) {
                FaultAction::GiveUp => return self.gave_up(),
                FaultAction::Restage => match self.stage_and_commit() {
                    Ok(()) => return self.report(Vec::new(), Vec::new()),
                    Err(failure) => {
                        let next = match &failure {
                            StageFailure::Runtime(failure) => S::fault(failure),
                            StageFailure::Rejected(_) => None,
                        };
                        match next {
                            Some(next) => fault = next,
                            None => return self.report(vec![self.diagnostic(&failure)], Vec::new()),
                        }
                    }
                },
            }
        }
    }

    fn reload(&mut self, changed: Vec<PathBuf>) -> ReloadReport<S::Kind> {
        self.supervisor.user_reload();
        let errors = match self.stage_and_commit() {
            Ok(()) => Vec::new(),
            Err(failure) => vec![self.diagnostic(&failure)],
        };
        self.report(errors, changed)
    }

    /// Stages from disk and, when it evaluates and the commit closure accepts, makes it current.
    fn stage_and_commit(&mut self) -> Result<(), StageFailure<S::Failure>> {
        let generation = match self.stager.stage() {
            Ok(generation) => generation,
            Err(failure) => {
                self.failed_files = S::locate(&failure).at.into_iter().map(|(file, _, _)| file).collect();
                return Err(StageFailure::Runtime(failure));
            }
        };
        if let Err(reason) = (self.commit)(&generation) {
            return Err(StageFailure::Rejected(reason));
        }
        self.failed_files.clear();
        self.stager.commit(generation);
        Ok(())
    }

    fn diagnostic(&self, failure: &StageFailure<S::Failure>) -> Diagnostic<S::Kind> {
        match failure {
            StageFailure::Rejected(reason) => Diagnostic {
                file: self.entry_path.clone(),
                line: 0,
                column: 0,
                message: format!("the compositor rejected the generation: {reason}"),
                kind: None,
            },
            StageFailure::Runtime(failure) => {
                let located = S::locate(failure);
                match located.at {
                    Some((file, line, column)) => Diagnostic {
                        file,
                        line,
                        column,
                        message: located.message,
                        kind: Some(located.kind),
                    },
                    None => Diagnostic {
                        file: self.entry_path.clone(),
                        line: 0,
                        column: 0,
                        message: located.message,
                        kind: Some(located.kind),
                    },
                }
            }
        }
    }

    fn gave_up(&self) -> ReloadReport<S::Kind> {
        let kept = match self.stager.current_id() {
            Some(id) => format!("generation {id} stays as the last good one"),
            None => "no generation is running".to_owned(),
        };
        let diagnostic = Diagnostic {
            file: self.entry_path.clone(),
            line: 0,
            column: 0,
            message: format!(
                "the config was interrupted too many times; not restaging until it is edited, \
                 `gnoblinctl reload` runs, or the cool-down passes: {kept}"
            ),
            kind: Some(S::budget_kind()),
        };
        self.report(vec![diagnostic], Vec::new())
    }

    fn report(&self, errors: Vec<Diagnostic<S::Kind>>, changed: Vec<PathBuf>) -> ReloadReport<S::Kind> {
        ReloadReport {
            ok: errors.is_empty(),
            generation: self.stager.current_id(),
            errors,
            warnings: Vec::new(),
            changed,
            health: self.supervisor.health(),
        }
    }
}

// ---- the rollback mark ---------------------------------------------------------------------------------------------

/// The rollback mark of a stage in progress.
///
/// A generation staged through a shell evaluates the user's entry against the live tree, so a top-level write lands in the
/// shell's pending writes before it is known that the stage succeeds. `M` is whatever the shell needs to put back (its
/// pending writes, its canvases, its claims and spawns): [`begin`](Self::begin) records it, [`abandon`](Self::abandon)
/// hands it back so the shell can restore it, and [`commit`](Self::commit) forgets it when the generation takes over.
#[derive(Debug)]
pub struct RollbackSlot<M> {
    mark: Option<M>,
}

impl<M> Default for RollbackSlot<M> {
    fn default() -> Self {
        Self { mark: None }
    }
}

impl<M> RollbackSlot<M> {
    /// A stage is starting; `mark` is what to put back if it is abandoned. A stage already open is replaced (a restage
    /// inside one stage rolls back to the earliest mark the shell kept, which the caller decides by not calling again).
    pub fn begin(&mut self, mark: M) {
        self.mark = Some(mark);
    }

    /// The stage failed: the mark to restore, once. `None` when no stage is open.
    pub fn abandon(&mut self) -> Option<M> {
        self.mark.take()
    }

    /// The staged generation took over: the mark is no longer needed.
    pub fn commit(&mut self) {
        self.mark = None;
    }

    /// Whether a stage is open.
    pub fn is_open(&self) -> bool {
        self.mark.is_some()
    }
}

// ---- the swap ------------------------------------------------------------------------------------------------------

/// A step of a swap that did not go to plan.
#[derive(Debug)]
pub enum StepError<F> {
    /// The step failed in the generation's own code. The swap goes on; the text is the diagnostic to report.
    Recovered(String),
    /// The link failed: the shell has ended and the swap stops.
    Fatal(F),
}

/// The steps a shell performs to replace the running generation with a staged one. [`swap`] calls them in the one order
/// that is correct; a shell implements what each does in its own terms.
pub trait SwapHost {
    /// The staged generation that takes over.
    type Generation;
    /// What the old generation knew that the new one needs (which window is on which workspace).
    type Snapshot;
    /// The seat claims the staged generation kept apart while it was staged.
    type Claims;
    /// A failure the swap cannot go on after.
    type Fatal;

    /// Reads the old generation's state before anything of it ends.
    ///
    /// # Errors
    ///
    /// [`StepError::Recovered`] when the old generation could not say: the new one then starts from the stock rule.
    fn snapshot(&mut self) -> Result<Self::Snapshot, StepError<Self::Fatal>>;

    /// Starts the programs the staged generation spawned, and reports the ones that could not start.
    fn start_programs(&mut self, generation: &Self::Generation) -> Vec<String>;

    /// Takes the seat claims the staged generation kept apart.
    fn take_claims(&mut self) -> Self::Claims;

    /// Ends the old generation: its drags are cancelled and its handlers told. Returns what it reports.
    ///
    /// # Errors
    ///
    /// [`StepError::Recovered`] when it did not end cleanly.
    fn end_old(&mut self) -> Result<Vec<String>, StepError<Self::Fatal>>;

    /// Gives back every claim the old generation still holds, sends the staged claims behind those releases, and sends the
    /// transaction. The compositor applies a transaction's calls in order, so each chord is free when its new claim
    /// arrives.
    ///
    /// # Errors
    ///
    /// The link failed.
    fn exchange_claims(&mut self, staged: Self::Claims) -> Result<(), Self::Fatal>;

    /// Makes the staged generation the running one and resets what counts against a generation (its strikes, its error
    /// reports).
    fn install(&mut self, generation: Self::Generation);

    /// The new generation takes over what the old one held: finishes its load and restores `previous`.
    ///
    /// # Errors
    ///
    /// [`StepError::Recovered`] when it could not take over.
    fn take_over(&mut self, previous: Option<&Self::Snapshot>) -> Result<(), StepError<Self::Fatal>>;

    /// What follows a swap: the settings the new generation wrote are applied and the failures of the calls that ran are
    /// collected into `diagnostics`.
    ///
    /// # Errors
    ///
    /// The link failed.
    fn finish(&mut self, diagnostics: &mut Vec<String>) -> Result<(), Self::Fatal>;
}

/// Replaces the running generation with `generation` in the order that keeps chords held by someone at every moment:
///
/// 1. the old generation's snapshot is read, while it is whole;
/// 2. the new generation's programs start, before the old one ends, so what the old one's end handlers spawn is a
///    handler's spawn and not one that waits for a takeover already under way;
/// 3. the claims the new generation kept are taken apart from the old one's;
/// 4. the old generation ends;
/// 5. its claims are released and the new claims are sent behind the releases in one transaction (release first, claim
///    behind);
/// 6. the new generation is installed, takes over what the old one held, and its settings are applied.
///
/// A script failure while the old generation ends or the new one takes over is a diagnostic in the result, not an error:
/// the swap has happened.
///
/// # Errors
///
/// The shell's fatal error: the link failed and the shell has ended.
pub fn swap<H: SwapHost>(host: &mut H, generation: H::Generation) -> Result<Vec<String>, H::Fatal> {
    let mut diagnostics = Vec::new();
    let previous = match host.snapshot() {
        Ok(snapshot) => Some(snapshot),
        Err(StepError::Recovered(message)) => {
            diagnostics.push(message);
            None
        }
        Err(StepError::Fatal(fatal)) => return Err(fatal),
    };
    diagnostics.extend(host.start_programs(&generation));
    let staged = host.take_claims();
    match host.end_old() {
        Ok(reported) => diagnostics.extend(reported),
        Err(StepError::Recovered(message)) => diagnostics.push(message),
        Err(StepError::Fatal(fatal)) => return Err(fatal),
    }
    host.exchange_claims(staged)?;
    host.install(generation);
    match host.take_over(previous.as_ref()) {
        Ok(()) => {}
        Err(StepError::Recovered(message)) => diagnostics.push(message),
        Err(StepError::Fatal(fatal)) => return Err(fatal),
    }
    host.finish(&mut diagnostics)?;
    Ok(diagnostics)
}

/// The entry path of a reloader, for a caller that keeps the config root and the entry name apart: `entry` relative to
/// `root`, or absolute.
pub fn entry_path(root: &Path, entry: &Path) -> PathBuf {
    if entry.is_absolute() {
        entry.to_path_buf()
    } else {
        root.join(entry)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::supervisor::{ManualClock, StrikePolicy};

    // ---- a fake language runtime ------------------------------------------------------------------------------------

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Kind {
        Syntax,
        Budget,
    }

    /// What staging the next time does.
    #[derive(Debug, Clone)]
    enum Script {
        Good,
        Fail {
            kind: Kind,
            at: Option<(PathBuf, u32, u32)>,
            message: &'static str,
        },
    }

    struct FakeStager {
        scripts: Vec<Script>,
        staged: u64,
        current: Option<u64>,
        files: Vec<PathBuf>,
    }

    impl FakeStager {
        fn new(scripts: Vec<Script>) -> Self {
            Self {
                scripts,
                staged: 0,
                current: None,
                files: vec![PathBuf::from("/c/init.ts")],
            }
        }
    }

    #[derive(Debug)]
    struct Failure {
        kind: Kind,
        at: Option<(PathBuf, u32, u32)>,
        message: &'static str,
    }

    impl Stager for FakeStager {
        type Generation = u64;
        type Failure = Failure;
        type Kind = Kind;

        fn stage(&mut self) -> Result<u64, Failure> {
            let script = if self.scripts.len() > 1 {
                self.scripts.remove(0)
            } else {
                self.scripts[0].clone()
            };
            match script {
                Script::Good => {
                    self.staged += 1;
                    Ok(self.staged)
                }
                Script::Fail { kind, at, message } => Err(Failure { kind, at, message }),
            }
        }

        fn commit(&mut self, generation: u64) {
            self.current = Some(generation);
        }

        fn current_id(&self) -> Option<u64> {
            self.current
        }

        fn current_files(&self) -> Vec<PathBuf> {
            self.files.clone()
        }

        fn fault(failure: &Failure) -> Option<GenerationFault> {
            (failure.kind == Kind::Budget).then_some(GenerationFault::Budget)
        }

        fn locate(failure: &Failure) -> Located<Kind> {
            Located {
                at: failure.at.clone(),
                message: failure.message.to_owned(),
                kind: failure.kind.clone(),
            }
        }

        fn budget_kind() -> Kind {
            Kind::Budget
        }
    }

    type Accept = Box<dyn FnMut(&u64) -> Result<(), String>>;

    fn reloader(scripts: Vec<Script>, accept: Accept) -> (Reloader<FakeStager, Accept, ManualClock>, ManualClock) {
        let clock = ManualClock::default();
        let supervisor = StrikeSupervisor::new(StrikePolicy::default(), clock.clone());
        (
            Reloader::new(FakeStager::new(scripts), "/c/init.ts", accept, supervisor),
            clock,
        )
    }

    fn accept_all() -> Accept {
        Box::new(|_| Ok(()))
    }

    fn broken(kind: Kind) -> Script {
        Script::Fail {
            kind,
            at: Some((PathBuf::from("/c/lib/x.ts"), 7, 3)),
            message: "unexpected token",
        }
    }

    // ---- reload -----------------------------------------------------------------------------------------------------

    #[test]
    fn a_good_stage_becomes_the_current_generation() {
        let (mut reloader, _clock) = reloader(vec![Script::Good], accept_all());
        let report = reloader.reload_now();
        assert!(report.ok, "{report:?}");
        assert_eq!(report.generation, Some(1));
        assert!(report.errors.is_empty());
        assert_eq!(reloader.stager().current_id(), Some(1));
    }

    #[test]
    fn a_failed_stage_keeps_the_old_generation_and_names_the_failing_file() {
        let (mut reloader, _clock) = reloader(vec![Script::Good, broken(Kind::Syntax)], accept_all());
        assert!(reloader.reload_now().ok);
        let report = reloader.reload_now();
        assert!(!report.ok);
        assert_eq!(report.generation, Some(1), "generation 1 is still the one running");
        assert_eq!(reloader.stager().current_id(), Some(1));
        let diagnostic = &report.errors[0];
        assert_eq!(
            (diagnostic.file.as_path(), diagnostic.line, diagnostic.column),
            (Path::new("/c/lib/x.ts"), 7, 3)
        );
        assert_eq!(diagnostic.kind, Some(Kind::Syntax));
        assert_eq!(diagnostic.to_string(), "/c/lib/x.ts:7:3: unexpected token");
        // The failing file is in no graph, and the edit that fixes it must still be seen.
        assert_eq!(
            reloader.watch_set(),
            vec![PathBuf::from("/c/init.ts"), PathBuf::from("/c/lib/x.ts")]
        );
    }

    #[test]
    fn a_good_stage_after_a_failure_clears_the_failing_file_from_the_watch_set() {
        let (mut reloader, _clock) = reloader(vec![broken(Kind::Syntax), Script::Good], accept_all());
        assert!(!reloader.reload_now().ok);
        assert!(reloader.watch_set().contains(&PathBuf::from("/c/lib/x.ts")));
        assert!(reloader.reload_now().ok);
        assert_eq!(reloader.watch_set(), vec![PathBuf::from("/c/init.ts")]);
    }

    #[test]
    fn a_rejected_commit_keeps_the_old_generation_and_says_who_refused() {
        let refuse_second: Accept = Box::new(|generation| {
            if *generation == 2 {
                Err("the transaction was refused".to_owned())
            } else {
                Ok(())
            }
        });
        let (mut reloader, _clock) = reloader(vec![Script::Good], refuse_second);
        assert!(reloader.reload_now().ok);
        let report = reloader.reload_now();
        assert!(!report.ok);
        assert_eq!(
            report.generation,
            Some(1),
            "the generation that was refused never became current"
        );
        let diagnostic = &report.errors[0];
        assert_eq!(diagnostic.kind, None, "a rejection is not a runtime failure");
        assert_eq!(diagnostic.file, PathBuf::from("/c/init.ts"));
        assert_eq!(
            diagnostic.message,
            "the compositor rejected the generation: the transaction was refused"
        );
    }

    #[test]
    fn a_fault_restages_until_the_strikes_run_out_and_then_keeps_the_last_good_generation() {
        let (mut reloader, clock) = reloader(vec![Script::Good, broken(Kind::Budget)], accept_all());
        assert!(reloader.reload_now().ok);
        // Every restage trips the budget again: two strikes restage, the third gives up.
        let report = reloader.recover(GenerationFault::Budget);
        assert!(!report.ok);
        assert!(
            report.errors[0]
                .message
                .contains("generation 1 stays as the last good one"),
            "{report:?}"
        );
        assert!(matches!(report.health, Health::Degraded { .. }));
        assert_eq!(reloader.stager().current_id(), Some(1));

        // While degraded, nothing restages; after the cool-down the slate is clean.
        clock.advance(Duration::from_secs(301));
        assert_eq!(reloader.health(), Health::Healthy);
    }

    #[test]
    fn a_user_reload_clears_the_strikes() {
        let (mut reloader, _clock) = reloader(vec![Script::Good], accept_all());
        reloader.reload_now();
        let report = reloader.recover(GenerationFault::Panic);
        assert!(report.ok, "a clean restage recovers: {report:?}");
        assert_eq!(report.generation, Some(2));
        assert_eq!(report.health, Health::Healthy);
    }

    #[test]
    fn a_restage_that_fails_another_way_is_not_retried() {
        let (mut reloader, _clock) = reloader(vec![Script::Good, broken(Kind::Syntax)], accept_all());
        reloader.reload_now();
        let report = reloader.recover(GenerationFault::Budget);
        assert!(!report.ok);
        assert_eq!(
            report.errors[0].kind,
            Some(Kind::Syntax),
            "the files on disk would fail the same way"
        );
        assert_eq!(report.generation, Some(1));
    }

    // ---- the rollback mark ------------------------------------------------------------------------------------------

    #[test]
    fn an_abandoned_stage_hands_its_mark_back_once_and_a_committed_one_forgets_it() {
        let mut slot = RollbackSlot::default();
        assert!(!slot.is_open());
        assert_eq!(slot.abandon(), None::<u32>, "nothing to restore when no stage is open");
        slot.begin(7_u32);
        assert!(slot.is_open());
        assert_eq!(slot.abandon(), Some(7));
        assert_eq!(slot.abandon(), None, "the mark is restored once");
        slot.begin(8);
        slot.commit();
        assert!(!slot.is_open());
        assert_eq!(slot.abandon(), None, "a committed stage has nothing to roll back");
    }

    // ---- the swap ---------------------------------------------------------------------------------------------------

    /// Records the order the steps run in, and what each was given.
    struct Recorder {
        log: Rc<RefCell<Vec<String>>>,
        snapshot: Option<StepError<&'static str>>,
        end_old: Option<StepError<&'static str>>,
        exchange: Option<&'static str>,
        take_over: Option<StepError<&'static str>>,
    }

    impl Recorder {
        fn new() -> (Self, Rc<RefCell<Vec<String>>>) {
            let log = Rc::new(RefCell::new(Vec::new()));
            (
                Self {
                    log: Rc::clone(&log),
                    snapshot: None,
                    end_old: None,
                    exchange: None,
                    take_over: None,
                },
                log,
            )
        }

        fn note(&self, step: &str) {
            self.log.borrow_mut().push(step.to_owned());
        }
    }

    impl SwapHost for Recorder {
        type Generation = &'static str;
        type Snapshot = &'static str;
        type Claims = Vec<&'static str>;
        type Fatal = &'static str;

        fn snapshot(&mut self) -> Result<&'static str, StepError<&'static str>> {
            self.note("snapshot");
            self.snapshot.take().map_or(Ok("old-world"), Err)
        }

        fn start_programs(&mut self, generation: &&'static str) -> Vec<String> {
            self.note(&format!("start-programs of {generation}"));
            vec!["a program did not start".to_owned()]
        }

        fn take_claims(&mut self) -> Vec<&'static str> {
            self.note("take-claims");
            vec!["Super+1", "Super+2"]
        }

        fn end_old(&mut self) -> Result<Vec<String>, StepError<&'static str>> {
            self.note("end-old");
            self.end_old
                .take()
                .map_or_else(|| Ok(vec!["old said bye".to_owned()]), Err)
        }

        fn exchange_claims(&mut self, staged: Vec<&'static str>) -> Result<(), &'static str> {
            self.note(&format!("release-all then send {}", staged.join("+")));
            self.exchange.map_or(Ok(()), Err)
        }

        fn install(&mut self, generation: &'static str) {
            self.note(&format!("install {generation}"));
        }

        fn take_over(&mut self, previous: Option<&&'static str>) -> Result<(), StepError<&'static str>> {
            self.note(&format!("take-over from {}", previous.copied().unwrap_or("nothing")));
            self.take_over.take().map_or(Ok(()), Err)
        }

        fn finish(&mut self, diagnostics: &mut Vec<String>) -> Result<(), &'static str> {
            self.note("finish");
            diagnostics.push("document applied".to_owned());
            Ok(())
        }
    }

    #[test]
    fn a_swap_releases_the_old_claims_before_the_new_ones_go_out_and_installs_after_both() {
        let (mut host, log) = Recorder::new();
        let diagnostics = swap(&mut host, "gen2").expect("the swap runs");
        assert_eq!(
            *log.borrow(),
            [
                "snapshot",
                "start-programs of gen2",
                "take-claims",
                "end-old",
                "release-all then send Super+1+Super+2",
                "install gen2",
                "take-over from old-world",
                "finish",
            ]
        );
        assert_eq!(
            diagnostics,
            ["a program did not start", "old said bye", "document applied"],
            "what the steps reported, in the order they ran"
        );
    }

    #[test]
    fn a_failure_inside_the_old_or_the_new_generation_is_a_diagnostic_and_the_swap_happens() {
        let (mut host, log) = Recorder::new();
        host.snapshot = Some(StepError::Recovered("the old workspaces could not be read".to_owned()));
        host.end_old = Some(StepError::Recovered(
            "the old generation did not end cleanly".to_owned(),
        ));
        host.take_over = Some(StepError::Recovered(
            "the new generation could not take over".to_owned(),
        ));
        let diagnostics = swap(&mut host, "gen2").expect("recovered steps do not stop the swap");
        assert!(log.borrow().contains(&"install gen2".to_owned()), "{:?}", log.borrow());
        assert!(
            log.borrow().contains(&"take-over from nothing".to_owned()),
            "no snapshot, no previous"
        );
        assert_eq!(
            diagnostics,
            [
                "the old workspaces could not be read",
                "a program did not start",
                "the old generation did not end cleanly",
                "the new generation could not take over",
                "document applied",
            ]
        );
    }

    #[test]
    fn a_dead_link_stops_the_swap_where_it_happened() {
        let (mut host, log) = Recorder::new();
        host.exchange = Some("the link is gone");
        assert_eq!(swap(&mut host, "gen2").unwrap_err(), "the link is gone");
        assert!(
            !log.borrow().iter().any(|step| step.starts_with("install")),
            "a generation is not installed after the claims failed to go out: {:?}",
            log.borrow()
        );

        let (mut host, log) = Recorder::new();
        host.snapshot = Some(StepError::Fatal("the link died reading the snapshot"));
        assert_eq!(
            swap(&mut host, "gen2").unwrap_err(),
            "the link died reading the snapshot"
        );
        assert_eq!(*log.borrow(), ["snapshot"], "nothing ran after a fatal first step");
    }

    #[test]
    fn the_entry_path_is_the_root_joined_with_a_relative_entry() {
        assert_eq!(
            entry_path(Path::new("/c"), Path::new("init.ts")),
            PathBuf::from("/c/init.ts")
        );
        assert_eq!(
            entry_path(Path::new("/c"), Path::new("/elsewhere/shell.py")),
            PathBuf::from("/elsewhere/shell.py")
        );
    }
}
