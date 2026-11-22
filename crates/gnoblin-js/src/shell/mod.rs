//! The JavaScript shell: what `ShellVm` is for the retired shell, over the same tree link.
//!
//! [`JsShell`] attaches to the compositor's tree, loads the embedded `gnoblin-core/host` module, evaluates the
//! user's entry module, and then runs one [`JsShell::frame`] per turn of the runtime loop: it applies the
//! compositor's batch to the mirror, tells the script's listeners what changed, and sends everything the
//! script wrote as one transaction.
//!
//! The protocol rules are the ones `the retired retired host`'s `ShellVm` keeps, because they belong to the tree
//! protocol and not to the language: one transaction in flight (a second one built on the same base is
//! refused as stale), a refusal matched to its transaction by position, a `stale-base` or `unknown-node`
//! refusal sent again ahead of the next frame's writes, at most [`STALE_RETRIES`] times in a row. A refusal
//! applies nothing, so what is sent again is the whole transaction, every kind of op, and a refusal that names no
//! op (the base is older than the compositor's history) sends the whole frame again. What cannot be sent again
//! (a call whose text is a secret) is answered as failed, and a write given up on is handed back to the layout.
//! A refusal no newer base cures (`read-only`, `not-finite`, `too-large`, ...) gives up the op it names and sends
//! the others again, so one bad write costs a frame that write and nothing else; a call that is given up is answered
//! as failed too, and the script reads what the compositor holds again.
//!
//! The boot order is: attach and drain the snapshot; load the `gnoblin` module; evaluate the entry
//! ([`JsShell::load_entry`]); announce `ready` for the snapshot's nodes, parents first, on the first
//! frame; then the frame loop.
//!
//! Every entry into script code has a wall-clock budget: each listener call on its own
//! ([`JsShellConfig::entry_budget`]) and the frame's dispatch as a whole ([`JsShellConfig::frame_budget`]).
//! A listener that throws, or runs out its budget, is reported by name and the frame goes on with the
//! next one. A budget trip is a strike against the generation ([`JsShell::strikes`]); what to do about
//! strikes (restart the generation) is the supervisor's call.
//!
//! The module is split by concern. This file holds the types, the boot and the generation swap; `frame` runs one
//! frame, `gate` decides whether a frame has anything to do, `txgate` is the transaction gate (one in flight, a
//! refusal sent again), `links` holds the links beside the tree (dialogs, idle, session, host API) and `document`
//! the configuration document.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gnoblin_tree_client::gate::TxGate;
use gnoblin_tree_client::session::{ATTACH_VERSION, BOOT_WAIT, SessionError, boot_mirror};
use gnoblin_tree_client::tree_mirror::{NodeId, TreeIncoming, TreeLink, TreeMirror};
use rquickjs::convert::Coerced;
use rquickjs::{FromJs, Function, Module, Object, Value};
use thiserror::Error;

use gnoblin_shell_host::stage::{RollbackSlot, StepError, SwapHost, swap};
use gnoblin_shell_host::supervisor::{Clock, SystemClock};

use crate::bindings::{HostState, SharedState, StageMark};
use crate::budget::{ENTRY_BUDGET, FRAME_BUDGET};
use crate::choice::{Choice, StockRuntime, choose, choose_source};
use crate::dialogs::JsDialogs;
use crate::error::{JsError, JsErrorKind};
use crate::generation::{JsConfig, JsGeneration};
use crate::host_idle::JsIdle;
use crate::stock::{HOST_MODULE_NAME, STARTER_MODULE_NAME, StockLibrary, starter_module};
use crate::workspaces::JsWorkspaceSnapshot;
use crate::{bindings, input};

mod document;
mod frame;
mod gate;
mod links;
mod txgate;

pub use self::gate::JsFrameStats;

gnoblin_log::logger!(log = "js");

/// The most promise jobs one frame's pump runs, so that a chain of jobs that schedules itself cannot hold the frame.
const MAX_JOBS_PER_PUMP: usize = 1000;

pub use gnoblin_tree_client::gate::STALE_RETRIES;

/// The least time the embedded library's install may use, whatever the config's `stage_budget`: the bundled modules
/// are compiled in that call, and a budget chosen to bound a user's entry must not starve them.
const HOST_INSTALL_BUDGET: Duration = Duration::from_secs(5);

/// The failure of reading the stock library, as the failure of a boot: the shell cannot run without it.
fn stock_failure(error: crate::stock::StockError) -> JsShellError {
    JsShellError::Js(JsError {
        kind: JsErrorKind::Exception,
        message: error.to_string(),
        location: None,
        stack: None,
    })
}

/// What a shell is built from.
#[derive(Debug, Clone)]
pub struct JsShellConfig {
    /// The generation's config: root, memory limit, stack limit and the budget of loading modules.
    pub js: JsConfig,
    /// What one listener call may use. Default [`ENTRY_BUDGET`].
    pub entry_budget: Duration,
    /// What one frame's dispatch may use, all listener calls together. Default [`FRAME_BUDGET`].
    pub frame_budget: Duration,
    /// The directories the stock library (`gnoblin-core`, `gnoblin-std`) is read from, in order, each laid out as `data/js`: a
    /// package is the one of the first directory that holds it. The library is installed data and nothing is compiled in, so a
    /// shell with no directory here has no `gnoblin-core` to import and cannot boot. The runtime finds them with
    /// [`stock_search_path`](crate::stock_search_path).
    pub stock_dirs: Vec<PathBuf>,
    /// The starter, the file the stock desktop is: a shell with no user entry, or whose entry could not load, runs it in the entry's
    /// place ([`JsShell::load_default_config`]). `None` runs nothing in its place, and a failed entry is the failure of the load.
    pub starter: Option<PathBuf>,
    /// Whether the session declared XR (`GNOBLIN_XR=1`). A config may then import `gnoblin-std/extensions/xr` before the
    /// headset's session has started; without it that import works only once the program's `xr` seat is in the tree.
    pub xr_declared: bool,
    /// The monotonic clock the scripts read (`gnoblin.timers.now`, and so every timer, fade, delay and arming window).
    /// The real clock by default; a test that must not depend on how fast the machine is gives it a
    /// [`ManualClock`](crate::ManualClock) and moves it.
    pub clock: Arc<dyn Clock>,
}

impl JsShellConfig {
    /// The product budgets of `crate::budget`: a listener call, a frame and a module load each have a limit set well
    /// above what a loaded machine needs.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            js: JsConfig::new(root),
            entry_budget: ENTRY_BUDGET,
            frame_budget: FRAME_BUDGET,
            stock_dirs: Vec::new(),
            starter: None,
            xr_declared: false,
            clock: Arc::new(SystemClock),
        }
    }

    /// Reads the stock library from `stock_dir` (laid out as `data/js`), and takes the `shells/default/shell.ts` of the source tree
    /// it sits in as the starter. An installed library has no starter: the packaged default shell is a shell of its own
    /// (`share/gnoblin/default-shell/shell.ts`), not a fallback of the engine.
    #[must_use]
    pub fn with_stock(mut self, stock_dir: impl Into<PathBuf>) -> Self {
        let stock_dir = stock_dir.into();
        self.starter = crate::stock::source_starter(&stock_dir);
        self.stock_dirs = vec![stock_dir];
        self
    }
}

/// Why the shell could not boot or run a frame.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JsShellError {
    /// The link to the compositor failed or sent something the shell cannot take.
    #[error("shell link: {0}")]
    Link(String),
    /// The mirror refused a snapshot or a change batch.
    #[error("shell mirror: {0}")]
    Mirror(String),
    /// The compositor did not answer the attach in time.
    #[error("the compositor did not answer the attach")]
    NoAttach,
    /// The snapshot did not complete in time.
    #[error("the snapshot did not complete")]
    IncompleteSnapshot,
    /// The engine or the shell's own module failed. A listener's failure is a diagnostic, not this.
    #[error(transparent)]
    Js(#[from] JsError),
}

/// What one frame produced for the caller.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct JsFrame {
    /// Listener failures (once per site), refused transactions and handlers that ran past their budget,
    /// in order.
    pub diagnostics: Vec<String>,
}

/// What a reload needs from a running shell to stage its next generation: the config the shell's own
/// generation was built from, and the hook that gives a staged generation the shell's host objects.
///
/// A generation staged through it reads and writes the same tree mirror as the running one, so the windows
/// keep their ids. Cheap to clone; it holds the shell's shared state and no script value.
#[derive(Clone)]
pub struct JsStaging {
    js: JsConfig,
    state: SharedState,
}

impl JsStaging {
    /// The config to stage generations from: the shell's, with the stock library and `gnoblin-core/host` embedded.
    pub fn config(&self) -> &JsConfig {
        &self.js
    }

    /// Installs the host objects in `generation` and loads `gnoblin-core/host`, so `gnoblin.clients` is there when
    /// the entry module runs. Run it on a fresh generation, before the entry. A restage inside the same stage
    /// starts its spawns over: what an interrupted generation spawned does not count for the one that replaces it.
    pub fn prepare(&self, generation: &JsGeneration) -> Result<(), JsError> {
        self.state.borrow_mut().api.processes.restage();
        install_host(generation, &self.state, &self.js, self.js.stage_budget)
    }
}

/// The shell's JavaScript runtime over the real tree.
pub struct JsShell {
    link: Box<dyn TreeLink>,
    generation: JsGeneration,
    /// The config generations are built from, with the stock library and the host module embedded.
    js: JsConfig,
    state: SharedState,
    /// What the shell held when a stage began, kept until that stage is swapped in or abandoned so a stage that fails
    /// after writing, spawning or making a canvas leaves nothing behind.
    stage: RollbackSlot<StageMark>,
    entry_budget: Duration,
    frame_budget: Duration,
    /// Messages that arrived with the boot snapshot, handled by the first frame.
    incoming: Vec<TreeIncoming>,
    /// The snapshot's nodes, parents first, to announce `ready` for on the first frame.
    boot_ready: Option<Vec<NodeId>>,
    /// The transaction gate: one transaction in flight, its reply matched to it by position, a refused transaction's writes
    /// sent again (the tree client's, shared with every shell runtime).
    tx: TxGate,
    /// How many times each failure site was seen. The first is reported, the rest only counted.
    reported: HashMap<String, u64>,
    /// How many times a call ran past its wall-clock or memory budget in this generation.
    strikes: u32,
    /// How many queued listener calls ran (`dispatches`).
    dispatches: u64,
    /// The shell's connection for slots, and the observation of the compositor's own dialogs on it.
    dialogs: JsDialogs,
    /// The shell's connection for the compositor's idle state (`gnoblin.idle`), opened while a script wants it.
    idle: JsIdle,
    /// The `documentRevision` the entry reported when the document was last committed. A device setting written
    /// after that moves it, and the next frame commits the document again.
    document_revision: u32,
    /// What decides whether a frame has anything to do (see `gate`).
    gate: gate::FrameGate,
    /// Whether the shell was given a starter to run when it has no entry ([`JsShellConfig::starter`]).
    has_starter: bool,
}

impl JsShell {
    /// Attaches to the tree and loads the `gnoblin-core/host` module, so `gnoblin.clients` is there for the entry
    /// module. The stock library is embedded beside it, and `gnoblin` re-exports both. Fails when the link
    /// does not deliver a complete snapshot within five seconds.
    pub fn new(mut link: Box<dyn TreeLink>, config: JsShellConfig) -> Result<Self, JsShellError> {
        link.attach(ATTACH_VERSION).map_err(|e| JsShellError::Link(e.0))?;
        let (mirror, incoming) = boot(link.as_mut())?;
        let boot_ready = bindings::parents_first(&mirror);
        let state: SharedState = Rc::new(RefCell::new(HostState::with_clock(mirror, Arc::clone(&config.clock))));
        state.borrow_mut().xr = crate::xr::JsXrState::declared(config.xr_declared);

        let mut js = config.js.clone();
        // The console's rate limit counts on the clock the scripts read, so a test that moves it moves that too.
        js.clock = Arc::clone(&config.clock);
        // The stock library is chosen for each generation by the import graph of its entry (`choice`): the native library, which
        // is embedded, or the TypeScript one, which is read from the directories when a shell needs it. A module the caller
        // embedded under a name stays. The stock desktop is the starter file itself, which a shell with no user entry runs as
        // its entry.
        let starter = match &config.starter {
            Some(path) => Some(starter_module(path).map_err(stock_failure)?),
            None => None,
        };
        let has_starter = starter.is_some();
        js.stock_dirs = Arc::new(config.stock_dirs.clone());
        js.stock = Some(Arc::new(StockLibrary::new(config.stock_dirs.clone(), starter)));
        // Generation 0 is the bare one (no user code); the reloader numbers the ones it stages from 1. It starts on the TypeScript
        // library, which every shell ran on before the native one existed, and is built again on the native library when the entry
        // loaded into it needs that.
        let generation = JsGeneration::new(&js.for_runtime(StockRuntime::Typescript)?, 0)?;
        install_host(&generation, &state, &js, js.stage_budget)?;

        Ok(Self {
            link,
            generation,
            js,
            state,
            stage: RollbackSlot::default(),
            entry_budget: config.entry_budget,
            frame_budget: config.frame_budget,
            incoming,
            boot_ready: Some(boot_ready),
            tx: TxGate::new(),
            reported: HashMap::new(),
            strikes: 0,
            dispatches: 0,
            dialogs: JsDialogs::new(),
            idle: JsIdle::new(),
            document_revision: 0,
            gate: gate::FrameGate::default(),
            has_starter,
        })
    }

    /// Whether the running generation is on the TypeScript library, whose object model the engine drives with the frame's
    /// stages. A generation on the native library has none: the core serves the objects and calls the script back.
    pub(super) fn legacy(&self) -> bool {
        self.generation.runtime() == StockRuntime::Typescript
    }

    /// Makes the running generation, which has run nothing of the user's yet, one on the library `choice` names, and records the
    /// library as the one the shell runs on. A choice that cannot be made (the graph could not be read) leaves the generation as it
    /// is: the load that follows fails the same way on either library.
    fn use_library_for(&mut self, choice: Choice) -> Result<(), JsError> {
        let wanted = match choice {
            Choice::Typescript(_) => StockRuntime::Typescript,
            Choice::Generated => StockRuntime::Generated,
            Choice::Unknown => self.generation.runtime(),
        };
        if wanted != self.generation.runtime() {
            let generation = JsGeneration::new(&self.js.for_runtime(wanted)?, self.generation.id())?;
            install_host(&generation, &self.state, &self.js, self.js.stage_budget)?;
            self.generation = generation;
        }
        if let Some(stock) = &self.js.stock {
            stock.note_serving(wanted);
        }
        Ok(())
    }

    /// What a reload stages its generations from. See [`JsStaging`].
    pub fn staging(&self) -> JsStaging {
        JsStaging {
            js: self.js.clone(),
            state: Rc::clone(&self.state),
        }
    }

    /// Marks the start of a stage. A generation staged through [`JsStaging`] evaluates the user's entry against
    /// the live tree, so a top-level write such as `client.x = 5` lands in the shell's pending writes before it
    /// is known that the stage succeeds. The mark is what [`abandon_stage`](Self::abandon_stage) returns them to,
    /// with the toolkit's canvases, the XR nodes, the output applies and the lock checks the entry made. The seat
    /// claims the staged generation makes are kept apart for the same reason: they are sent only when the
    /// generation takes over, after the old one's have been released. So are the programs it spawns.
    pub fn begin_stage(&mut self) {
        // The staged generation's entry runs against the live tree, so it is script that runs between frames.
        self.note_script_ran();
        let mark = self.state.borrow_mut().begin_stage();
        self.stage.begin(mark);
    }

    /// Ends a stage that failed: the pending writes go back to what they were at [`begin_stage`](Self::begin_stage),
    /// the claims it kept are dropped, and the programs it spawned never start. The running generation was never
    /// touched.
    pub fn abandon_stage(&mut self) {
        self.note_script_ran();
        if let Some(mark) = self.stage.abandon() {
            self.state.borrow_mut().abandon_stage(mark);
        }
    }

    /// Replaces the running generation with a staged one, after [`begin_stage`](Self::begin_stage).
    ///
    /// The tree is the compositor's and the mirror is shared, so every window keeps its id and its place: nothing
    /// is re-created or moved. The order is the one the input and workspace lanes wrote their hooks for: the old
    /// generation's workspace snapshot is read, then it ends (its drags are cancelled, its seat claims and the
    /// dialog observation's requests are given back), then the new generation's claims are sent behind the old
    /// ones' releases in the same transaction, so a chord is never refused for being held and never held by nobody,
    /// and the new generation takes over. Its dialog slot, if it has a listener, is the one the old generation
    /// held, and is not released and claimed again. It has run its entry (a
    /// listener it added was told of the windows already there), so it is finished like a loaded entry
    /// (`finishLoad`: a config that declared no workspace gets the stock one) and the windows go back to the
    /// workspaces of the same id ([`reconcile`](Self::reconcile)), without being offered to the layout as
    /// arrivals. It is announced `ready` for every window the tree holds on the next frame, as at boot. The old
    /// generation is dropped with its listeners, queued calls and error reports, and strikes start again at zero.
    ///
    /// The programs the staged config spawned at its top level start now, unless the old config started the same
    /// program already, in which case the new generation takes that process over; what the old config started and
    /// the new one does not is stopped (`Processes::end_stage`).
    ///
    /// A script failure while the old generation ends or the new one finishes is a diagnostic in the result, not
    /// an error: the swap has happened.
    ///
    /// # Errors
    ///
    /// The link failed or the mirror refused a batch; the shell has ended.
    pub fn swap_generation(&mut self, generation: JsGeneration) -> Result<Vec<String>, JsShellError> {
        self.note_script_ran();
        swap(&mut ShellSwap { shell: self }, generation)
    }

    /// Finishes a loaded entry: a config that declared no workspace gets the stock one now, so
    /// `gnoblin.workspaces` is never empty for what runs after the entry.
    fn finish_load(&mut self) -> Result<(), JsShellError> {
        if !self.legacy() {
            return Ok(());
        }
        self.call_entry(self.entry_budget, |entry| {
            entry.get::<_, Function<'_>>("finishLoad")?.call::<_, ()>(())
        })
    }

    /// The id of the generation that runs.
    pub fn generation_id(&self) -> u64 {
        self.generation.id()
    }

    /// Runs `f` in the running generation under the listener budget, as a listener would run. A library that calls script back from
    /// outside a frame (the std library's layouts and listeners, driven by the core's dispatch) enters the generation this way.
    ///
    /// # Errors
    ///
    /// The script threw, or ran past its budget.
    pub fn call_script<R>(
        &self,
        f: impl for<'js> FnOnce(&rquickjs::Ctx<'js>) -> rquickjs::Result<R>,
    ) -> Result<R, JsError> {
        self.note_script_ran();
        self.generation.call_budgeted(self.entry_budget, f)
    }

    /// The name of every embedded module the running generation loaded, sorted: the import log of the stock library.
    /// A shell whose entry imports only `gnoblin-core` has no `gnoblin-std` module in it.
    pub fn embedded_modules(&self) -> Vec<String> {
        self.generation.embedded_modules()
    }

    /// The running generation and the budget one call of it has, for the console: a console expression runs in the
    /// shell's own generation under the listeners' budget, and a trip of it is not a strike (see `console`).
    pub(crate) fn console_target(&self) -> (&JsGeneration, Duration) {
        // A console expression is script that runs between frames: what it queued or wrote is for the next frame.
        self.note_script_ran();
        (&self.generation, self.entry_budget)
    }

    /// How many times a call ran past its wall-clock or memory budget in this generation.
    pub const fn strikes(&self) -> u32 {
        self.strikes
    }

    /// How many queued listener calls the frames have run in all: the work a script's handlers did. A transition the
    /// compositor plays on its own runs none between the commit and its end, which is what this counts.
    pub const fn dispatches(&self) -> u64 {
        self.dispatches
    }

    /// Evaluates the user's entry module, a path under the config root. Returns the failures of listener
    /// calls that ran while it loaded (a replay on `addListener`). An error loading the module is `Err`,
    /// and counts a strike when it is a budget trip.
    pub fn load_entry(&mut self, entry: &Path) -> Result<Vec<String>, JsShellError> {
        self.note_script_ran();
        self.use_library_for(choose(&self.js.root, entry))?;
        let loaded = self.generation.eval_module(entry);
        self.finish_entry(loaded)
    }

    /// Runs the stock desktop as the generation's whole config, for a shell with no user entry (or whose entry could
    /// not load): the starter file ([`JsShellConfig::starter`], `shells/default/shell.ts`, the text `gnoblinctl config init`
    /// writes) is the entry, so the default is a user's `shell.ts` that nobody wrote. Returns the failures of calls that ran
    /// while it loaded.
    ///
    /// # Errors
    ///
    /// The shell has no starter, the starter threw (a bug in the library) or the link failed.
    pub fn load_default_config(&mut self) -> Result<Vec<String>, JsShellError> {
        if !self.has_starter {
            return Err(JsShellError::Js(JsError {
                kind: JsErrorKind::Exception,
                message: "the shell has no starter to run in the place of an entry".to_owned(),
                location: None,
                stack: None,
            }));
        }
        // A load, like `load_entry`, so it has the stage budget: it evaluates the whole starter, which took as long as
        // any config under load, and a listener call's budget would interrupt it.
        self.note_script_ran();
        let choice = match self.js.stock.as_ref().and_then(|stock| stock.starter()) {
            Some(starter) => choose_source(&self.js.root, &starter.path, &starter.source),
            None => Choice::Unknown,
        };
        self.use_library_for(choice)?;
        let loaded = self.generation.eval_specifier(STARTER_MODULE_NAME.to_owned());
        self.finish_entry(loaded)
    }

    /// What follows the evaluation of an entry module, whichever module it was: a failed load counts a strike when it
    /// is a budget trip; a good one lets the windows that were there at boot join a workspace on the first frame, then
    /// applies the document and collects the failures of the listener calls that ran.
    fn finish_entry(&mut self, loaded: Result<(), JsError>) -> Result<Vec<String>, JsShellError> {
        if let Err(error) = loaded {
            self.count_strike(&error);
            return Err(error.into());
        }
        self.finish_load()?;
        let mut diagnostics = Vec::new();
        self.apply_document(&mut diagnostics, true)?;
        self.collect_reports(&mut diagnostics)?;
        Ok(diagnostics)
    }

    /// Which window is on which workspace and which workspace is shown, to hand to the shell of the next
    /// generation. Workspaces are script objects, so the compositor cannot say.
    pub fn workspace_snapshot(&mut self) -> Result<JsWorkspaceSnapshot, JsShellError> {
        if !self.legacy() {
            return Ok(JsWorkspaceSnapshot::default());
        }
        self.call_entry(self.entry_budget, |entry| {
            let snapshot: Object<'_> = entry.get::<_, Function<'_>>("snapshot")?.call(())?;
            JsWorkspaceSnapshot::from_js(&snapshot)
        })
    }

    /// Puts the windows back where `previous` had them, by workspace id, after the entry has loaded: a window
    /// stays on the workspace of that id when this generation declares one (and is not placed again), and
    /// moves to the first workspace when it does not. The shown workspace is kept when it still exists.
    /// Windows `previous` did not know join a workspace by the initial-workspace rule on the first frame.
    pub fn reconcile(&mut self, previous: &JsWorkspaceSnapshot) -> Result<(), JsShellError> {
        self.note_script_ran();
        if !self.legacy() {
            return Ok(());
        }
        self.call_entry(self.entry_budget, |entry| {
            let previous = previous.to_js(entry.ctx())?;
            entry.get::<_, Function<'_>>("reconcile")?.call::<_, ()>((previous,))
        })
    }

    /// Evaluates `source` as a script in the generation and returns its value as text, or `None` for
    /// `undefined`. For tests and a console; it runs under the listener budget.
    pub fn eval(&mut self, source: &str) -> Result<Option<String>, JsShellError> {
        self.note_script_ran();
        let outcome = self.generation.call_budgeted(self.entry_budget, |ctx| {
            let value: Value<'_> = ctx.eval(source)?;
            if value.is_undefined() {
                return Ok(None);
            }
            Coerced::<String>::from_js(ctx, value).map(|text| Some(text.0))
        });
        outcome.map_err(|error| {
            self.count_strike(&error);
            error.into()
        })
    }

    /// Ends the generation's hold on input: every drag and gesture a binding is running gets its `onCancel`
    /// once, under the call budget, and every seat claim is released, so no pointer or chord stays captured
    /// by a generation that is going away. The caller runs this before it drops the shell (a shutdown ends it;
    /// a reload swaps generations through [`swap_generation`](Self::swap_generation), which keeps what the next
    /// generation takes over). The `unclaim` calls travel in the transaction this
    /// sends; when an earlier transaction is still in flight they wait for the next [`JsShell::frame`]. The
    /// compositor releases a connection's claims by itself when the shell dies, so a crash cannot leave a
    /// chord claimed.
    pub fn retire(&mut self) -> Result<JsFrame, JsShellError> {
        self.note_script_ran();
        let frame = self.end_generation(false)?;
        self.send_transaction()?;
        Ok(frame)
    }

    /// What [`retire`](Self::retire) does short of sending the transaction. `keep_slots` leaves the `dialogs.observe`
    /// claim with the shell instead of releasing it: a reload keeps it for the generation that takes over, which
    /// releases it itself in its first frame if it has no dialog listener, so the compositor never sees a moment in
    /// which nobody observes its dialogs.
    fn end_generation(&mut self, keep_slots: bool) -> Result<JsFrame, JsShellError> {
        let mut diagnostics = Vec::new();
        if !self.legacy() {
            // The core ends what a generation of the native library holds (its claims, its drags, its dialogs) when the
            // generation closes; there is nothing of the engine's to give back.
            self.generation.flush_console();
            return Ok(JsFrame { diagnostics });
        }
        let pending: usize = self.call_entry(self.frame_budget, |entry| {
            entry.get::<_, Function<'_>>("cancelAll")?.call(())
        })?;
        let started = Instant::now();
        self.state.borrow_mut().input.inbox.clear();
        self.state.borrow_mut().input.forget_window_events();
        self.run_pending(pending, started, &mut diagnostics)?;
        if keep_slots {
            self.dialogs.hand_over();
        } else {
            self.dialogs.retire();
        }
        let holders: Vec<String> = self.call_entry(self.entry_budget, |entry| {
            entry.get::<_, Function<'_>>("idleHolders")?.call(())
        })?;
        self.idle.retire(&holders);
        self.collect_reports(&mut diagnostics)?;
        self.generation.flush_console();
        Ok(JsFrame { diagnostics })
    }
}

/// The steps of a swap in this shell's terms. The order they run in, and what a failure of each one means, is the shell
/// host's protocol (`gnoblin_shell_host::stage::swap`); this says what each step does to the JavaScript shell. A step that
/// fails inside script counts a strike and becomes a diagnostic.
struct ShellSwap<'a> {
    shell: &'a mut JsShell,
}

impl SwapHost for ShellSwap<'_> {
    type Generation = JsGeneration;
    type Snapshot = JsWorkspaceSnapshot;
    type Claims = Vec<input::StagedClaim>;
    type Fatal = JsShellError;

    fn snapshot(&mut self) -> Result<JsWorkspaceSnapshot, StepError<JsShellError>> {
        match self.shell.workspace_snapshot() {
            Ok(snapshot) => Ok(snapshot),
            Err(JsShellError::Js(error)) => {
                self.shell.count_strike(&error);
                Err(StepError::Recovered(format!(
                    "the old generation's workspaces could not be read, so windows rejoin a workspace by the stock \
                     rule: {error}"
                )))
            }
            Err(other) => Err(StepError::Fatal(other)),
        }
    }

    fn start_programs(&mut self, generation: &JsGeneration) -> Vec<String> {
        // The new generation's programs start now, before the old one ends: what the old one's end handlers spawn is
        // then a handler's spawn, started at once, and not one that waits for a takeover that is already under way.
        let unstarted = self.shell.state.borrow_mut().api.processes.end_stage(generation.id());
        unstarted
            .into_iter()
            .map(|(id, error)| format!("the program of process {id} did not start: {error}"))
            .collect()
    }

    fn take_claims(&mut self) -> Vec<input::StagedClaim> {
        // The new generation's claims were kept while it was staged.
        input::end_staging(&mut self.shell.state.borrow_mut())
    }

    fn end_old(&mut self) -> Result<Vec<String>, StepError<JsShellError>> {
        match self.shell.end_generation(true) {
            Ok(frame) => Ok(frame.diagnostics),
            Err(JsShellError::Js(error)) => {
                self.shell.count_strike(&error);
                Err(StepError::Recovered(format!(
                    "the old generation did not end cleanly: {error}"
                )))
            }
            Err(other) => Err(StepError::Fatal(other)),
        }
    }

    fn exchange_claims(&mut self, staged: Vec<input::StagedClaim>) -> Result<(), JsShellError> {
        // The new claims go out behind everything the old one gives up, in one transaction: the compositor applies a
        // transaction's calls in order, so each chord is free when its new claim arrives. A claim the old generation did
        // not release (its script threw first) is released here, or its chord would be refused to the new generation.
        {
            let mut state = self.shell.state.borrow_mut();
            input::release_all(&mut state);
            input::send_staged(&mut state, staged);
        }
        self.shell.send_transaction()
    }

    fn install(&mut self, generation: JsGeneration) {
        self.shell.stage.commit();
        if let Some(stock) = &self.shell.js.stock {
            stock.note_serving(generation.runtime());
        }
        self.shell.generation = generation;
        self.shell.strikes = 0;
        self.shell.reported.clear();
        self.shell.boot_ready = Some(bindings::parents_first(&self.shell.state.borrow().mirror));
    }

    fn take_over(&mut self, previous: Option<&JsWorkspaceSnapshot>) -> Result<(), StepError<JsShellError>> {
        let finished = self.shell.finish_load().and_then(|()| match previous {
            Some(previous) => self.shell.reconcile(previous),
            None => Ok(()),
        });
        match finished {
            Ok(()) => Ok(()),
            Err(JsShellError::Js(error)) => {
                self.shell.count_strike(&error);
                Err(StepError::Recovered(format!(
                    "the new generation could not take over the workspaces: {error}"
                )))
            }
            Err(other) => Err(StepError::Fatal(other)),
        }
    }

    fn finish(&mut self, diagnostics: &mut Vec<String>) -> Result<(), JsShellError> {
        self.shell.apply_document(diagnostics, true)?;
        self.shell.collect_reports(diagnostics)
    }
}

/// Installs the host functions in `generation` and, for a generation on the TypeScript library, loads `gnoblin-core/host`, under
/// `budget` or [`HOST_INSTALL_BUDGET`], whichever is longer. A generation on the native library has the objects of the libraries
/// (`config.natives`) and nothing of the engine's own object model: no `__gnoblinHost`, no host module.
fn install_host(
    generation: &JsGeneration,
    state: &SharedState,
    config: &JsConfig,
    budget: Duration,
) -> Result<(), JsError> {
    let legacy = generation.runtime() == StockRuntime::Typescript;
    generation.call_budgeted(budget.max(HOST_INSTALL_BUDGET), |ctx| {
        if legacy {
            bindings::install(ctx, state)?;
        }
        let ids = crate::natives::ScriptIds::new(state);
        for native in &config.natives {
            native.install(ctx, &ids)?;
        }
        if legacy {
            Module::import(ctx, HOST_MODULE_NAME)?.finish::<Value<'_>>().map(drop)?;
        }
        Ok(())
    })
}

/// Drains the link until the complete snapshot has arrived, building the mirror from it (the tree client's boot, shared
/// with every shell runtime). Anything else that arrives meanwhile is kept for the first frame.
fn boot(link: &mut dyn TreeLink) -> Result<(TreeMirror, Vec<TreeIncoming>), JsShellError> {
    boot_mirror(link, BOOT_WAIT).map_err(|error| match error {
        SessionError::Link(why) => JsShellError::Link(why),
        SessionError::Mirror(why) => JsShellError::Mirror(why),
        SessionError::NoAttach => JsShellError::NoAttach,
        SessionError::IncompleteSnapshot => JsShellError::IncompleteSnapshot,
    })
}
