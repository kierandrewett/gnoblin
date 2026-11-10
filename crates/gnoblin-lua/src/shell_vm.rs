//! Shell VM: the shell's Lua runtime over the real tree core.
//!
//! This module boots `data/lua/gnoblin/node.lua` against `tree_core`, keeps the
//! mirror in step with the compositor through a `TreeLink`, and sends each
//! frame's writes as one transaction. It is NOT yet loaded by the runtime role;
//! wiring it into the shell is a later step.
//!
//! The link must answer the attach with the snapshot in its `poll` results: the
//! VM cannot load `node.lua` without a mirror, so `ShellVm::new` drains the link
//! until the last snapshot chunk arrives.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use mlua::{Function, Lua, Table, Value};

use crate::tree_core::{self, ClassSpec, Shared, TreeCore};
use crate::tree_mirror::{
    LuaChange, LuaChord, LuaIncoming, LuaMirror, LuaOp, MirrorChangeKind, NodeId, SnapshotNode, TreeLink,
};

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
    /// Messages that arrived with the boot snapshot, handled by the first frame.
    incoming: Vec<LuaIncoming>,
    /// The ops of each sent transaction, by serial, until its reply arrives. A refusal names its
    /// failed op from this record, and a stale-base refusal re-sends the rest.
    sent: BTreeMap<u64, Vec<LuaOp>>,
    /// Serial of the one transaction in flight. Nothing else is sent until the mirror reaches it:
    /// a second transaction built on the same base is refused as stale.
    in_flight: Option<u64>,
    /// Ops refused `stale-base` that are sent again, ahead of the next tick's writes, once the
    /// mirror reaches the refused serial.
    retry: Vec<LuaOp>,
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
    /// Boots node.lua from `lua_dir` over the real core and attaches `link`.
    ///
    /// Fails when the link does not deliver a complete snapshot in its first poll.
    pub fn new(mut link: Box<dyn TreeLink>, lua_dir: &Path, classes: Vec<ClassSpec>) -> Result<Self> {
        link.attach(ATTACH_VERSION).map_err(|e| ShellVmError::Link(e.0))?;
        let (mirror, incoming) = Self::boot_mirror(link.as_mut())?;
        let core = TreeCore::shared(mirror, classes).map_err(ShellVmError::Core)?;

        let lua = Lua::new();
        let core_table = tree_core::install(&lua, &core).map_err(lua_error)?;
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
            incoming,
            in_flight: None,
            sent: BTreeMap::new(),
            retry: Vec::new(),
        })
    }

    /// Runs `src` as a chunk named `name` and returns its result, if it has one.
    pub fn run(&mut self, name: &str, src: &str) -> Result<Option<String>> {
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
                    let ops = self.sent.remove(&reply.serial).unwrap_or_default();
                    if !reply.applied && reply.code == "stale-base" && reply.failed_op >= 0 {
                        // A published batch changed a node this transaction writes. The refused
                        // transaction applied nothing, so its other ops are sent again on the next
                        // base; the failed op is dropped, since the published change wins. The gate
                        // stays shut until the mirror reaches this serial, which that batch does.
                        let failed = usize::try_from(reply.failed_op).ok();
                        let mut kept: Vec<LuaOp> = ops
                            .into_iter()
                            .enumerate()
                            .filter(|(index, _)| Some(*index) != failed)
                            .map(|(_, op)| op)
                            .collect();
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
                        if self.in_flight == Some(reply.serial) && !reply.applied {
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
                                .and_then(|i| ops.get(i).map(|op| describe_op(i, op, self.core.borrow().mirror())))
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
                }),
                LuaIncoming::Chord(chord) => tree_events.push(LuaEvent {
                    target: 0,
                    kind: "chord".to_owned(),
                    bubbles: false,
                    chord: Some(chord),
                }),
                LuaIncoming::Attached(_) | LuaIncoming::Snapshot { .. } => {
                    return Err(ShellVmError::Link("attach or snapshot after boot".to_owned()));
                }
                LuaIncoming::Answer { token, error, .. } => {
                    diagnostics.push(format!("unexpected answer {token}: {error}"));
                }
            }
        }

        let wake = self.tick_lua(now, &touched, &created, events.into_iter().chain(tree_events))?;
        diagnostics.extend(self.core.borrow_mut().take_reports());

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
                // Ops refused stale-base go first, on this base.
                let mut ops = std::mem::take(&mut self.retry);
                ops.append(&mut tx.ops);
                tx.ops = ops;
            }
            if !tx.ops.is_empty() {
                self.sent.insert(tx.serial, tx.ops.clone());
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
            if let Some(chord) = &event.chord {
                let claim = lua.create_table().map_err(lua_error)?;
                claim.set("claim", chord.claim).map_err(lua_error)?;
                claim.set("chord", chord.chord.as_str()).map_err(lua_error)?;
                claim.set("state", chord.state.as_str()).map_err(lua_error)?;
                entry.set("chord", claim).map_err(lua_error)?;
            }
            events_table.set(i + 1, entry).map_err(lua_error)?;
        }
        self.tick
            .call((now, changes, events_table, created_table))
            .map_err(lua_error)
    }
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
