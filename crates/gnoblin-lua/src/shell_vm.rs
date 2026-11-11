//! Shell VM: the shell's Lua runtime over the real tree core.
//!
//! This module boots `data/lua/gnoblin/node.lua` against `tree_core`, keeps the
//! mirror in step with the compositor through a `TreeLink`, and sends each
//! frame's writes as one transaction. The runtime role runs it by default (see
//! `shell_role.rs`); `GNOBLIN_SHELL_VM=0` turns it off.
//!
//! The link must answer the attach with the snapshot in its `poll` results: the
//! VM cannot load `node.lua` without a mirror, so `ShellVm::new` drains the link
//! until the last snapshot chunk arrives.
//!
//! The Lua state is the sandbox's configuration profile, the one the config host runs
//! scripts in: no `io`, `debug`, `package`, `dofile` or `loadfile`, and `load` accepts text
//! only. `require` answers only from the stock library; the user's shell files get their own `require`
//! over the config directory when they load (see `shell_scripts.rs`). Every entry into Lua has a budget
//! (`ShellLimits`): the frame as a whole, and each handler, binding or timer inside it. A handler that
//! runs away is stopped and reported by name, and the frame goes on with the next one.

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gnoblin_lua_sandbox::{Guard, Limits, Profile};
use mlua::{Function, Lua, MultiValue, Table, Thread, Value};

use crate::require::{ModuleRoots, Modules};
use crate::tree_core::{self, ClassSpec, Shared, TreeCore};
use crate::tree_mirror::{
    LuaChange, LuaChord, LuaIncoming, LuaMirror, LuaOp, MirrorChangeKind, NodeId, SnapshotNode, TreeLink,
};

/// The budgets the shell VM runs under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShellLimits {
    /// What one handler, binding or timer may use. A handler that goes past it is stopped and
    /// reported, and the frame carries on.
    pub entry: Limits,
    /// What one whole frame may use, all of its entries together. It stops a frame made of
    /// many slow entries, and a loop in the shell's own code; a frame that goes past it ends the VM.
    pub frame: Limits,
    /// What loading the stock shell may use, and what each of the user's shell files may use on its own.
    /// A file that goes past it is stopped and reported, and the next file loads.
    pub load: Limits,
}

impl Default for ShellLimits {
    fn default() -> Self {
        Self {
            // A handler should finish inside a frame.
            entry: Limits {
                instructions: 10_000_000,
                memory_bytes: 256 * 1024 * 1024,
                handler_ticks: 100,
            },
            frame: Limits {
                instructions: 100_000_000,
                memory_bytes: 256 * 1024 * 1024,
                handler_ticks: 500,
            },
            load: Limits {
                instructions: 400_000_000,
                memory_bytes: 256 * 1024 * 1024,
                handler_ticks: 2_000,
            },
        }
    }
}

/// The same report text is logged at most once in this many seconds of the frame clock, so a
/// handler that fails every frame does not fill the log.
pub const REPORT_INTERVAL: f64 = 5.0;

/// The protocol version the VM attaches with.
const ATTACH_VERSION: u64 = 1;

/// How long the boot waits for the compositor's attach reply and its complete snapshot. The
/// compositor is another process, so the reply can trail the attach by more than one poll.
const BOOT_WAIT: Duration = Duration::from_secs(5);
/// The pause between boot polls while the reply is still on its way.
const BOOT_POLL: Duration = Duration::from_millis(5);

/// Result alias for shell VM operations.
pub type Result<T> = std::result::Result<T, ShellVmError>;

/// Errors raised by the shell VM.
#[derive(Debug)]
pub enum ShellVmError {
    Load(String),
    Lua(String),
    Link(String),
    Mirror(String),
    Core(String),
}

impl std::fmt::Display for ShellVmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Load(m) => write!(f, "shell vm load: {m}"),
            Self::Lua(m) => write!(f, "shell vm lua: {m}"),
            Self::Link(m) => write!(f, "shell vm link: {m}"),
            Self::Mirror(m) => write!(f, "shell vm mirror: {m}"),
            Self::Core(m) => write!(f, "shell vm core: {m}"),
        }
    }
}

impl std::error::Error for ShellVmError {}

/// An input event delivered to node.lua's `events` argument.
#[derive(Debug, Clone, PartialEq)]
pub struct LuaEvent {
    pub target: u64,
    pub kind: String,
    pub bubbles: bool,
    /// A claimed chord the compositor reports; node.lua routes it to the seat that made the claim.
    pub chord: Option<LuaChord>,
    /// Positional strings: an `answer` event carries the call's result and its error, else empty.
    pub args: Vec<String>,
}

/// What one frame produced for the caller.
#[derive(Debug, Default, PartialEq)]
pub struct ShellFrame {
    /// When node.lua wants to run again without input, on the frame clock.
    pub wake: Option<f64>,
    /// Script reports, refused transactions and unexpected messages, in order.
    pub diagnostics: Vec<String>,
}

/// The shell's Lua VM over the real tree core.
pub struct ShellVm {
    link: Box<dyn TreeLink>,
    lua: Lua,
    core: Shared,
    tick: Function,
    guard: Rc<Guard>,
    limits: ShellLimits,
    /// When each report text was last passed on, and how many copies were held back since.
    reported: BTreeMap<String, (f64, u64)>,
    /// Messages that arrived with the boot snapshot, handled by the first frame.
    incoming: Vec<LuaIncoming>,
    /// The serial and ops of each sent transaction, oldest first, until its reply arrives. The compositor
    /// answers in order, but a refusal carries the serial the tree had then, not the serial the shell
    /// stamped, so a reply is matched to its transaction by position. A refusal names its failed op from
    /// this record, and a stale-base or unknown-node refusal re-sends the rest. A call that carries a
    /// secret is recorded as `None`: its text is not kept past the send, so a retry cannot send it again.
    sent: VecDeque<(u64, Vec<Option<LuaOp>>)>,
    /// Serial of the one transaction in flight. Nothing else is sent until the mirror reaches it:
    /// a second transaction built on the same base is refused as stale.
    in_flight: Option<u64>,
    /// Ops refused `stale-base` that are sent again, ahead of the next tick's writes, once the
    /// mirror reaches the refused serial.
    retry: Vec<LuaOp>,
}

/// The calls whose text is content and not a secret: a `Canvas.measure` carries the string to measure, and a
/// `Seat.claim` the chord to grab. Every other call that carries text is treated as carrying a password.
const PLAIN_TEXT_CALLS: &[&str] = &["measure", "claim"];

/// What the sent-log keeps of one op for a stale-base retry. A call that carries a secret is not kept: its text
/// is not held past the send, so a retry cannot send it again. A call whose text is plain content is kept, so a
/// race with a published batch does not leave a text unmeasured or a chord unclaimed for good.
fn retained_op(op: &LuaOp) -> Option<LuaOp> {
    match op {
        LuaOp::Call { method, text, .. } if !text.is_empty() && !PLAIN_TEXT_CALLS.contains(&method.as_str()) => None,
        other => Some(other.clone()),
    }
}

/// Names one op for a refusal: its index, class, property and value kind, so a refused write is
/// traceable to the Lua call that made it.
pub fn describe_op(index: usize, op: &LuaOp, mirror: &LuaMirror) -> String {
    let class_of = |node: NodeId| {
        mirror
            .node(node)
            .map_or_else(|| "unknown".to_owned(), |record| record.class.clone())
    };
    match op {
        LuaOp::New { id, class } => format!("op {index}: new node {id} class {class}"),
        LuaOp::Set { node, key, value, .. } => format!(
            "op {index}: set node {node} ({}) property {key} value kind {}",
            class_of(*node),
            value.kind_name()
        ),
        LuaOp::Insert {
            node,
            parent,
            index: at,
        } => {
            format!(
                "op {index}: insert node {node} ({}) into {parent} at {at}",
                class_of(*node)
            )
        }
        LuaOp::Destroy { node } => format!("op {index}: destroy node {node} ({})", class_of(*node)),
        LuaOp::Call { node, method, .. } => {
            format!("op {index}: call {method} on node {node} ({})", class_of(*node))
        }
    }
}

impl ShellVm {
    /// Boots node.lua from `lua_dir` over the real core and attaches `link`, under the default budgets.
    ///
    /// Fails when the link does not deliver a complete snapshot in its first poll.
    pub fn new(link: Box<dyn TreeLink>, lua_dir: &Path, classes: Vec<ClassSpec>) -> Result<Self> {
        Self::with_limits(link, lua_dir, classes, ShellLimits::default())
    }

    /// Boots like [`ShellVm::new`], with the given budgets.
    pub fn with_limits(
        mut link: Box<dyn TreeLink>,
        lua_dir: &Path,
        classes: Vec<ClassSpec>,
        limits: ShellLimits,
    ) -> Result<Self> {
        link.attach(ATTACH_VERSION).map_err(|e| ShellVmError::Link(e.0))?;
        let (mirror, incoming) = Self::boot_mirror(link.as_mut())?;
        let core = TreeCore::shared(mirror, classes).map_err(ShellVmError::Core)?;

        let guard = Rc::new(Guard::new());
        guard.begin(&limits.frame);
        let lua = gnoblin_lua_sandbox::create(
            Profile::Config,
            &limits.entry,
            Rc::clone(&guard),
            crate::host::script_sink(),
        )
        .map_err(lua_error)?;
        let core_table = tree_core::install(&lua, &core).map_err(lua_error)?;
        install_entries(&lua, &core_table, &guard, limits.entry).map_err(lua_error)?;
        let shipped = lua_dir.parent().unwrap_or(lua_dir).to_path_buf();
        install_require(&lua, Modules::new(ModuleRoots { shipped, user: None })).map_err(lua_error)?;
        let path = lua_dir.join("node.lua");
        let source =
            std::fs::read_to_string(&path).map_err(|e| ShellVmError::Load(format!("{}: {e}", path.display())))?;
        let tick: Function = lua
            .load(source.as_str())
            .set_name("node.lua")
            .call(core_table)
            .map_err(lua_error)?;

        Ok(Self {
            link,
            lua,
            core,
            tick,
            guard,
            limits,
            reported: BTreeMap::new(),
            incoming,
            in_flight: None,
            sent: VecDeque::new(),
            retry: Vec::new(),
        })
    }

    /// Loads the stock shell (`require("gnoblin.shell")`) from `stock`, the directory holding `node.lua`, and then the
    /// user's shell files from the config directory `user`: `shell.lua`, then `shell/*.lua`. Both run before the first
    /// frame, the stock shell under `ShellLimits::load` as a whole and each user file under it on its own. The state is
    /// the sandbox's, so neither can reach the host; the user's files also run in an environment of their own (see
    /// `shell_scripts.rs`).
    ///
    /// Returns the errors the user's files raised, in load order; each was also logged and none stops the VM. An error
    /// loading the stock shell is `Err`: the VM has no shell and the caller decides what to do.
    pub fn load_shell(&mut self, stock: &Path, user: Option<&Path>) -> Result<Vec<String>> {
        let budget = crate::shell_scripts::LoadBudget {
            guard: &self.guard,
            limits: self.limits.load,
        };
        crate::shell_scripts::load(&self.lua, stock, user, &budget).map_err(ShellVmError::Load)
    }

    /// Runs `src` as a chunk named `name` and returns its result, if it has one. The chunk runs
    /// under the frame's budget.
    pub fn run(&mut self, name: &str, src: &str) -> Result<Option<String>> {
        self.guard.begin(&self.limits.frame);
        let value: Value = self.lua.load(src).set_name(name).eval().map_err(lua_error)?;
        match value {
            Value::Nil => Ok(None),
            other => {
                let tostring: Function = self.lua.globals().get("tostring").map_err(lua_error)?;
                let text: String = tostring.call(other).map_err(lua_error)?;
                Ok(Some(text))
            }
        }
    }

    /// Runs one frame: applies what the compositor sent, ticks node.lua with the
    /// frame clock and `events`, then sends this frame's writes as one transaction.
    pub fn frame(&mut self, now: f64, events: Vec<LuaEvent>) -> Result<ShellFrame> {
        let mut diagnostics = Vec::new();
        let mut touched: Vec<(u64, String)> = Vec::new();
        let mut created: Vec<u64> = Vec::new();
        let mut tree_events: Vec<LuaEvent> = Vec::new();

        let mut incoming = std::mem::take(&mut self.incoming);
        incoming.extend(self.link.poll().map_err(|e| ShellVmError::Link(e.0))?);
        for message in incoming {
            match message {
                LuaIncoming::Changes { serial, changes } => {
                    // A closed node runs its `destroyed` handlers before node.lua drops its face.
                    // Every node the change took is told, the deepest first as `Node:destroy` does: a titlebar
                    // under a closed window must stop its bindings, not read a node that is gone.
                    tree_events.extend(
                        changes
                            .iter()
                            .filter(|c| c.kind == MirrorChangeKind::Destroyed)
                            .flat_map(|c| {
                                let subtree = c.removed.iter().copied().filter(|id| *id != c.node);
                                subtree.rev().chain(std::iter::once(c.node))
                            })
                            .map(|node| LuaEvent {
                                target: node,
                                kind: "destroyed".to_owned(),
                                bubbles: false,
                                chord: None,
                                args: Vec::new(),
                            }),
                    );
                    self.apply_changes(serial, &changes)?;
                    touched.extend(
                        changes
                            .iter()
                            .filter(|c| c.kind == MirrorChangeKind::Set)
                            .map(|c| (c.node, c.key.clone())),
                    );
                    created.extend(
                        changes
                            .iter()
                            .filter(|c| c.kind == MirrorChangeKind::Created)
                            .map(|c| c.node),
                    );
                }
                LuaIncoming::Reply(reply) => {
                    let (sent_serial, ops) = self.sent.pop_front().unwrap_or((reply.serial, Vec::new()));
                    let resend = matches!(reply.code.as_str(), "stale-base" | "unknown-node");
                    if !reply.applied && resend && reply.failed_op >= 0 {
                        // A published batch changed a node this transaction writes (stale-base), or
                        // destroyed it (unknown-node). The refused transaction applied nothing, so its
                        // other ops are sent again on the next base; the failed op is dropped, since
                        // the published change wins. The gate stays shut until the mirror reaches
                        // this serial, which that batch does.
                        let failed = usize::try_from(reply.failed_op).ok();
                        let mut kept: Vec<LuaOp> = Vec::new();
                        for (index, op) in ops.into_iter().enumerate() {
                            if Some(index) == failed {
                                continue;
                            }
                            match op {
                                Some(op) => kept.push(op),
                                // Its text was not kept, so it is not sent again. A password
                                // is never re-sent; the diagnostic says which call was dropped.
                                None => diagnostics.push(format!(
                                    "transaction {} stale at op {}: text call {index} was dropped and not sent again",
                                    reply.serial, reply.failed_op
                                )),
                            }
                        }
                        crate::log::debug!(
                            area = "shell",
                            "transaction {} stale at op {}; {} ops sent again",
                            reply.serial,
                            reply.failed_op,
                            kept.len()
                        );
                        kept.append(&mut self.retry);
                        self.retry = kept;
                    } else {
                        if self.in_flight == Some(sent_serial) && !reply.applied {
                            // A refused transaction never advances the serial, so the gate opens now.
                            self.in_flight = None;
                        }
                        crate::log::debug!(
                            area = "shell",
                            "transaction {} applied={} code={} failed_op={}",
                            reply.serial,
                            reply.applied,
                            reply.code,
                            reply.failed_op
                        );
                        if !reply.applied {
                            let op = usize::try_from(reply.failed_op)
                                .ok()
                                .and_then(|i| {
                                    ops.get(i).map(|op| match op {
                                        Some(op) => describe_op(i, op, self.core.borrow().mirror()),
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
                }
                LuaIncoming::Event(event) => tree_events.push(LuaEvent {
                    target: event.node,
                    kind: event.kind,
                    bubbles: false,
                    chord: None,
                    args: Vec::new(),
                }),
                LuaIncoming::Chord(chord) => tree_events.push(LuaEvent {
                    target: 0,
                    kind: "chord".to_owned(),
                    bubbles: false,
                    chord: Some(chord),
                    args: Vec::new(),
                }),
                LuaIncoming::Attached(_) | LuaIncoming::Snapshot { .. } => {
                    return Err(ShellVmError::Link("attach or snapshot after boot".to_owned()));
                }
                LuaIncoming::Answer { token, result, error } => {
                    let node = self.core.borrow_mut().take_call_node(token);
                    match node {
                        Some(target) => tree_events.push(LuaEvent {
                            target,
                            kind: token.to_string(),
                            bubbles: false,
                            chord: None,
                            args: vec![String::from_utf8_lossy(&result).into_owned(), error],
                        }),
                        None => diagnostics.push(format!("answer {token} has no pending call")),
                    }
                }
            }
        }

        let settled = self.sent.is_empty() && self.retry.is_empty();
        self.core.borrow_mut().begin_tick(settled);
        // A node a later batch of this frame destroyed has nothing to wrap or to touch: its destroy event says so.
        {
            let core = self.core.borrow();
            let mirror = core.mirror();
            touched.retain(|(node, _)| mirror.node(*node).is_some());
            created.retain(|node| mirror.node(*node).is_some());
        }
        let wake = self.tick_lua(now, &touched, &created, events.into_iter().chain(tree_events))?;
        let reports = self.core.borrow_mut().take_reports();
        diagnostics.extend(self.throttle(now, reports));

        let ready = self.in_flight.is_none_or(|s| self.core.borrow().mirror().serial() >= s);
        let tx = if ready {
            let mut core = self.core.borrow_mut();
            let serial = core.mirror_mut().serial();
            Some(core.take_transaction(serial + 1))
        } else {
            None
        };
        if let Some(mut tx) = tx {
            if !self.retry.is_empty() {
                // Ops refused stale-base go first, on this base. An op on a node the tree has lost since is
                // moot, and sending it would refuse the whole transaction again.
                let mut ops = std::mem::take(&mut self.retry);
                let core = self.core.borrow();
                let mirror = core.mirror();
                let mut made: Vec<NodeId> = Vec::new();
                ops.retain(|op| match op {
                    LuaOp::New { id, .. } => {
                        made.push(*id);
                        true
                    }
                    LuaOp::Set { node, .. } | LuaOp::Destroy { node } | LuaOp::Call { node, .. } => {
                        made.contains(node) || mirror.node(*node).is_some()
                    }
                    LuaOp::Insert { node, parent, .. } => {
                        (made.contains(node) || mirror.node(*node).is_some())
                            && (made.contains(parent) || mirror.node(*parent).is_some())
                    }
                });
                drop(core);
                ops.append(&mut tx.ops);
                tx.ops = ops;
            }
            if !tx.ops.is_empty() {
                self.sent
                    .push_back((tx.serial, tx.ops.iter().map(retained_op).collect()));
                self.in_flight = Some(tx.serial);
                self.link.transact(&tx).map_err(|e| ShellVmError::Link(e.0))?;
            }
        }

        Ok(ShellFrame { wake, diagnostics })
    }

    /// Drains the link until the complete snapshot has arrived, building the mirror from it.
    /// Anything else that arrives meanwhile is kept for the first frame.
    fn boot_mirror(link: &mut dyn TreeLink) -> Result<(LuaMirror, Vec<LuaIncoming>)> {
        let mut attached = None;
        let mut nodes: Vec<SnapshotNode> = Vec::new();
        let mut snapshot_serial = 0;
        let mut rest = Vec::new();
        let mut complete = false;
        let deadline = Instant::now() + BOOT_WAIT;
        loop {
            for message in link.poll().map_err(|e| ShellVmError::Link(e.0))? {
                match message {
                    LuaIncoming::Attached(a) => attached = Some(a),
                    LuaIncoming::Snapshot {
                        serial,
                        last,
                        nodes: chunk,
                        ..
                    } => {
                        snapshot_serial = serial;
                        nodes.extend(chunk);
                        if last {
                            complete = true;
                        }
                    }
                    other => rest.push(other),
                }
            }
            if (attached.is_some() && complete) || Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(BOOT_POLL);
        }
        let attached = attached.ok_or_else(|| ShellVmError::Link("compositor did not answer attach".to_owned()))?;
        if !complete {
            return Err(ShellVmError::Link("snapshot incomplete".to_owned()));
        }
        let mirror = LuaMirror::from_snapshot(attached.epoch, attached.root, snapshot_serial, &nodes)
            .map_err(|e| ShellVmError::Mirror(format!("{e:?}")))?;
        Ok((mirror, rest))
    }

    fn apply_changes(&mut self, serial: u64, changes: &[LuaChange]) -> Result<()> {
        self.core
            .borrow_mut()
            .mirror_mut()
            .apply_changes(serial, changes)
            .map(|_| ())
            .map_err(|e| ShellVmError::Mirror(format!("{e:?}")))
    }

    fn tick_lua(
        &self,
        now: f64,
        touched: &[(u64, String)],
        created: &[u64],
        events: impl Iterator<Item = LuaEvent>,
    ) -> Result<Option<f64>> {
        let lua = &self.lua;
        let changes = lua.create_table().map_err(lua_error)?;
        for (i, (node, key)) in touched.iter().enumerate() {
            let entry = lua.create_table().map_err(lua_error)?;
            entry.set(1, *node as i64).map_err(lua_error)?;
            entry.set(2, key.as_str()).map_err(lua_error)?;
            changes.set(i + 1, entry).map_err(lua_error)?;
        }
        let created_table = lua.create_table().map_err(lua_error)?;
        for (i, node) in created.iter().enumerate() {
            created_table.set(i + 1, *node as i64).map_err(lua_error)?;
        }
        let events_table: Table = lua.create_table().map_err(lua_error)?;
        for (i, event) in events.enumerate() {
            let entry = lua.create_table().map_err(lua_error)?;
            entry.set("target", event.target as i64).map_err(lua_error)?;
            entry.set("type", event.kind.as_str()).map_err(lua_error)?;
            entry.set("bubbles", event.bubbles).map_err(lua_error)?;
            let args = lua.create_table().map_err(lua_error)?;
            for (j, arg) in event.args.iter().enumerate() {
                args.set(j + 1, arg.as_str()).map_err(lua_error)?;
            }
            entry.set("args", args).map_err(lua_error)?;
            if let Some(chord) = &event.chord {
                let claim = lua.create_table().map_err(lua_error)?;
                claim.set("claim", chord.claim).map_err(lua_error)?;
                claim.set("chord", chord.chord.as_str()).map_err(lua_error)?;
                claim.set("state", chord.state.as_str()).map_err(lua_error)?;
                entry.set("chord", claim).map_err(lua_error)?;
            }
            events_table.set(i + 1, entry).map_err(lua_error)?;
        }
        self.guard.begin(&self.limits.frame);
        self.tick
            .call((now, changes, events_table, created_table))
            .map_err(lua_error)
    }

    /// Passes on each distinct report text at most once per [`REPORT_INTERVAL`] seconds. The
    /// first copy after the interval says how many were held back, so a handler that fails every
    /// frame costs one log line per interval, and the count shows it is still failing.
    fn throttle(&mut self, now: f64, reports: Vec<String>) -> Vec<String> {
        let mut passed = Vec::new();
        for report in reports {
            match self.reported.get_mut(&report) {
                Some((at, held)) if now - *at < REPORT_INTERVAL => *held += 1,
                Some((at, held)) => {
                    let text = match *held {
                        0 => report.clone(),
                        n => format!("{report} (repeated {n} more times since it was last reported)"),
                    };
                    *at = now;
                    *held = 0;
                    passed.push(text);
                }
                None => {
                    self.reported.insert(report.clone(), (now, 0));
                    passed.push(report);
                }
            }
        }
        self.reported.retain(|_, (at, _)| now - *at < REPORT_INTERVAL * 12.0);
        passed
    }
}

/// Puts `resume` and `pcall` on the core table: protected calls that give the call its own
/// budget. `node.lua` starts every handler, binding and timer through one of them, so a loop in
/// one of them is stopped, reported and cleared, and the rest of the frame is not aborted with it.
/// Both answer as the Lua functions of those names do: `true` and the results, or `false` and the
/// message.
fn install_entries(lua: &Lua, core: &Table, guard: &Rc<Guard>, entry: Limits) -> mlua::Result<()> {
    fn answer(lua: &Lua, result: mlua::Result<MultiValue>) -> mlua::Result<MultiValue> {
        match result {
            Ok(values) => Ok(std::iter::once(Value::Boolean(true)).chain(values).collect()),
            Err(error) => {
                let message = match error {
                    mlua::Error::RuntimeError(message) => message,
                    other => other.to_string(),
                };
                Ok([Value::Boolean(false), Value::String(lua.create_string(&message)?)]
                    .into_iter()
                    .collect())
            }
        }
    }
    let guarded = Rc::clone(guard);
    core.set(
        "resume",
        lua.create_function(move |lua, (thread, args): (Thread, MultiValue)| {
            let (result, _) = guarded.scoped(&entry, || thread.resume::<MultiValue>(args));
            answer(lua, result)
        })?,
    )?;
    let guarded = Rc::clone(guard);
    core.set(
        "pcall",
        lua.create_function(move |lua, (function, args): (Function, MultiValue)| {
            let thread = lua.create_thread(function)?;
            let (result, _) = guarded.scoped(&entry, || thread.resume::<MultiValue>(args));
            answer(lua, result)
        })?,
    )
}

/// Replaces the cleared `require` with the module loader: a dotted name is a file under the
/// stock library or the user's directory, `a.b` being `a/b.lua` or `a/b/init.lua`, read as text
/// and run once. Nothing outside those two roots can be named.
pub(crate) fn install_require(lua: &Lua, modules: Modules) -> mlua::Result<()> {
    let modules = Rc::new(modules);
    lua.globals().set(
        "require",
        lua.create_function(move |lua, name: String| modules.fetch(lua, &name).map(|(value, _)| value))?,
    )
}

fn lua_error(e: mlua::Error) -> ShellVmError {
    ShellVmError::Lua(e.to_string())
}

#[cfg(test)]
mod refusal_tests {
    use super::describe_op;
    use crate::tree_mirror::{LuaMirror, LuaOp, LuaValue};

    #[test]
    fn a_refused_set_names_its_index_class_property_and_value_kind() {
        let mirror = LuaMirror::default();
        let op = LuaOp::Set {
            node: 7,
            key: "width".to_owned(),
            value: LuaValue::Text("wide".to_owned()),
            seconds: None,
            samples: None,
        };
        assert_eq!(
            describe_op(32, &op, &mirror),
            "op 32: set node 7 (unknown) property width value kind text"
        );
    }

    #[test]
    fn a_refused_new_names_its_class() {
        let op = LuaOp::New {
            id: 9,
            class: "view".to_owned(),
        };
        assert_eq!(
            describe_op(0, &op, &LuaMirror::default()),
            "op 0: new node 9 class view"
        );
    }
}
