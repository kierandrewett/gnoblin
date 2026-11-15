//! The JavaScript shell: what `ShellVm` is for the retired shell, over the same tree link.
//!
//! [`JsShell`] attaches to the compositor's tree, loads the embedded `gnoblin/host` module, evaluates the
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
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gnoblin_tree_client::tree_mirror::{NodeId, SnapshotNode, TreeIncoming, TreeLink, TreeMirror, TxBuilder, TxOp};
use rquickjs::convert::Coerced;
use rquickjs::{FromJs, Function, Module, Object, Value};
use thiserror::Error;

use self::txgate::Sent;
use crate::bindings::{HostState, SharedState};
use crate::budget::{ENTRY_BUDGET, FRAME_BUDGET};
use crate::dialogs::JsDialogs;
use crate::error::JsError;
use crate::generation::{JsConfig, JsGeneration};
use crate::host_idle::JsIdle;
use crate::stock::{HOST_MODULE_NAME, host_export, stock_sources_in};
use crate::supervisor::{JsClock, JsSystemClock};
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

/// How many times a write the compositor refused `stale-base` is sent again before it is dropped: the
/// number `the retired retired host`'s shell keeps. A node the compositor changes before every landing would refuse the
/// write for ever, and the writes beside it, so after this many refusals in a row the refused write is
/// dropped and the transaction's other writes go on.
pub const STALE_RETRIES: u32 = 3;

/// The protocol version the shell attaches with.
const ATTACH_VERSION: u64 = 1;

/// How long the boot waits for the compositor's attach reply and its complete snapshot.
const BOOT_WAIT: Duration = Duration::from_secs(5);
/// The pause between boot polls while the reply is still on its way.
const BOOT_POLL: Duration = Duration::from_millis(5);

/// The least time the embedded library's install may use, whatever the config's `stage_budget`: the bundled modules
/// are compiled in that call, and a budget chosen to bound a user's entry must not starve them.
const HOST_INSTALL_BUDGET: Duration = Duration::from_secs(5);

/// What a shell is built from.
#[derive(Debug, Clone)]
pub struct JsShellConfig {
    /// The generation's config: root, memory limit, stack limit and the budget of loading modules.
    pub js: JsConfig,
    /// What one listener call may use. Default [`ENTRY_BUDGET`].
    pub entry_budget: Duration,
    /// What one frame's dispatch may use, all listener calls together. Default [`FRAME_BUDGET`].
    pub frame_budget: Duration,
    /// A directory to read the stock `gnoblin` modules from instead of the copies compiled in (the layout of
    /// `data/js/gnoblin`). A module missing from it is the compiled-in one. `None` runs the compiled-in library.
    pub stock_dir: Option<PathBuf>,
    /// Whether the session declared XR (`GNOBLIN_XR=1`). A config may then import `gnoblin/extensions/xr` before the
    /// headset's session has started; without it that import works only once the program's `xr` seat is in the tree.
    pub xr_declared: bool,
    /// The monotonic clock the scripts read (`gnoblin.timers.now`, and so every timer, fade, delay and arming window).
    /// The real clock by default; a test that must not depend on how fast the machine is gives it a
    /// [`JsManualClock`](crate::JsManualClock) and moves it.
    pub clock: Arc<dyn JsClock>,
}

impl JsShellConfig {
    /// The product budgets of `crate::budget`: a listener call, a frame and a module load each have a limit set well
    /// above what a loaded machine needs.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            js: JsConfig::new(root),
            entry_budget: ENTRY_BUDGET,
            frame_budget: FRAME_BUDGET,
            stock_dir: None,
            xr_declared: false,
            clock: Arc::new(JsSystemClock),
        }
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
    /// The config to stage generations from: the shell's, with the stock library and `gnoblin/host` embedded.
    pub fn config(&self) -> &JsConfig {
        &self.js
    }

    /// Installs the host objects in `generation` and loads `gnoblin/host`, so `gnoblin.clients` is there when
    /// the entry module runs. Run it on a fresh generation, before the entry.
    pub fn prepare(&self, generation: &JsGeneration) -> Result<(), JsError> {
        install_host(generation, &self.state, self.js.stage_budget)
    }
}

/// The shell's JavaScript runtime over the real tree.
pub struct JsShell {
    link: Box<dyn TreeLink>,
    generation: JsGeneration,
    /// The config generations are built from, with the stock library and the host module embedded.
    js: JsConfig,
    state: SharedState,
    /// The writes the shell held when a stage began, kept until that stage is swapped in or abandoned so a stage
    /// that fails after writing leaves nothing behind.
    stage_mark: Option<TxBuilder>,
    entry_budget: Duration,
    frame_budget: Duration,
    /// Messages that arrived with the boot snapshot, handled by the first frame.
    incoming: Vec<TreeIncoming>,
    /// The snapshot's nodes, parents first, to announce `ready` for on the first frame.
    boot_ready: Option<Vec<NodeId>>,
    /// The serial and ops of each sent transaction, oldest first, until its reply arrives. The compositor
    /// answers in order, but a refusal carries the serial the tree had then and not the one the shell
    /// stamped, so a reply is matched to its transaction by position.
    sent: VecDeque<Sent>,
    /// Serial of the one transaction in flight. Nothing else is sent until its reply has come and the mirror reaches it.
    in_flight: Option<u64>,
    /// Ops refused `stale-base` that are sent again, ahead of the next frame's writes.
    retry: Vec<TxOp>,
    /// How many transactions in a row were refused `stale-base` and sent again; an applied one resets it.
    stale_refusals: u32,
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
}

impl JsShell {
    /// Attaches to the tree and loads the `gnoblin/host` module, so `gnoblin.clients` is there for the entry
    /// module. The stock library is embedded beside it, and `gnoblin` re-exports both. Fails when the link
    /// does not deliver a complete snapshot within five seconds.
    pub fn new(mut link: Box<dyn TreeLink>, config: JsShellConfig) -> Result<Self, JsShellError> {
        link.attach(ATTACH_VERSION).map_err(|e| JsShellError::Link(e.0))?;
        let (mirror, incoming) = boot_mirror(link.as_mut())?;
        let boot_ready = bindings::parents_first(&mirror);
        let state: SharedState = Rc::new(RefCell::new(HostState::with_clock(mirror, Arc::clone(&config.clock))));
        state.borrow_mut().xr = crate::xr::JsXrState::declared(config.xr_declared);

        let mut js = config.js.clone();
        // The console's rate limit counts on the clock the scripts read, so a test that moves it moves that too.
        js.clock = Arc::clone(&config.clock);
        let typescript = Arc::make_mut(&mut js.typescript);
        // The user-facing `gnoblin` module is the stock index plus the host's exports (`stock::host_export`, which
        // `editor_library` appends to the index an editor reads too). A module the caller embedded under a name
        // stays: the caller knows what it wants. The library is embedded as TypeScript and stripped only when a
        // module is imported and the bytecode cache lacks it.
        let mut stock = stock_sources_in(config.stock_dir.as_deref());
        if let Some(index) = stock.get_mut("gnoblin") {
            index.source.to_mut().push_str(&host_export());
        }
        for (name, module) in stock {
            typescript.entry(name).or_insert(module);
        }
        // Generation 0 is the bare one (the host objects and no user code); the reloader numbers the ones it
        // stages from 1.
        let generation = JsGeneration::new(&js, 0)?;
        install_host(&generation, &state, js.stage_budget)?;

        Ok(Self {
            link,
            generation,
            js,
            state,
            stage_mark: None,
            entry_budget: config.entry_budget,
            frame_budget: config.frame_budget,
            incoming,
            boot_ready: Some(boot_ready),
            sent: VecDeque::new(),
            in_flight: None,
            retry: Vec::new(),
            stale_refusals: 0,
            reported: HashMap::new(),
            strikes: 0,
            dispatches: 0,
            dialogs: JsDialogs::new(),
            idle: JsIdle::new(),
            document_revision: 0,
            gate: gate::FrameGate::default(),
        })
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
    /// is known that the stage succeeds. The mark is what [`abandon_stage`](Self::abandon_stage) returns them to.
    /// The seat claims the staged generation makes are kept apart for the same reason: they are sent only when the
    /// generation takes over, after the old one's have been released.
    pub fn begin_stage(&mut self) {
        // The staged generation's entry runs against the live tree, so it is script that runs between frames.
        self.note_script_ran();
        let tick = self.state.borrow().tick.clone();
        self.stage_mark = Some(tick);
        input::begin_staging(&mut self.state.borrow_mut());
    }

    /// Ends a stage that failed: the pending writes go back to what they were at [`begin_stage`](Self::begin_stage),
    /// and the claims it kept are dropped. The running generation was never touched.
    pub fn abandon_stage(&mut self) {
        self.note_script_ran();
        let mut state = self.state.borrow_mut();
        input::abandon_staging(&mut state);
        if let Some(tick) = self.stage_mark.take() {
            state.tick = tick;
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
    /// A script failure while the old generation ends or the new one finishes is a diagnostic in the result, not
    /// an error: the swap has happened.
    ///
    /// # Errors
    ///
    /// The link failed or the mirror refused a batch; the shell has ended.
    pub fn swap_generation(&mut self, generation: JsGeneration) -> Result<Vec<String>, JsShellError> {
        self.note_script_ran();
        let mut diagnostics = Vec::new();
        let previous = match self.workspace_snapshot() {
            Ok(snapshot) => Some(snapshot),
            Err(JsShellError::Js(error)) => {
                self.count_strike(&error);
                diagnostics.push(format!(
                    "the old generation's workspaces could not be read, so windows rejoin a workspace by the stock \
                     rule: {error}"
                ));
                None
            }
            Err(other) => return Err(other),
        };
        // The new generation's claims were kept while it was staged. They go out behind everything the old one
        // gives up, in one transaction: the compositor applies a transaction's calls in order, so each chord is
        // free when its new claim arrives. A claim the old generation did not release (its script threw first) is
        // released here, or its chord would be refused to the new generation.
        let staged = input::end_staging(&mut self.state.borrow_mut());
        match self.end_generation(true) {
            Ok(frame) => diagnostics.extend(frame.diagnostics),
            Err(JsShellError::Js(error)) => {
                self.count_strike(&error);
                diagnostics.push(format!("the old generation did not end cleanly: {error}"));
            }
            Err(other) => return Err(other),
        }
        {
            let mut state = self.state.borrow_mut();
            input::release_all(&mut state);
            input::send_staged(&mut state, staged);
        }
        self.send_transaction()?;

        self.stage_mark = None;
        self.generation = generation;
        self.strikes = 0;
        self.reported.clear();
        self.boot_ready = Some(bindings::parents_first(&self.state.borrow().mirror));

        let finished = self.finish_load().and_then(|()| match &previous {
            Some(previous) => self.reconcile(previous),
            None => Ok(()),
        });
        match finished {
            Ok(()) => {}
            Err(JsShellError::Js(error)) => {
                self.count_strike(&error);
                diagnostics.push(format!(
                    "the new generation could not take over the workspaces: {error}"
                ));
            }
            Err(other) => return Err(other),
        }
        self.apply_document(&mut diagnostics, true)?;
        self.collect_reports(&mut diagnostics)?;
        Ok(diagnostics)
    }

    /// Finishes a loaded entry: a config that declared no workspace gets the stock one now, so
    /// `gnoblin.workspaces` is never empty for what runs after the entry.
    fn finish_load(&mut self) -> Result<(), JsShellError> {
        self.call_entry(self.entry_budget, |entry| {
            entry.get::<_, Function<'_>>("finishLoad")?.call::<_, ()>(())
        })
    }

    /// The id of the generation that runs.
    pub fn generation_id(&self) -> u64 {
        self.generation.id()
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
        if let Err(error) = self.generation.eval_module(entry) {
            self.count_strike(&error);
            return Err(error.into());
        }
        // The windows that were there at boot join a workspace on the first frame.
        self.finish_load()?;
        let mut diagnostics = Vec::new();
        self.apply_document(&mut diagnostics, true)?;
        self.collect_reports(&mut diagnostics)?;
        Ok(diagnostics)
    }

    /// Runs the stock default config as the generation's whole config, for a shell with no user entry (or whose
    /// entry could not load): `installDefaults` of `gnoblin/starter`, which declares four floating workspaces, the
    /// titlebar, menu and alert helpers and the stock keys with the same calls a user's `init.ts` makes. Returns the
    /// failures of calls that ran while it loaded.
    ///
    /// # Errors
    ///
    /// The stock config threw (a bug in the library) or the link failed.
    pub fn load_default_config(&mut self) -> Result<Vec<String>, JsShellError> {
        self.note_script_ran();
        // A load, like `load_entry`, so it has the stage budget: it evaluates the whole starter, which took as long as
        // any config under load, and a listener call's budget would interrupt it.
        self.call_entry(self.js.stage_budget, |entry| {
            entry.get::<_, Function<'_>>("loadDefault")?.call::<_, ()>(())
        })?;
        self.finish_load()?;
        let mut diagnostics = Vec::new();
        self.apply_document(&mut diagnostics, true)?;
        self.collect_reports(&mut diagnostics)?;
        Ok(diagnostics)
    }

    /// Which window is on which workspace and which workspace is shown, to hand to the shell of the next
    /// generation. Workspaces are script objects, so the compositor cannot say.
    pub fn workspace_snapshot(&mut self) -> Result<JsWorkspaceSnapshot, JsShellError> {
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

/// Installs the host functions in `generation` and loads `gnoblin/host`, under `budget` or
/// [`HOST_INSTALL_BUDGET`], whichever is longer.
fn install_host(generation: &JsGeneration, state: &SharedState, budget: Duration) -> Result<(), JsError> {
    generation.call_budgeted(budget.max(HOST_INSTALL_BUDGET), |ctx| {
        bindings::install(ctx, state)?;
        Module::import(ctx, HOST_MODULE_NAME)?.finish::<Value<'_>>().map(drop)
    })
}

/// Drains the link until the complete snapshot has arrived, building the mirror from it. Anything else
/// that arrives meanwhile is kept for the first frame.
fn boot_mirror(link: &mut dyn TreeLink) -> Result<(TreeMirror, Vec<TreeIncoming>), JsShellError> {
    let mut attached = None;
    let mut nodes: Vec<SnapshotNode> = Vec::new();
    let mut snapshot_serial = 0;
    let mut rest = Vec::new();
    let mut complete = false;
    let deadline = Instant::now() + BOOT_WAIT;
    loop {
        for message in link.poll().map_err(|e| JsShellError::Link(e.0))? {
            match message {
                TreeIncoming::Attached(a) => attached = Some(a),
                TreeIncoming::Snapshot {
                    serial,
                    last,
                    nodes: chunk,
                    ..
                } => {
                    snapshot_serial = serial;
                    nodes.extend(chunk);
                    complete |= last;
                }
                other => rest.push(other),
            }
        }
        if (attached.is_some() && complete) || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(BOOT_POLL);
    }
    let attached = attached.ok_or(JsShellError::NoAttach)?;
    if !complete {
        return Err(JsShellError::IncompleteSnapshot);
    }
    let mirror = TreeMirror::from_snapshot(attached.epoch, attached.root, snapshot_serial, &nodes)
        .map_err(|e| JsShellError::Mirror(format!("{e:?}")))?;
    Ok((mirror, rest))
}
