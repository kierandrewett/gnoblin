//! The JavaScript shell: what `ShellVm` is for Lua, over the same tree link.
//!
//! [`JsShell`] attaches to the compositor's tree, loads the embedded `gnoblin/host` module, evaluates the
//! user's entry module, and then runs one [`JsShell::frame`] per turn of the runtime loop: it applies the
//! compositor's batch to the mirror, tells the script's listeners what changed, and sends everything the
//! script wrote as one transaction.
//!
//! The protocol rules are the ones `gnoblin-lua`'s `ShellVm` keeps, because they belong to the tree
//! protocol and not to the language: one transaction in flight (a second one built on the same base is
//! refused as stale), a refusal matched to its transaction by position, a `stale-base` or `unknown-node`
//! refusal sent again ahead of the next frame's writes, at most [`STALE_RETRIES`] times in a row.
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

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gnoblin_tree_client::tree_mirror::{
    MirrorChangeKind, NodeId, SnapshotNode, TreeIncoming, TreeLink, TreeMirror, TxBuilder, TxOp, TxReply,
};
use rquickjs::convert::Coerced;
use rquickjs::{Array, FromJs, Function, Module, Object, Value};
use thiserror::Error;

use crate::bindings::{self, HostState, JsSpawner, SharedState};
use crate::error::{JsError, JsErrorKind};
use crate::generation::{JsConfig, JsGeneration};
use crate::input::{JsInputMessage, messages_to_js};
use crate::stock::stock_modules_in;
use crate::ts::strip_types;
use crate::workspaces::{JsWorkspaceSnapshot, coalesce_sets};

gnoblin_log::logger!(log = "js");

/// The object model of the `gnoblin` global (`gnoblin.clients`, `Client`, the listeners), in TypeScript. It
/// is stripped when a shell boots, like a user's file, and embedded as `gnoblin/host`, which the stock
/// `gnoblin` module re-exports.
const HOST_MODULE: &str = include_str!("../js/gnoblin.ts");

/// The import name of [`HOST_MODULE`].
const HOST_MODULE_NAME: &str = "gnoblin/host";

/// How many times a write the compositor refused `stale-base` is sent again before it is dropped: the
/// number `gnoblin-lua`'s shell keeps. A node the compositor changes before every landing would refuse the
/// write for ever, and the writes beside it, so after this many refusals in a row the refused write is
/// dropped and the transaction's other writes go on.
pub const STALE_RETRIES: u32 = 3;

/// The protocol version the shell attaches with.
const ATTACH_VERSION: u64 = 1;

/// How long the boot waits for the compositor's attach reply and its complete snapshot.
const BOOT_WAIT: Duration = Duration::from_secs(5);
/// The pause between boot polls while the reply is still on its way.
const BOOT_POLL: Duration = Duration::from_millis(5);

/// Calls whose text is content and not a secret, so a retry may send it again. The same list the Lua
/// shell keeps; no JavaScript call carries text yet, so this only holds the rule in place.
const PLAIN_TEXT_CALLS: &[&str] = &["measure", "claim", "claim_gesture", "claim_device", "capture"];

/// What a shell is built from.
#[derive(Debug, Clone)]
pub struct JsShellConfig {
    /// The generation's config: root, memory limit, stack limit and the budget of loading modules.
    pub js: JsConfig,
    /// What one listener call may use. Default 100 ms.
    pub entry_budget: Duration,
    /// What one frame's dispatch may use, all listener calls together. Default 500 ms.
    pub frame_budget: Duration,
    /// A directory to read the stock `gnoblin` modules from instead of the copies compiled in (the layout of
    /// `data/js/gnoblin`). A module missing from it is the compiled-in one. `None` runs the compiled-in library.
    pub stock_dir: Option<PathBuf>,
}

impl JsShellConfig {
    /// The defaults of the Lua shell: 100 ms per listener call, 500 ms per frame.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            js: JsConfig::new(root),
            entry_budget: Duration::from_millis(100),
            frame_budget: Duration::from_millis(500),
            stock_dir: None,
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

/// The sent-log entry of one transaction: its serial and each op, or `None` for a call whose text is not
/// kept.
type Sent = (u64, Vec<Option<TxOp>>);

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
    /// Serial of the one transaction in flight. Nothing else is sent until the mirror reaches it.
    in_flight: Option<u64>,
    /// Ops refused `stale-base` that are sent again, ahead of the next frame's writes.
    retry: Vec<TxOp>,
    /// How many transactions in a row were refused `stale-base` and sent again; an applied one resets it.
    stale_refusals: u32,
    /// How many times each failure site was seen. The first is reported, the rest only counted.
    reported: HashMap<String, u64>,
    /// How many times a call ran past its wall-clock or memory budget in this generation.
    strikes: u32,
}

impl JsShell {
    /// Attaches to the tree and loads the `gnoblin/host` module, so `gnoblin.clients` is there for the entry
    /// module. The stock library is embedded beside it, and `gnoblin` re-exports both. Fails when the link
    /// does not deliver a complete snapshot within five seconds.
    pub fn new(mut link: Box<dyn TreeLink>, config: JsShellConfig) -> Result<Self, JsShellError> {
        link.attach(ATTACH_VERSION).map_err(|e| JsShellError::Link(e.0))?;
        let (mirror, incoming) = boot_mirror(link.as_mut())?;
        let boot_ready = bindings::parents_first(&mirror);
        let state: SharedState = Rc::new(RefCell::new(HostState::new(mirror)));

        let host_module = strip_types(Path::new("gnoblin/host.ts"), HOST_MODULE).map_err(JsError::from)?;
        let mut js = config.js.clone();
        let embedded = Arc::make_mut(&mut js.embedded);
        // The user-facing `gnoblin` module is the stock index plus the host's exports, so one import line
        // reaches `defineConfig` and `ClientListener` alike. The stock sources stay as they are: their own
        // tests embed them without a host. A module the caller embedded under a name stays: the caller
        // knows what it wants.
        let mut stock = stock_modules_in(config.stock_dir.as_deref());
        stock
            .entry("gnoblin".to_owned())
            .or_default()
            .push_str(&format!("\nexport * from \"{HOST_MODULE_NAME}\";\n"));
        for (name, source) in stock {
            embedded.entry(name).or_insert(source);
        }
        embedded.insert(HOST_MODULE_NAME.to_owned(), host_module.code);
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
    pub fn begin_stage(&mut self) {
        let tick = self.state.borrow().tick.clone();
        self.stage_mark = Some(tick);
    }

    /// Ends a stage that failed: the pending writes go back to what they were at [`begin_stage`](Self::begin_stage).
    /// The running generation was never touched.
    pub fn abandon_stage(&mut self) {
        if let Some(tick) = self.stage_mark.take() {
            self.state.borrow_mut().tick = tick;
        }
    }

    /// Replaces the running generation with a staged one, after [`begin_stage`](Self::begin_stage).
    ///
    /// The tree is the compositor's and the mirror is shared, so every window keeps its id and its place: nothing
    /// is re-created or moved. The order is the one the input and workspace lanes wrote their hooks for: the old
    /// generation's workspace snapshot is read, then it is [`retire`](Self::retire)d (its drags are cancelled and
    /// its seat claims released, in the transaction that also carries the new generation's own claims, so no
    /// chord is ever unclaimed between them), then the new generation takes over. It has run its entry (a
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
        match self.retire() {
            Ok(frame) => diagnostics.extend(frame.diagnostics),
            Err(JsShellError::Js(error)) => {
                self.count_strike(&error);
                diagnostics.push(format!("the old generation did not end cleanly: {error}"));
            }
            Err(other) => return Err(other),
        }

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
                diagnostics.push(format!("the new generation could not take over the workspaces: {error}"));
            }
            Err(other) => return Err(other),
        }
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

    /// Routes `gnoblin.spawn` to `spawner`. Without one, `spawn` throws a `GnoblinError`.
    pub fn set_spawner(&mut self, spawner: JsSpawner) {
        self.state.borrow_mut().spawner = Some(spawner);
    }

    /// How many times a call ran past its wall-clock or memory budget in this generation.
    pub const fn strikes(&self) -> u32 {
        self.strikes
    }

    /// Evaluates the user's entry module, a path under the config root. Returns the failures of listener
    /// calls that ran while it loaded (a replay on `addListener`). An error loading the module is `Err`,
    /// and counts a strike when it is a budget trip.
    pub fn load_entry(&mut self, entry: &Path) -> Result<Vec<String>, JsShellError> {
        if let Err(error) = self.generation.eval_module(entry) {
            self.count_strike(&error);
            return Err(error.into());
        }
        // The windows that were there at boot join a workspace on the first frame.
        self.finish_load()?;
        let mut diagnostics = Vec::new();
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
        self.call_entry(self.entry_budget, |entry| {
            let previous = previous.to_js(entry.ctx())?;
            entry.get::<_, Function<'_>>("reconcile")?.call::<_, ()>((previous,))
        })
    }

    /// Evaluates `source` as a script in the generation and returns its value as text, or `None` for
    /// `undefined`. For tests and a console; it runs under the listener budget.
    pub fn eval(&mut self, source: &str) -> Result<Option<String>, JsShellError> {
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

    /// Runs one frame: applies what the compositor sent, tells the script's listeners, runs them, and
    /// sends what the script wrote as one transaction.
    pub fn frame(&mut self) -> Result<JsFrame, JsShellError> {
        let mut diagnostics = Vec::new();
        let mut touched: Vec<NodeId> = Vec::new();
        let mut created: Vec<NodeId> = Vec::new();
        let mut destroyed: Vec<NodeId> = Vec::new();

        let mut incoming = std::mem::take(&mut self.incoming);
        incoming.extend(self.link.poll().map_err(|e| JsShellError::Link(e.0))?);
        for message in incoming {
            match message {
                TreeIncoming::Changes { serial, changes } => {
                    let mut state = self.state.borrow_mut();
                    // A destroyed node's last values stay readable for the frame's plan, deepest node
                    // first as a node's own destruction follows its subtree's.
                    for change in changes.iter().filter(|c| c.kind == MirrorChangeKind::Destroyed) {
                        let subtree = change.removed.iter().copied().filter(|id| *id != change.node);
                        for id in subtree.rev().chain(std::iter::once(change.node)) {
                            if let Some(record) = state.mirror.node(id).cloned() {
                                state.graveyard.insert(id, record);
                            }
                            destroyed.push(id);
                        }
                    }
                    state
                        .mirror
                        .apply_changes(serial, &changes)
                        .map_err(|e| JsShellError::Mirror(format!("{e:?}")))?;
                    touched.extend(
                        changes
                            .iter()
                            .filter(|c| c.kind == MirrorChangeKind::Set)
                            .map(|c| c.node),
                    );
                    created.extend(
                        changes
                            .iter()
                            .filter(|c| c.kind == MirrorChangeKind::Created)
                            .map(|c| c.node),
                    );
                }
                TreeIncoming::Reply(reply) => self.handle_reply(&reply, &mut diagnostics),
                // A claimed chord or gesture is parked for the input module, which the frame hands it to once
                // the listeners are planned. Pointer and key events addressed to nodes and call answers have no
                // JavaScript API: they are consumed so they do not pile up.
                TreeIncoming::Chord(chord) => self.state.borrow_mut().input.inbox.push(JsInputMessage::Chord(chord)),
                TreeIncoming::Gesture(gesture) => self
                    .state
                    .borrow_mut()
                    .input
                    .inbox
                    .push(JsInputMessage::Gesture(gesture)),
                TreeIncoming::Event(_) | TreeIncoming::Answer { .. } => {}
                TreeIncoming::Attached(_) | TreeIncoming::Snapshot { .. } => {
                    return Err(JsShellError::Link("attach or snapshot after boot".to_owned()));
                }
            }
        }

        let settled = self.sent.is_empty() && self.retry.is_empty();
        {
            let state = &mut *self.state.borrow_mut();
            if settled {
                state.tick.settle();
            } else {
                state.tick.refresh(&state.mirror);
            }
            // A node a later batch of this frame destroyed has nothing to wrap or to touch, and a node made
            // in this frame is born with its final values, so only older nodes raise change events.
            created.retain(|node| state.mirror.node(*node).is_some());
            touched.retain(|node| state.mirror.node(*node).is_some() && !created.contains(node));
        }

        let started = Instant::now();
        let outcome = self.dispatch(&created, &touched, &destroyed, started, &mut diagnostics);
        self.state.borrow_mut().graveyard.clear();
        outcome?;
        self.run_layouts(&mut diagnostics)?;
        self.collect_reports(&mut diagnostics)?;

        self.send_transaction()?;
        Ok(JsFrame { diagnostics })
    }

    /// Ends the generation's hold on input: every drag and gesture a binding is running gets its `onCancel`
    /// once, under the call budget, and every seat claim is released, so no pointer or chord stays captured
    /// by a generation that is going away. The caller runs this before it drops or replaces the shell (a
    /// reload swaps generations, a shutdown ends it). The `unclaim` calls travel in the transaction this
    /// sends; when an earlier transaction is still in flight they wait for the next [`JsShell::frame`]. The
    /// compositor releases a connection's claims by itself when the shell dies, so a crash cannot leave a
    /// chord claimed.
    pub fn retire(&mut self) -> Result<JsFrame, JsShellError> {
        let mut diagnostics = Vec::new();
        let pending: usize = self.call_entry(self.frame_budget, |entry| {
            entry.get::<_, Function<'_>>("cancelAll")?.call(())
        })?;
        let started = Instant::now();
        self.state.borrow_mut().input.inbox.clear();
        self.run_pending(pending, started, &mut diagnostics)?;
        self.collect_reports(&mut diagnostics)?;
        self.send_transaction()?;
        Ok(JsFrame { diagnostics })
    }

    /// Hands the frame's changes to the module's `plan`, then runs the queued listener calls one by one,
    /// each under its own budget, until they are all run or the frame's budget is.
    fn dispatch(
        &mut self,
        created: &[NodeId],
        touched: &[NodeId],
        destroyed: &[NodeId],
        started: Instant,
        diagnostics: &mut Vec<String>,
    ) -> Result<(), JsShellError> {
        if let Some(nodes) = self.boot_ready.take() {
            let nodes = self.script_ids(&nodes);
            let _queued: usize = self.call_entry(self.frame_budget, move |entry| {
                entry.get::<_, Function<'_>>("ready")?.call((nodes,))
            })?;
        }
        let (created, touched, destroyed) = (
            self.script_ids(created),
            self.script_ids(touched),
            self.script_ids(destroyed),
        );
        let _queued: usize = self.call_entry(self.frame_budget, move |entry| {
            entry
                .get::<_, Function<'_>>("plan")?
                .call((created, touched, destroyed))
        })?;
        // The input matcher runs after the plan, so a client that died in this frame has already cancelled
        // its drags when the frame's chord messages are read. It runs every frame, with or without messages:
        // a drag samples the pointer once per frame.
        let messages = std::mem::take(&mut self.state.borrow_mut().input.inbox);
        let pending: usize = self.call_entry(self.frame_budget, move |entry| {
            let list = messages_to_js(entry.ctx(), &messages)?;
            entry.get::<_, Function<'_>>("input")?.call((list,))
        })?;
        self.run_pending(pending, started, diagnostics)
    }

    /// Runs the queued calls one by one, each under its own budget, until they are all run or the frame's
    /// budget is used up. Calls that do not fit wait for the next frame.
    fn run_pending(
        &mut self,
        mut pending: usize,
        started: Instant,
        diagnostics: &mut Vec<String>,
    ) -> Result<(), JsShellError> {
        while pending > 0 {
            if started.elapsed() >= self.frame_budget {
                diagnostics.push(format!(
                    "the frame's {} ms budget is used up: {pending} listener calls wait for the next frame",
                    self.frame_budget.as_millis()
                ));
                break;
            }
            match self.call_entry(self.entry_budget, |entry| {
                entry.get::<_, Function<'_>>("next")?.call(())
            }) {
                Ok(left) => pending = left,
                Err(JsShellError::Js(error)) if matches!(error.kind, JsErrorKind::Budget | JsErrorKind::Memory) => {
                    // The call that ran out was taken off the queue before it ran, so the next one is
                    // next. Its failure is a diagnostic here, and the listeners after it still run.
                    self.count_strike(&error);
                    diagnostics.push(format!("a listener call was stopped: {error}"));
                    pending = self.call_entry(self.entry_budget, |entry| {
                        entry.get::<_, Function<'_>>("pending")?.call(())
                    })?;
                }
                Err(other) => return Err(other),
            }
        }
        Ok(())
    }

    /// Lays out the workspaces that changed this frame (membership, shown workspace, area or a layout's own
    /// fields), once each, after every listener has run so the layouts see the frame's final state. A layout
    /// that throws is a diagnostic from the module; one that runs out its budget stops the pass, counts a
    /// strike, and leaves the workspaces after it for the next frame.
    fn run_layouts(&mut self, diagnostics: &mut Vec<String>) -> Result<(), JsShellError> {
        match self.call_entry(self.frame_budget, |entry| {
            entry.get::<_, Function<'_>>("layout")?.call::<_, usize>(())
        }) {
            Ok(_laid_out) => Ok(()),
            Err(JsShellError::Js(error)) if matches!(error.kind, JsErrorKind::Budget | JsErrorKind::Memory) => {
                self.count_strike(&error);
                diagnostics.push(format!("the layout pass was stopped: {error}"));
                Ok(())
            }
            Err(other) => Err(other),
        }
    }

    /// The numbers a script holds for a list of node ids (see `NodeHandles`).
    fn script_ids(&self, nodes: &[NodeId]) -> Vec<f64> {
        let state = self.state.borrow();
        nodes.iter().map(|id| state.handles.to_script(*id)).collect()
    }

    /// Runs `f` on the module's entry object, under `budget`.
    fn call_entry<R>(
        &self,
        budget: Duration,
        f: impl for<'js> FnOnce(Object<'js>) -> rquickjs::Result<R>,
    ) -> Result<R, JsShellError> {
        self.generation
            .call_budgeted(budget, |ctx| {
                let host: Object<'_> = ctx.globals().get("__gnoblinHost")?;
                f(host.get("entry")?)
            })
            .map_err(JsShellError::from)
    }

    /// Takes the failures the listeners recorded and reports each site once.
    fn collect_reports(&mut self, diagnostics: &mut Vec<String>) -> Result<(), JsShellError> {
        let generation = &self.generation;
        let failures: Vec<(String, JsError)> = generation.call_budgeted(self.entry_budget, |ctx| {
            let host: Object<'_> = ctx.globals().get("__gnoblinHost")?;
            let entry: Object<'_> = host.get("entry")?;
            let list: Array<'_> = entry.get::<_, Function<'_>>("takeErrors")?.call(())?;
            let mut failures = Vec::new();
            for item in list.iter::<Object<'_>>() {
                let item = item?;
                let site: String = item.get("site")?;
                let thrown: Value<'_> = item.get("error")?;
                failures.push((site, generation.describe(ctx, &thrown)));
            }
            Ok(failures)
        })?;
        for (site, error) in failures {
            // One site is one listener method failing at one place; a loop that fails every frame is
            // logged once, not once per frame.
            let key = format!(
                "{site}@{}",
                error.location.as_ref().map_or_else(String::new, location_text)
            );
            let seen = self.reported.entry(key).or_insert(0);
            *seen += 1;
            if *seen == 1 {
                let text = format!("{site} threw: {error}");
                log::warn!(area = "listener", "{text}");
                diagnostics.push(text);
            }
        }
        Ok(())
    }

    fn count_strike(&mut self, error: &JsError) {
        if matches!(error.kind, JsErrorKind::Budget | JsErrorKind::Memory) {
            self.strikes = self.strikes.saturating_add(1);
        }
    }

    /// Matches a reply to its transaction and, for a stale refusal, keeps the ops to send again.
    fn handle_reply(&mut self, reply: &TxReply, diagnostics: &mut Vec<String>) {
        let (sent_serial, ops) = self.sent.pop_front().unwrap_or((reply.serial, Vec::new()));
        if reply.applied {
            self.stale_refusals = 0;
        }
        let resend = matches!(reply.code.as_str(), "stale-base" | "unknown-node");
        if !reply.applied && resend && reply.failed_op >= 0 {
            // A published batch changed a node this transaction writes (stale-base), or destroyed it
            // (unknown-node). The refused transaction applied nothing, so its ops are sent again on the
            // next base, ahead of what the next frame writes, so the script's newest write lands last. The
            // gate stays shut until the mirror reaches this serial, which that batch does.
            //
            // The failed op is sent again too under stale-base: the node still exists and the op is the
            // script's intent, which nothing else writes again. It is sent again `STALE_RETRIES` times in a
            // row; after that, and always under unknown-node (its node is gone), it is dropped.
            let failed = usize::try_from(reply.failed_op).ok();
            let retry_failed = reply.code == "stale-base" && self.stale_refusals < STALE_RETRIES;
            self.stale_refusals = self.stale_refusals.saturating_add(1);
            let mut kept: Vec<TxOp> = Vec::new();
            for (index, op) in ops.into_iter().enumerate() {
                if Some(index) == failed && !retry_failed {
                    continue;
                }
                match op {
                    Some(op) => kept.push(op),
                    // Its text was not kept, so it is not sent again; the diagnostic says which call.
                    None => diagnostics.push(format!(
                        "transaction {} stale at op {}: text call {index} was dropped and not sent again",
                        reply.serial, reply.failed_op
                    )),
                }
            }
            log::debug!(
                area = "shell",
                "transaction {} stale at op {}; {} ops sent again",
                reply.serial,
                reply.failed_op,
                kept.len()
            );
            kept.append(&mut self.retry);
            self.retry = kept;
            return;
        }
        if self.in_flight == Some(sent_serial) && !reply.applied {
            // A refused transaction never advances the serial, so the gate opens now.
            self.in_flight = None;
        }
        log::debug!(
            area = "shell",
            "transaction {} applied={} code={} failed_op={}",
            reply.serial,
            reply.applied,
            reply.code,
            reply.failed_op
        );
        if !reply.applied {
            let state = self.state.borrow();
            let op = usize::try_from(reply.failed_op)
                .ok()
                .and_then(|i| {
                    ops.get(i).map(|op| match op {
                        Some(op) => describe_op(i, op, &state.mirror),
                        None => format!("op {i}: call (text not kept)"),
                    })
                })
                .unwrap_or_else(|| "op not recorded".to_owned());
            diagnostics.push(format!(
                "transaction {} refused at op {}: {}; {}",
                reply.serial, reply.failed_op, reply.code, op
            ));
        }
    }

    /// Sends this frame's writes as one transaction, once the one in flight has landed.
    fn send_transaction(&mut self) -> Result<(), JsShellError> {
        let tx = {
            let mut guard = self.state.borrow_mut();
            let state = &mut *guard;
            if !self.in_flight.is_none_or(|serial| state.mirror.serial() >= serial) {
                return Ok(());
            }
            let serial = state.mirror.serial();
            let tick = std::mem::take(&mut state.tick);
            // What the transaction wrote stays readable until the compositor's batch brings it into the
            // mirror, which may be frames away.
            state.tick = TxBuilder::seeded(tick.sent(&state.mirror));
            let mut tx = tick.into_transaction(&state.mirror, serial + 1);
            if !self.retry.is_empty() {
                // Ops refused stale-base go first, on this base. An op on a node the tree has lost since is
                // moot, and sending it would refuse the whole transaction again.
                let mirror = &state.mirror;
                let mut ops = std::mem::take(&mut self.retry);
                let mut made: Vec<NodeId> = Vec::new();
                ops.retain(|op| match op {
                    TxOp::New { id, .. } => {
                        made.push(*id);
                        true
                    }
                    TxOp::Set { node, .. } | TxOp::Destroy { node } | TxOp::Call { node, .. } => {
                        made.contains(node) || mirror.node(*node).is_some()
                    }
                    TxOp::Insert { node, parent, .. } => {
                        (made.contains(node) || mirror.node(*node).is_some())
                            && (made.contains(parent) || mirror.node(*parent).is_some())
                    }
                });
                ops.append(&mut tx.ops);
                tx.ops = ops;
            }
            // One write per property, the last: a handler's write and the layout's are one intent.
            tx.ops = coalesce_sets(std::mem::take(&mut tx.ops));
            tx
        };
        if !tx.ops.is_empty() {
            self.sent
                .push_back((tx.serial, tx.ops.iter().map(retained_op).collect()));
            self.in_flight = Some(tx.serial);
            self.link.transact(&tx).map_err(|e| JsShellError::Link(e.0))?;
        }
        Ok(())
    }
}

/// Installs the host functions in `generation` and loads `gnoblin/host`, under `budget`.
fn install_host(generation: &JsGeneration, state: &SharedState, budget: Duration) -> Result<(), JsError> {
    generation.call_budgeted(budget, |ctx| {
        bindings::install(ctx, state)?;
        Module::import(ctx, HOST_MODULE_NAME)?.finish::<Value<'_>>().map(drop)
    })
}

fn location_text(at: &crate::error::JsLocation) -> String {
    format!("{}:{}:{}", at.file.display(), at.line, at.column)
}

/// What the sent-log keeps of one op for a stale-base retry. A call that carries a secret is not kept: its
/// text is not held past the send, so a retry cannot send it again.
fn retained_op(op: &TxOp) -> Option<TxOp> {
    match op {
        TxOp::Call { method, text, .. } if !text.is_empty() && !PLAIN_TEXT_CALLS.contains(&method.as_str()) => None,
        other => Some(other.clone()),
    }
}

/// Names one op for a refusal: its index, class, property and value kind, so a refused write is traceable
/// to the script call that made it.
fn describe_op(index: usize, op: &TxOp, mirror: &TreeMirror) -> String {
    let class_of = |node: NodeId| {
        mirror
            .node(node)
            .map_or_else(|| "unknown".to_owned(), |record| record.class.clone())
    };
    match op {
        TxOp::New { id, class } => format!("op {index}: new node {id} class {class}"),
        TxOp::Set { node, key, value, .. } => format!(
            "op {index}: set node {node} ({}) property {key} value kind {}",
            class_of(*node),
            value.kind_name()
        ),
        TxOp::Insert {
            node,
            parent,
            index: at,
        } => format!(
            "op {index}: insert node {node} ({}) into {parent} at {at}",
            class_of(*node)
        ),
        TxOp::Destroy { node } => format!("op {index}: destroy node {node} ({})", class_of(*node)),
        TxOp::Call { node, method, .. } => format!("op {index}: call {method} on node {node} ({})", class_of(*node)),
    }
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
