//! The state of one shell instance that lives on its shell thread: the tree it is attached to, the programs it started, the input
//! claims it holds, its timers and the state it keeps across reloads.
//!
//! The generation host (`gnoblin-shell-host`) owns generations and the registrations each one made. What belongs to the *shell
//! instance*, and outlives a generation, is here: nodes belong to the instance (a reload's generation finds the windows where the
//! old one left them), and so does the mirror, the gate that keeps one transaction in flight and the programs a reload may adopt.
//! One session per shell thread, in a thread-local, because every function that touches it is a shell-thread function.
//!
//! The rule that keeps this sound: **nothing in here calls into the host or into shell code while the session is borrowed.** A
//! method collects what to publish and returns it; the caller publishes after the borrow ends. A shell callback may therefore call
//! any table function, and the registry's release closures (which run while a generation retires) may reach back here.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use gnoblin_core_abi as abi;
use gnoblin_shell_host::generation::{GenerationId, GenerationState, LifecycleObserver};
use gnoblin_shell_host::process::{ProcessError, ProcessExit, Processes, SpawnOptions, Started};
use gnoblin_shell_host::supervisor::{Clock, SystemClock};
use gnoblin_tree_client::session::{NodeEvent, SessionError, TickMark, TreeSession};
use gnoblin_tree_client::tree_mirror::{LinkError, MirrorValue, NodeId, TreeIncoming, TreeLink, TxAnimation};

use crate::catalog;
use crate::events::{AnswerNotice, ClaimNotice, ExitNotice, NodeNotice, ShellEvent, SignalDelivery};
use crate::log;
use crate::values::OwnedValue;

/// How a shell wants the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachMode {
    /// As the shell: the only connection that may change the tree.
    Holder,
    /// As an observer: reads the snapshot and the change stream.
    Observer,
}

/// What the host gives the core to reach a compositor's tree: a way to open a link on the shell thread when the shell attaches.
///
/// The provider is not `Send`: it is set on the shell thread and called there, so a link that holds a non-`Send` handle (the
/// in-memory fake of the tests) is as welcome as a socket. The in-process compositor edge provides one over its tree queue; a
/// test provides one over the fake compositor.
pub trait TreeLinkProvider {
    /// Opens a link for `mode`. The core attaches over it.
    ///
    /// # Errors
    ///
    /// The compositor cannot be reached.
    fn open(&self, mode: AttachMode) -> Result<Box<dyn TreeLink>, LinkError>;
}

/// The most state a shell may keep across reloads, per the ABI.
const STATE_LIMIT: usize = abi::STATE_MAX_BYTES as usize;

/// An input claim the shell holds.
struct ClaimRecord {
    seat: NodeId,
    owner: GenerationId,
    /// The seat method that makes it (`claim`, `claim_gesture`, `claim_device`, `claim_pointer`, `grab`).
    method: &'static str,
    /// The claim's second integer, when it has one.
    second: Option<i64>,
    /// The call's text.
    text: String,
    /// Whether the call went out. A claim of a staged generation waits for the takeover.
    sent: bool,
}

struct TimerRecord {
    owner: GenerationId,
    due: Duration,
    period: Option<Duration>,
}

/// A value kept across reloads, with the version its writer gave it.
#[derive(Clone)]
struct Stored {
    version: u32,
    bytes: Vec<u8>,
}

/// A node a generation made under a name, so that the next generation finds it again (`node_adopt`).
#[derive(Clone, Debug)]
struct NamedNode {
    node: NodeId,
    /// The generation that made it.
    made_by: GenerationId,
    /// The latest generation that asked for it.
    claimed_by: GenerationId,
    /// Whether it dies with its generation unless the next one adopts it.
    ephemeral: bool,
}

/// What a stage puts back when it is abandoned: everything a staged generation's `init` can change in the instance that is
/// not a registration (those the generation host drops). Taken when the generation is made, forgotten when it takes over.
struct StageMark {
    generation: GenerationId,
    /// This tick's writes at that moment, when the shell was attached.
    tick: Option<TickMark>,
    named: BTreeMap<(String, String), NamedNode>,
    /// The state entries the stage replaced or added, by key, as they were before the stage first touched them.
    state: BTreeMap<String, Option<Stored>>,
}

/// The shell instance's state. See the module docs.
pub(crate) struct Session {
    provider: Option<Box<dyn TreeLinkProvider>>,
    tree: Option<TreeSession>,
    processes: Processes,
    claims: BTreeMap<i64, ClaimRecord>,
    next_claim: i64,
    timers: BTreeMap<u64, TimerRecord>,
    next_timer: u64,
    /// Flushes whose callback waits for everything sent so far to be answered, by token.
    flush_waiters: Vec<u64>,
    next_flush: u64,
    /// What the compositor refused since the oldest flush waiter asked.
    refusals: Vec<String>,
    state: BTreeMap<String, Stored>,
    state_bytes: usize,
    /// The nodes generations made under a name, by class and name: what `node_adopt` finds again.
    named: BTreeMap<(String, String), NamedNode>,
    /// The stage in progress, if a generation is staged.
    stage: Option<StageMark>,
    clock: Arc<dyn Clock>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            provider: None,
            tree: None,
            processes: Processes::default(),
            claims: BTreeMap::new(),
            next_claim: 0,
            timers: BTreeMap::new(),
            next_timer: 0,
            flush_waiters: Vec::new(),
            next_flush: 0,
            refusals: Vec::new(),
            state: BTreeMap::new(),
            state_bytes: 0,
            named: BTreeMap::new(),
            stage: None,
            clock: Arc::new(SystemClock),
        }
    }
}

thread_local! {
    static SESSION: RefCell<Session> = RefCell::new(Session::default());
}

/// Runs `f` on this thread's session. `f` must not call the host or shell code (see the module docs).
pub(crate) fn with<R>(f: impl FnOnce(&mut Session) -> R) -> R {
    SESSION.with(|session| f(&mut session.borrow_mut()))
}

/// The tokens flush waiters use: a counter with the top bit set, so they cannot meet a call's token (a small counter).
const FLUSH_TOKEN_BIT: u64 = 1 << 63;

impl Session {
    // ---- the host's seams --------------------------------------------------------------------------------------------

    /// Installs the way to reach the compositor's tree, replacing any earlier one. The tree is not opened until the shell attaches.
    pub(crate) fn set_provider(&mut self, provider: Box<dyn TreeLinkProvider>) {
        self.provider = Some(provider);
    }

    /// Installs the clock timers read. Tests give a manual one.
    pub(crate) fn set_clock(&mut self, clock: Arc<dyn Clock>) {
        self.clock = clock;
    }

    /// Drops the tree and everything that belongs to it: the end of a shell instance. Claims are gone with the connection (the
    /// compositor releases them), and so are the pending flushes.
    pub(crate) fn reset(&mut self) {
        *self = Session {
            provider: self.provider.take(),
            clock: Arc::clone(&self.clock),
            ..Session::default()
        };
    }

    /// Whether the shell is attached to the tree.
    pub(crate) fn attached(&self) -> bool {
        self.tree.is_some()
    }

    /// Opens a link through the provider and attaches over it. A session that is attached already stays so: a reload's
    /// generation shares the tree the first one holds.
    ///
    /// # Errors
    ///
    /// The ABI code and text: no provider, the link would not open, the compositor refused, or the snapshot did not arrive.
    pub(crate) fn attach(&mut self, mode: AttachMode) -> Result<(), (i32, String)> {
        if self.tree.is_some() {
            return Ok(());
        }
        let Some(provider) = self.provider.as_ref() else {
            return Err((
                abi::E_ERR,
                "tree_attach: this core has no link to a compositor (the host gave it none)".to_owned(),
            ));
        };
        let link = provider
            .open(mode)
            .map_err(|error| link_failure("tree_attach", &error.0))?;
        let tree = TreeSession::attach(link, gnoblin_tree_client::session::BOOT_WAIT).map_err(|error| match error {
            SessionError::NoAttach | SessionError::IncompleteSnapshot => {
                (abi::E_TIMEOUT, format!("tree_attach: {error}"))
            }
            SessionError::Link(text) => link_failure("tree_attach", &text),
            SessionError::Mirror(_) => (abi::E_ERR, format!("tree_attach: {error}")),
        })?;
        self.tree = Some(tree);
        Ok(())
    }

    /// The tree, when the shell has attached.
    pub(crate) fn tree(&mut self) -> Result<&mut TreeSession, (i32, String)> {
        self.tree.as_mut().ok_or_else(|| {
            (
                abi::E_ERR,
                "the shell has not attached to the tree: call tree_attach first".to_owned(),
            )
        })
    }

    /// The tree for a read.
    pub(crate) fn tree_ref(&self) -> Result<&TreeSession, (i32, String)> {
        self.tree.as_ref().ok_or_else(|| {
            (
                abi::E_ERR,
                "the shell has not attached to the tree: call tree_attach first".to_owned(),
            )
        })
    }

    // ---- the pump ----------------------------------------------------------------------------------------------------

    /// Reads what the compositor sent and what the programs reported, applies it, and returns the events to publish, in the order
    /// they happened.
    ///
    /// # Errors
    ///
    /// The link failed or a change batch was missed: the tree is dropped (the shell must attach again) and the reason returned.
    pub(crate) fn pump(&mut self) -> Result<Vec<ShellEvent>, String> {
        let mut events = Vec::new();
        if let Some(tree) = self.tree.as_mut() {
            let pumped = match tree.pump() {
                Ok(pumped) => pumped,
                Err(error) => {
                    self.tree = None;
                    self.claims.clear();
                    self.flush_waiters.clear();
                    return Err(error.to_string());
                }
            };
            for line in &pumped.effects.diagnostics {
                log::warn!("tree: {line}");
            }
            if !self.flush_waiters.is_empty() {
                self.refusals.extend(pumped.effects.diagnostics.iter().cloned());
            }
            for change in pumped.changes {
                events.extend(node_notices(change).into_iter().map(ShellEvent::Node));
            }
            for message in pumped.incoming {
                events.extend(shell_event(message));
            }
            if !self.flush_waiters.is_empty() && tree.is_settled() {
                let result = tree.serial().to_string().into_bytes();
                let error = std::mem::take(&mut self.refusals).join("; ");
                for token in self.flush_waiters.drain(..) {
                    events.push(ShellEvent::Answer(AnswerNotice {
                        token,
                        result: result.clone(),
                        error: error.clone(),
                    }));
                }
            }
        }
        for (process, exit) in self.processes.drain() {
            events.push(ShellEvent::Exit(exit_notice(process, exit)));
        }
        Ok(events)
    }

    /// Sends what the tick wrote, as one transaction when the gate allows.
    ///
    /// # Errors
    ///
    /// The link failed: the tree is dropped and the reason returned.
    pub(crate) fn flush(&mut self) -> Result<bool, String> {
        let Some(tree) = self.tree.as_mut() else {
            return Ok(false);
        };
        match tree.flush() {
            Ok(sent) => Ok(sent),
            Err(error) => {
                self.tree = None;
                self.claims.clear();
                self.flush_waiters.clear();
                Err(error.to_string())
            }
        }
    }

    /// Makes a token for a flush waiter: the next time every transaction sent so far is answered, an answer is published under it.
    pub(crate) fn wait_for_flush(&mut self) -> u64 {
        self.next_flush += 1;
        let token = FLUSH_TOKEN_BIT | self.next_flush;
        self.flush_waiters.push(token);
        token
    }

    /// Forgets a flush waiter whose callback was cancelled.
    pub(crate) fn forget_flush(&mut self, token: u64) {
        self.flush_waiters.retain(|waiting| *waiting != token);
    }

    // ---- timers ------------------------------------------------------------------------------------------------------

    /// Starts a timer for `owner` that falls due in `ms`, and again every `ms` when `repeat`.
    pub(crate) fn add_timer(&mut self, owner: GenerationId, ms: u32, repeat: bool) -> u64 {
        self.next_timer += 1;
        let id = self.next_timer;
        let period = Duration::from_millis(u64::from(ms));
        self.timers.insert(
            id,
            TimerRecord {
                owner,
                due: self.clock.now() + period,
                period: repeat.then_some(period),
            },
        );
        id
    }

    /// Forgets a timer (its registration was removed).
    pub(crate) fn remove_timer(&mut self, id: u64) {
        self.timers.remove(&id);
    }

    /// The timers that are due and whose generation is active, advanced or (a one-shot) forgotten. A timer of a staged generation
    /// stays due and fires when the generation takes over.
    pub(crate) fn take_due(&mut self, active: impl Fn(GenerationId) -> bool) -> Vec<u64> {
        let now = self.clock.now();
        let mut fired = Vec::new();
        let mut spent = Vec::new();
        for (id, timer) in &mut self.timers {
            if timer.due > now || !active(timer.owner) {
                continue;
            }
            fired.push(*id);
            match timer.period {
                // A timer that is behind starts its next period from now: a stalled shell is not owed a burst.
                Some(period) => timer.due = now + period.max(Duration::from_millis(1)),
                None => spent.push(*id),
            }
        }
        for id in spent {
            self.timers.remove(&id);
        }
        fired
    }

    /// How long until the next timer of an active generation falls due, if one is waiting.
    pub(crate) fn next_due_in(&self, active: impl Fn(GenerationId) -> bool) -> Option<Duration> {
        let now = self.clock.now();
        self.timers
            .values()
            .filter(|timer| active(timer.owner))
            .map(|timer| timer.due.saturating_sub(now))
            .min()
    }

    // ---- input claims ------------------------------------------------------------------------------------------------

    /// Allocates the id of a claim. Ids are the shell instance's, never reused, and shared by every kind of claim.
    pub(crate) fn next_claim_id(&mut self) -> i64 {
        self.next_claim += 1;
        self.next_claim
    }

    /// Records a claim and, when its generation is active, sends the call that makes it. A staged generation's claim waits for the
    /// takeover (`activated`), so that the old generation's chords are released first and the compositor never sees both.
    ///
    /// # Errors
    ///
    /// The seat is not a node of the tree.
    pub(crate) fn add_claim(
        &mut self,
        id: i64,
        seat: NodeId,
        owner: GenerationId,
        spec: (&'static str, Option<i64>, String),
        active: bool,
    ) -> Result<(), (i32, String)> {
        let (method, second, text) = spec;
        self.claims.insert(
            id,
            ClaimRecord {
                seat,
                owner,
                method,
                second,
                text,
                sent: false,
            },
        );
        if active {
            self.send_claim(id)?;
        }
        Ok(())
    }

    fn send_claim(&mut self, id: i64) -> Result<(), (i32, String)> {
        let Some(claim) = self.claims.get_mut(&id) else {
            return Ok(());
        };
        let mut args = id.to_le_bytes().to_vec();
        if let Some(second) = claim.second {
            args.extend_from_slice(&second.to_le_bytes());
        }
        let (seat, method, text) = (claim.seat, claim.method, claim.text.clone());
        claim.sent = true;
        let tree = self
            .tree
            .as_mut()
            .ok_or_else(|| (abi::E_ERR, "the tree is gone".to_owned()))?;
        if let Err(error) = tree.call(seat, method, args, text) {
            self.claims.remove(&id);
            return Err((abi::E_NO_SUCH_NODE, format!("claim: {error}")));
        }
        Ok(())
    }

    /// Gives a claim up: the call that releases it goes out in the next transaction. A claim that never went out is dropped
    /// without a call.
    pub(crate) fn release_claim(&mut self, id: i64) {
        let Some(claim) = self.claims.remove(&id) else {
            return;
        };
        if !claim.sent {
            return;
        }
        // A seat that is gone took its claims with it, so a refused call here needs no report.
        if let Some(tree) = self.tree.as_mut() {
            let _gone = tree.call(claim.seat, "unclaim", id.to_le_bytes().to_vec(), String::new());
        }
    }

    // ---- processes ---------------------------------------------------------------------------------------------------

    /// Starts a program, or keeps the spawn of a staged generation until it takes over.
    ///
    /// # Errors
    ///
    /// The limit or the program was refused.
    pub(crate) fn spawn(&mut self, argv: &[String], options: &SpawnOptions) -> Result<Started, ProcessError> {
        self.processes.spawn(argv, options)
    }

    /// Sends a signal to a program's process group.
    ///
    /// # Errors
    ///
    /// The signal's name is wrong, or the process has ended.
    pub(crate) fn kill(&mut self, process: u64, signal: &str) -> Result<(), ProcessError> {
        self.processes.kill(process, signal)
    }

    // ---- state kept across reloads -----------------------------------------------------------------------------------

    /// Keeps `bytes` under `key`, replacing what was there.
    ///
    /// # Errors
    ///
    /// The store would pass its limit.
    pub(crate) fn put_state(&mut self, key: &str, bytes: Vec<u8>, version: u32) -> Result<(), String> {
        let replaced = self.state.get(key).map_or(0, |stored| stored.bytes.len() + key.len());
        let after = self.state_bytes - replaced + bytes.len() + key.len();
        if after > STATE_LIMIT {
            return Err(format!(
                "the state store holds at most {STATE_LIMIT} bytes, and this would make it {after}"
            ));
        }
        if let Some(stage) = self.stage.as_mut() {
            // A staged generation's write is undone if the stage is abandoned: remember what the key held before the stage first
            // touched it.
            stage
                .state
                .entry(key.to_owned())
                .or_insert_with(|| self.state.get(key).cloned());
        }
        self.state_bytes = after;
        self.state.insert(key.to_owned(), Stored { version, bytes });
        Ok(())
    }

    /// What was kept under `key`, and the version it was kept with.
    pub(crate) fn get_state(&self, key: &str) -> Option<(Vec<u8>, u32)> {
        self.state.get(key).map(|stored| (stored.bytes.clone(), stored.version))
    }

    // ---- nodes found again by name -----------------------------------------------------------------------------------

    /// `node_adopt`: the node of `class` that `owner`'s generation or an earlier one made under `name`, or a new one. Returns the
    /// node and whether an earlier generation made it.
    ///
    /// # Errors
    ///
    /// The shell has not attached, or the write was refused.
    pub(crate) fn adopt_node(
        &mut self,
        owner: GenerationId,
        class: &str,
        name: &str,
        ephemeral: bool,
    ) -> Result<(NodeId, bool), (i32, String)> {
        let key = (class.to_owned(), name.to_owned());
        let tree = self.tree.as_mut().ok_or_else(|| {
            (
                abi::E_ERR,
                "the shell has not attached to the tree: call tree_attach first".to_owned(),
            )
        })?;
        if let Some(named) = self.named.get_mut(&key) {
            if tree.class_of(named.node).as_deref() == Some(class) {
                let adopted = named.made_by != owner;
                named.claimed_by = owner;
                named.ephemeral = ephemeral;
                return Ok((named.node, adopted));
            }
            // The shell destroyed it, or the compositor did: the name is free again.
            self.named.remove(&key);
        }
        let node = tree.create(class).map_err(|error| (abi::E_ERR, error.to_string()))?;
        tree.set(node, "name", MirrorValue::Text(name.to_owned()), TxAnimation::default())
            .map_err(|error| (abi::E_NO_SUCH_NODE, error.to_string()))?;
        self.named.insert(
            key,
            NamedNode {
                node,
                made_by: owner,
                claimed_by: owner,
                ephemeral,
            },
        );
        Ok((node, false))
    }

    /// The ephemeral nodes that `id`, which just took over, did not adopt: destroyed, as the design says a generation's
    /// ephemeral nodes die with it unless the next one takes them over.
    fn destroy_unadopted_ephemerals(&mut self, id: GenerationId) {
        let gone: Vec<(String, String)> = self
            .named
            .iter()
            .filter(|(_, named)| named.ephemeral && named.claimed_by != id)
            .map(|(key, _)| key.clone())
            .collect();
        for key in gone {
            let Some(named) = self.named.remove(&key) else {
                continue;
            };
            let Some(tree) = self.tree.as_mut() else {
                continue;
            };
            if !tree.exists(named.node) {
                continue;
            }
            match tree.destroy(named.node) {
                Ok(removed) => log::info!(
                    "destroyed the ephemeral {} `{}` (node {}, {} node(s)): generation {id} did not adopt it",
                    key.0,
                    key.1,
                    named.node,
                    removed.len()
                ),
                Err(error) => log::warn!("cannot destroy the ephemeral {} `{}`: {error}", key.0, key.1),
            }
        }
    }

    // ---- the generation lifecycle ------------------------------------------------------------------------------------

    /// A generation was made and will run its `init` against the live instance: remember what that `init` can change, to put it
    /// back if the stage is abandoned.
    fn staged(&mut self, id: GenerationId) {
        self.processes.begin_stage();
        self.stage = Some(StageMark {
            generation: id,
            tick: self.tree.as_ref().map(TreeSession::mark_tick),
            named: self.named.clone(),
            state: BTreeMap::new(),
        });
    }

    /// The stage failed or was abandoned: the writes its `init` queued, the nodes it named and the state it kept are put back.
    fn roll_back_stage(&mut self, id: GenerationId) {
        let Some(mark) = self.stage.take_if(|mark| mark.generation == id) else {
            return;
        };
        let mut restored = true;
        if let Some(tree) = self.tree.as_mut() {
            restored = match mark.tick {
                Some(tick) => tree.rollback_tick(tick),
                // The shell attached during this very `init`: everything the tick holds is the stage's.
                None => tree.discard_tick(),
            };
            if !restored {
                log::warn!(
                    "generation {id} flushed during its init before it failed: the writes it sent are the compositor's now and \
                     cannot be taken back, so the nodes it made under a name stay and the next generation finds them"
                );
            }
        }
        if restored {
            self.named = mark.named;
        }
        for (key, before) in mark.state {
            match before {
                Some(stored) => {
                    self.state.insert(key, stored);
                }
                None => {
                    self.state.remove(&key);
                }
            }
        }
        self.state_bytes = self
            .state
            .iter()
            .map(|(key, stored)| key.len() + stored.bytes.len())
            .sum();
    }

    /// A staged generation took over: the old one's claims are released, the new one's go out behind the releases in the same
    /// transaction, the programs it started begin (and the ones the old config started and it did not are stopped).
    fn activated(&mut self, id: GenerationId) {
        // The stage committed: its mark is no longer needed.
        self.stage = self.stage.take().filter(|mark| mark.generation != id);
        let old: Vec<i64> = self
            .claims
            .iter()
            .filter(|(_, claim)| claim.owner != id && claim.sent)
            .map(|(claim, _)| *claim)
            .collect();
        for claim in old {
            self.release_claim(claim);
        }
        let waiting: Vec<i64> = self
            .claims
            .iter()
            .filter(|(_, claim)| claim.owner == id && !claim.sent)
            .map(|(claim, _)| *claim)
            .collect();
        for claim in waiting {
            if let Err((_, text)) = self.send_claim(claim) {
                log::warn!("{text}");
            }
        }
        for (process, error) in self.processes.end_stage(id) {
            log::warn!("the program of process {process} did not start: {error}");
        }
        self.destroy_unadopted_ephemerals(id);
    }

    /// A generation retired. If it never took over, what it staged is dropped: its spawns never start, its claims never go out.
    fn retired(&mut self, id: GenerationId, was: GenerationState) {
        if was != GenerationState::Staged {
            return;
        }
        self.roll_back_stage(id);
        self.processes.abandon_stage();
        let unsent: Vec<i64> = self
            .claims
            .iter()
            .filter(|(_, claim)| claim.owner == id && !claim.sent)
            .map(|(claim, _)| *claim)
            .collect();
        for claim in unsent {
            self.claims.remove(&claim);
        }
    }
}

/// The shell host's lifecycle hooks, over this thread's session.
pub(crate) struct Lifecycle;

impl LifecycleObserver for Lifecycle {
    fn staged(&self, id: GenerationId) {
        with(|session| session.staged(id));
    }

    fn activated(&self, id: GenerationId, on_shell_thread: bool) {
        if on_shell_thread {
            with(|session| session.activated(id));
        }
    }

    fn retired(&self, id: GenerationId, was: GenerationState, on_shell_thread: bool) {
        if on_shell_thread {
            with(|session| session.retired(id, was));
        }
    }
}

/// The ABI code and text for a failed link: a compositor that already has a holder is `E_BUSY`, a refusal `E_REFUSED`.
fn link_failure(what: &str, text: &str) -> (i32, String) {
    let code = if text.contains("shell-already-attached") {
        abi::E_BUSY
    } else if text.contains("refused") {
        abi::E_REFUSED
    } else {
        abi::E_ERR
    };
    (code, format!("{what}: {text}"))
}

// ---- tree messages as events -------------------------------------------------------------------------------------------

/// A change of the tree as the notices registrations match: a destroyed node tells of its subtree first, deepest first, as the
/// mirror removes them. A change nobody can be told of (a node placed under a parent, a property of a kind the ABI does not carry)
/// is none.
fn node_notices(change: NodeEvent) -> Vec<NodeNotice> {
    match change {
        NodeEvent::Created { node, class } => vec![NodeNotice {
            kind: abi::NODE_CREATED,
            class: catalog::tree_class_id(&class),
            property: 0,
            node,
            parent: 0,
            value: None,
        }],
        NodeEvent::Destroyed { node, class, removed } => removed
            .into_iter()
            .chain(std::iter::once((node, class)))
            .map(|(node, class)| NodeNotice {
                kind: abi::NODE_DESTROYED,
                class: catalog::tree_class_id(&class),
                property: 0,
                node,
                parent: 0,
                value: None,
            })
            .collect(),
        NodeEvent::Changed {
            node,
            class,
            key,
            value,
        } => {
            let class = catalog::tree_class_id(&class);
            let Some(value) = OwnedValue::from_mirror(&value) else {
                return Vec::new();
            };
            vec![NodeNotice {
                kind: abi::NODE_CHANGED,
                class,
                property: catalog::property_id(class, &key).unwrap_or(0),
                node,
                parent: 0,
                value: Some(value),
            }]
        }
        // A node placed under a parent is not an event of its own in the ABI: a window's parent is the root, and a shell that
        // cares reads `node_parent`.
        NodeEvent::Parented { .. } => Vec::new(),
    }
}

/// The event a message of the tree link is, or `None` for one nobody listens to.
fn shell_event(message: TreeIncoming) -> Option<ShellEvent> {
    Some(match message {
        TreeIncoming::Answer { token, result, error } => ShellEvent::Answer(AnswerNotice { token, result, error }),
        TreeIncoming::Event(event) => {
            let class = catalog::class_id("tree")?;
            let signal = catalog::signal_id(class, "event")?;
            ShellEvent::Signal(SignalDelivery {
                class,
                signal,
                node: event.node,
                // The schema's field order: frame, seq, node, kind, x, y, button, key, text, dx, dy, request, pressed.
                values: vec![
                    OwnedValue::Uint(event.frame),
                    OwnedValue::Uint(event.seq),
                    OwnedValue::Uint(event.node),
                    OwnedValue::Text(event.kind),
                    OwnedValue::Double(event.fields.x),
                    OwnedValue::Double(event.fields.y),
                    OwnedValue::Uint(event.fields.button),
                    OwnedValue::Text(event.fields.key),
                    OwnedValue::Text(event.fields.text),
                    OwnedValue::Double(event.fields.dx),
                    OwnedValue::Double(event.fields.dy),
                    OwnedValue::Text(event.fields.request),
                    OwnedValue::Bool(event.fields.pressed),
                ],
            })
        }
        TreeIncoming::TrackEnded {
            node,
            property,
            completed,
            tween,
        } => {
            let class = catalog::class_id("tree")?;
            let signal = catalog::signal_id(class, "track_ended")?;
            ShellEvent::Signal(SignalDelivery {
                class,
                signal,
                node,
                values: vec![
                    OwnedValue::Uint(node),
                    OwnedValue::Text(property),
                    OwnedValue::Text(if completed { "completed" } else { "cancelled" }.to_owned()),
                    OwnedValue::Uint(tween.unwrap_or(0)),
                ],
            })
        }
        TreeIncoming::Chord(chord) => ShellEvent::Claim(ClaimNotice {
            kind: abi::CLAIM_EVENT_CHORD,
            claim: chord.claim,
            state: chord.state,
            text: chord.chord,
            ..claim_blank()
        }),
        TreeIncoming::Gesture(gesture) => ShellEvent::Claim(ClaimNotice {
            kind: abi::CLAIM_EVENT_GESTURE,
            claim: gesture.claim,
            state: gesture.phase,
            text: gesture.kind,
            fingers: gesture.fingers,
            dx: gesture.dx,
            dy: gesture.dy,
            scale: gesture.scale,
            rotation: gesture.rotation,
            ..claim_blank()
        }),
        TreeIncoming::PointerChord(drag) => ShellEvent::Claim(ClaimNotice {
            kind: abi::CLAIM_EVENT_POINTER_CHORD,
            claim: drag.claim,
            state: drag.phase,
            text: drag.modifiers,
            button: drag.button,
            x: drag.x,
            y: drag.y,
            dx: drag.dx,
            dy: drag.dy,
            time: drag.time,
            ..claim_blank()
        }),
        TreeIncoming::PointerButton(button) => ShellEvent::Claim(ClaimNotice {
            kind: abi::CLAIM_EVENT_POINTER_BUTTON,
            claim: button.claim,
            state: button.state,
            button: button.button,
            x: button.x,
            y: button.y,
            time: button.time,
            ..claim_blank()
        }),
        // The tree session consumes these.
        TreeIncoming::Attached(_)
        | TreeIncoming::Snapshot { .. }
        | TreeIncoming::Changes { .. }
        | TreeIncoming::Reply(_) => {
            return None;
        }
    })
}

fn claim_blank() -> ClaimNotice {
    ClaimNotice {
        kind: 0,
        claim: 0,
        state: String::new(),
        text: String::new(),
        fingers: 0,
        button: 0,
        x: 0.0,
        y: 0.0,
        dx: 0.0,
        dy: 0.0,
        scale: 0.0,
        rotation: 0.0,
        time: 0,
    }
}

fn exit_notice(process: u64, exit: ProcessExit) -> ExitNotice {
    ExitNotice {
        process,
        code: exit.exit_code,
        signal: exit.signal,
        timed_out: exit.timed_out,
        stdout: exit.stdout,
        stderr: exit.stderr,
        truncated: u32::from(exit.stdout_truncated) | (u32::from(exit.stderr_truncated) << 1),
    }
}

#[cfg(test)]
mod tests {
    use gnoblin_tree_client::tree_mirror::{EventFields, TreeEvent};

    use super::*;

    #[test]
    fn the_event_signal_is_built_in_the_schemas_field_order() {
        let ShellEvent::Signal(delivery) = shell_event(TreeIncoming::Event(TreeEvent {
            frame: 3,
            seq: 4,
            node: 9,
            kind: "key".to_owned(),
            fields: EventFields::default(),
        }))
        .expect("an event") else {
            panic!("the event is a signal");
        };
        let class = catalog::class(delivery.class).unwrap();
        let signal = catalog::signal_def(delivery.class, delivery.signal).unwrap();
        assert_eq!((class.name, signal.name), ("tree", "event"));
        assert_eq!(delivery.values.len(), signal.args.len(), "one value per schema field");
        let kind_of = |value: &OwnedValue| match value {
            OwnedValue::Uint(_) => abi::VALUE_UINT,
            OwnedValue::Double(_) => abi::VALUE_DOUBLE,
            OwnedValue::Text(_) => abi::VALUE_TEXT,
            OwnedValue::Bool(_) => abi::VALUE_BOOL,
            other => panic!("{other:?}"),
        };
        let built: Vec<u32> = delivery.values.iter().map(kind_of).collect();
        assert_eq!(built, signal.args, "the kinds are the schema's, in its order");
    }

    #[test]
    fn the_track_end_signal_is_built_in_the_schemas_field_order() {
        let ShellEvent::Signal(delivery) = shell_event(TreeIncoming::TrackEnded {
            node: 5,
            property: "x".to_owned(),
            completed: true,
            tween: None,
        })
        .expect("a track end") else {
            panic!("a signal");
        };
        let signal = catalog::signal_def(delivery.class, delivery.signal).unwrap();
        assert_eq!(signal.name, "track_ended");
        assert_eq!(
            delivery.values,
            [
                OwnedValue::Uint(5),
                OwnedValue::Text("x".to_owned()),
                OwnedValue::Text("completed".to_owned()),
                OwnedValue::Uint(0)
            ]
        );
        assert_eq!(
            signal.args,
            [abi::VALUE_UINT, abi::VALUE_TEXT, abi::VALUE_TEXT, abi::VALUE_UINT]
        );
    }

    #[test]
    fn a_timer_fires_once_and_a_repeating_one_again() {
        use gnoblin_shell_host::supervisor::ManualClock;
        let clock = ManualClock::default();
        let mut session = Session::default();
        session.set_clock(Arc::new(clock.clone()));
        let once = session.add_timer(1, 100, false);
        let again = session.add_timer(1, 40, true);
        assert!(session.take_due(|_| true).is_empty(), "nothing is due at the start");
        assert_eq!(session.next_due_in(|_| true), Some(Duration::from_millis(40)));
        clock.advance(Duration::from_millis(40));
        assert_eq!(session.take_due(|_| true), [again]);
        clock.advance(Duration::from_millis(60));
        assert_eq!(session.take_due(|_| true), [once, again]);
        assert_eq!(
            session.take_due(|_| true),
            Vec::<u64>::new(),
            "the one-shot is spent and the repeat waits a period"
        );
        // A generation that is not active fires nothing and keeps its timers.
        clock.advance(Duration::from_millis(500));
        assert!(session.take_due(|_| false).is_empty());
        assert_eq!(session.take_due(|_| true), [again]);
    }

    #[test]
    fn the_state_store_is_bounded_and_replaces_in_place() {
        let mut session = Session::default();
        session.put_state("layout", vec![1; 10], 2).unwrap();
        assert_eq!(session.get_state("layout"), Some((vec![1; 10], 2)));
        session.put_state("layout", vec![2; 4], 3).unwrap();
        assert_eq!(session.get_state("layout"), Some((vec![2; 4], 3)));
        assert!(session.put_state("big", vec![0; STATE_LIMIT], 1).is_err());
        assert_eq!(session.get_state("big"), None, "a refused put keeps nothing");
    }
}
