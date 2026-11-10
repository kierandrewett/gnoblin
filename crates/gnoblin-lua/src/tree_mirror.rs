//! The shell's mirror of the scene tree, the transaction builder that writes to it,
//! and the link trait that carries both over the owner socket (design/node-core.md,
//! sections 4.1 and 4.6).
//!
//! The compositor owns the tree. The shell holds a mirror rebuilt from a snapshot and
//! advanced by change batches, each stamped with a serial. Writes made inside one tick
//! are gathered by a [`LuaTxBuilder`] that reads through its own overlay, so a write is
//! visible to later reads in the same tick without touching the mirror. Only a
//! transaction the compositor applies changes the mirror, through the change batch it
//! publishes; a refused transaction therefore leaves the mirror exactly as it was.
//!
//! This module is pure Rust and knows nothing of Lua. The Lua `core` table is built
//! on it in `tree_core`.

use std::collections::BTreeMap;

use std::fmt::Write as _;

use gnoblin_proto::tree::effect;

/// A node id. Zero is nil.
pub type NodeId = u64;

/// The nil node id: the parent of the root and of every node not yet inserted.
pub const NIL: NodeId = 0;

/// The shell's name for a class the wire carries by its schema name: `seat` is `Seat`. The
/// schema names are lowercase; the shell's classes are their names with the first letter
/// capitalised (`tree_core::tree_classes`).
pub fn lua_class(wire: &str) -> String {
    let lower = wire.to_ascii_lowercase();
    let mut chars = lower.chars();
    chars
        .next()
        .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
        .unwrap_or_default()
}

/// The schema name the wire carries for a shell class: `Seat` is `seat`. The compositor
/// checks classes by exact schema name, so a class the shell creates is sent this way.
pub fn wire_class(lua: &str) -> String {
    lua.to_ascii_lowercase()
}

/// A property value, one variant per `value_kind` the wire carries.
#[derive(Clone, Debug, PartialEq)]
pub enum LuaValue {
    Bool(bool),
    Int(i64),
    Uint(u64),
    Double(f64),
    Text(String),
    Node(NodeId),
    List(Vec<f64>),
    Texts(Vec<String>),
    Effects(Vec<effect::Properties>),
}

impl LuaValue {
    fn dump(&self) -> String {
        match self {
            Self::Bool(v) => format!("bool {v}"),
            Self::Int(v) => format!("int {v}"),
            Self::Uint(v) => format!("uint {v}"),
            Self::Double(v) => format!("double {v:?}"),
            Self::Text(v) => format!("text {v:?}"),
            Self::Node(v) => format!("node {v}"),
            Self::List(v) => format!("list {v:?}"),
            Self::Texts(v) => format!("texts {v:?}"),
            Self::Effects(v) => format!("effects {v:?}"),
        }
    }
}

/// One node of a snapshot chunk.
#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotNode {
    pub id: NodeId,
    pub parent: NodeId,
    pub index: i64,
    pub class: String,
    /// The node's title; empty when it has none. Stored as the `title` property when set.
    pub title: String,
    /// The node's app id; empty when it has none. Stored as the `app_id` property when set.
    pub app_id: String,
    /// The node's property values that differ from their schema defaults, by schema name.
    /// A property at its default is absent, so reads of it return the default.
    pub props: Vec<(String, LuaValue)>,
}

/// What a change entry does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirrorChangeKind {
    Created,
    Parented,
    Set,
    Destroyed,
}

/// One entry of a change batch. Fields the kind does not use are zero or empty.
#[derive(Clone, Debug, PartialEq)]
pub struct LuaChange {
    pub kind: MirrorChangeKind,
    pub node: NodeId,
    pub class: String,
    pub parent: NodeId,
    pub index: i64,
    pub key: String,
    pub value: Option<LuaValue>,
    pub removed: Vec<NodeId>,
}

/// One write, as sent in a transaction.
#[derive(Clone, Debug, PartialEq)]
pub enum LuaOp {
    New {
        id: NodeId,
        class: String,
    },
    Set {
        node: NodeId,
        key: String,
        value: LuaValue,
        seconds: Option<f64>,
        samples: Option<Vec<f64>>,
    },
    Insert {
        node: NodeId,
        parent: NodeId,
        index: i64,
    },
    Destroy {
        node: NodeId,
    },
    Call {
        node: NodeId,
        method: String,
        args: Vec<u8>,
        /// The op's text: a seat claim's chord. Empty for every other call.
        text: String,
    },
}

/// A transaction: the ops of one tick, stamped with the batch the shell last applied.
#[derive(Clone, Debug, PartialEq)]
pub struct LuaTransaction {
    pub base_serial: u64,
    pub serial: u64,
    pub ops: Vec<LuaOp>,
}

/// The compositor's answer to a transaction.
#[derive(Clone, Debug, PartialEq)]
pub struct TxReply {
    pub serial: u64,
    pub applied: bool,
    /// The index of the first op that failed, or -1 when all applied.
    pub failed_op: i64,
    /// The refusal code, empty when applied.
    pub code: String,
    /// The subtree ids a destroy removed.
    pub removed: Vec<NodeId>,
}

/// An input event addressed to a node.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeEvent {
    pub frame: u64,
    pub seq: u64,
    pub node: NodeId,
    pub kind: String,
}

/// A claimed chord the compositor saw change state: `claim` is the id the Lua side allocated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LuaChord {
    pub claim: i64,
    pub chord: String,
    pub state: String,
}

/// The reply to attach: the epoch this shell holds, the root and the snapshot serial.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attached {
    pub epoch: u64,
    pub root: NodeId,
    pub serial: u64,
}

/// Everything the compositor can send on the tree namespace.
#[derive(Clone, Debug, PartialEq)]
pub enum LuaIncoming {
    Attached(Attached),
    Snapshot {
        serial: u64,
        chunk: u32,
        last: bool,
        nodes: Vec<SnapshotNode>,
    },
    Changes {
        serial: u64,
        changes: Vec<LuaChange>,
    },
    Reply(TxReply),
    Answer {
        token: u64,
        result: Vec<u8>,
        error: String,
    },
    Event(TreeEvent),
    Chord(LuaChord),
}

/// A failure of the link itself: the socket closed or a frame did not decode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkError(pub String);

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LinkError {}

/// The seam between the mirror and the owner socket. The socket implementation lives in
/// `tree_link`; tests use an in-memory compositor.
pub trait TreeLink {
    /// Ask for a snapshot. The attach reply and the snapshot chunks arrive through [`TreeLink::poll`].
    fn attach(&mut self, version: u64) -> Result<(), LinkError>;
    /// Send one transaction. Its reply arrives through [`TreeLink::poll`].
    fn transact(&mut self, tx: &LuaTransaction) -> Result<(), LinkError>;
    /// Drain everything that has arrived since the last call, in arrival order.
    fn poll(&mut self) -> Result<Vec<LuaIncoming>, LinkError>;
}

/// The mirror's view of one node.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeRecord {
    pub class: String,
    pub parent: NodeId,
    pub children: Vec<NodeId>,
    pub props: BTreeMap<String, LuaValue>,
}

/// Why the mirror refused a snapshot or a change batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MirrorError {
    /// The batch is not the next one after the mirror's serial: a batch was missed.
    /// The shell must reattach and rebuild the mirror from a fresh snapshot (F23).
    Gap { expected: u64, got: u64 },
    /// A change named a node the mirror does not hold, or was otherwise inconsistent.
    /// The mirror is not usable after this error: reattach.
    Inconsistent(String),
    /// A snapshot did not describe a tree, or its chunks did not arrive in order.
    Snapshot(String),
}

impl std::fmt::Display for MirrorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Gap { expected, got } => write!(f, "change batch {got} arrived, expected {expected}"),
            Self::Inconsistent(why) => write!(f, "inconsistent change batch: {why}"),
            Self::Snapshot(why) => write!(f, "bad snapshot: {why}"),
        }
    }
}

impl std::error::Error for MirrorError {}

/// What [`LuaMirror::apply_changes`] did with a batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LuaApplied {
    LuaApplied,
    /// The batch was at or before the mirror's serial: it is stale and was ignored (F24).
    Ignored,
}

/// The shell's copy of the tree.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LuaMirror {
    epoch: u64,
    root: NodeId,
    serial: u64,
    nodes: BTreeMap<NodeId, NodeRecord>,
}

impl LuaMirror {
    /// An empty mirror, before the first snapshot.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Build a mirror from a whole snapshot: the nodes of every chunk, in any order.
    pub fn from_snapshot(epoch: u64, root: NodeId, serial: u64, nodes: &[SnapshotNode]) -> Result<Self, MirrorError> {
        let mut map: BTreeMap<NodeId, NodeRecord> = BTreeMap::new();
        for node in nodes {
            if node.id == NIL {
                return Err(MirrorError::Snapshot("node id zero".to_owned()));
            }
            let mut record = NodeRecord {
                class: lua_class(&node.class),
                parent: node.parent,
                ..NodeRecord::default()
            };
            for (key, text) in [("title", &node.title), ("app_id", &node.app_id)] {
                if !text.is_empty() {
                    record.props.insert(key.to_owned(), LuaValue::Text(text.clone()));
                }
            }
            for (key, value) in &node.props {
                record.props.insert(key.clone(), value.clone());
            }
            if map.insert(node.id, record).is_some() {
                return Err(MirrorError::Snapshot(format!("node {} listed twice", node.id)));
            }
        }
        if !map.contains_key(&root) {
            return Err(MirrorError::Snapshot(format!("root {root} is not in the snapshot")));
        }
        let mut order: Vec<(NodeId, i64, NodeId)> = nodes.iter().map(|n| (n.parent, n.index, n.id)).collect();
        order.sort_unstable();
        for (parent, _, id) in order {
            if parent == NIL {
                continue;
            }
            let Some(record) = map.get_mut(&parent) else {
                return Err(MirrorError::Snapshot(format!("node {id} has missing parent {parent}")));
            };
            record.children.push(id);
        }
        Ok(Self {
            epoch,
            root,
            serial,
            nodes: map,
        })
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn root(&self) -> NodeId {
        self.root
    }

    /// The serial of the last change batch applied, or of the snapshot.
    pub fn serial(&self) -> u64 {
        self.serial
    }

    pub fn node(&self, id: NodeId) -> Option<&NodeRecord> {
        self.nodes.get(&id)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// A property the mirror holds. `None` means the mirror has not seen it set.
    pub fn value(&self, id: NodeId, key: &str) -> Option<&LuaValue> {
        self.node(id)?.props.get(key)
    }

    /// Apply one change batch published at `serial`.
    ///
    /// A batch at or before the mirror's serial is stale and ignored (F24). A batch past
    /// the next serial means one was missed: [`MirrorError::Gap`] (F23). Any other error
    /// leaves the mirror half-applied, so the caller must reattach and rebuild it.
    pub fn apply_changes(&mut self, serial: u64, changes: &[LuaChange]) -> Result<LuaApplied, MirrorError> {
        if serial <= self.serial {
            return Ok(LuaApplied::Ignored);
        }
        let expected = self.serial + 1;
        if serial != expected {
            return Err(MirrorError::Gap { expected, got: serial });
        }
        for change in changes {
            self.apply_change(change)?;
        }
        self.serial = serial;
        Ok(LuaApplied::LuaApplied)
    }

    fn apply_change(&mut self, change: &LuaChange) -> Result<(), MirrorError> {
        match change.kind {
            MirrorChangeKind::Created => {
                if self.nodes.contains_key(&change.node) {
                    return Err(inconsistent(format!("node {} created twice", change.node)));
                }
                self.nodes.insert(
                    change.node,
                    NodeRecord {
                        class: lua_class(&change.class),
                        ..NodeRecord::default()
                    },
                );
                Ok(())
            }
            MirrorChangeKind::Parented => {
                self.require(change.node)?;
                if change.parent != NIL {
                    self.require(change.parent)?;
                    if self.is_within(change.parent, change.node) {
                        return Err(inconsistent(format!("node {} parented under itself", change.node)));
                    }
                }
                self.detach(change.node);
                self.nodes.entry(change.node).and_modify(|r| r.parent = change.parent);
                if change.parent != NIL {
                    self.attach_at(change.parent, change.node, change.index);
                }
                Ok(())
            }
            MirrorChangeKind::Set => {
                let Some(value) = change.value.clone() else {
                    return Err(inconsistent(format!("set of {} has no value", change.key)));
                };
                self.require(change.node)?;
                if let Some(record) = self.nodes.get_mut(&change.node) {
                    record.props.insert(change.key.clone(), value);
                }
                Ok(())
            }
            MirrorChangeKind::Destroyed => {
                self.require(change.node)?;
                self.detach(change.node);
                for id in std::iter::once(&change.node).chain(&change.removed) {
                    if self.nodes.remove(id).is_none() {
                        return Err(inconsistent(format!("destroyed node {id} is not in the mirror")));
                    }
                }
                Ok(())
            }
        }
    }

    fn require(&self, id: NodeId) -> Result<(), MirrorError> {
        if self.nodes.contains_key(&id) {
            Ok(())
        } else {
            Err(inconsistent(format!("node {id} is not in the mirror")))
        }
    }

    /// Whether `id` is `ancestor` or lies beneath it.
    fn is_within(&self, mut id: NodeId, ancestor: NodeId) -> bool {
        while id != NIL {
            if id == ancestor {
                return true;
            }
            id = self.nodes.get(&id).map_or(NIL, |r| r.parent);
        }
        false
    }

    /// Remove `id` from its parent's children and clear its parent link.
    fn detach(&mut self, id: NodeId) {
        let Some(parent) = self.nodes.get(&id).map(|r| r.parent) else {
            return;
        };
        if let Some(record) = self.nodes.get_mut(&parent) {
            record.children.retain(|child| *child != id);
        }
        if let Some(record) = self.nodes.get_mut(&id) {
            record.parent = NIL;
        }
    }

    fn attach_at(&mut self, parent: NodeId, id: NodeId, index: i64) {
        if let Some(record) = self.nodes.get_mut(&parent) {
            let at = usize::try_from(index).map_or(record.children.len(), |i| i.min(record.children.len()));
            record.children.insert(at, id);
        }
    }

    /// A deterministic text rendering of the whole mirror, for replay comparisons.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "epoch {} root {} serial {}", self.epoch, self.root, self.serial);
        for (id, record) in &self.nodes {
            let _ = writeln!(
                out,
                "node {id} class={} parent={} children={:?}",
                record.class, record.parent, record.children
            );
            for (key, value) in &record.props {
                let _ = writeln!(out, "  {key} = {}", value.dump());
            }
        }
        out
    }
}

fn inconsistent(why: String) -> MirrorError {
    MirrorError::Inconsistent(why)
}

/// Gathers one snapshot from its chunks. A reattach starts a new builder.
#[derive(Debug)]
pub struct SnapshotBuilder {
    epoch: u64,
    root: NodeId,
    serial: u64,
    next_chunk: u32,
    nodes: Vec<SnapshotNode>,
}

impl SnapshotBuilder {
    /// Start a snapshot from the attach reply.
    pub fn new(attached: Attached) -> Self {
        Self {
            epoch: attached.epoch,
            root: attached.root,
            serial: attached.serial,
            next_chunk: 0,
            nodes: Vec::new(),
        }
    }

    /// Add one chunk. Returns the finished mirror when `last` is set.
    pub fn chunk(
        &mut self,
        serial: u64,
        chunk: u32,
        last: bool,
        nodes: Vec<SnapshotNode>,
    ) -> Result<Option<LuaMirror>, MirrorError> {
        if serial != self.serial {
            return Err(MirrorError::Snapshot(format!(
                "chunk of serial {serial} in a snapshot of serial {}",
                self.serial
            )));
        }
        if chunk != self.next_chunk {
            return Err(MirrorError::Snapshot(format!(
                "chunk {chunk} arrived, expected {}",
                self.next_chunk
            )));
        }
        self.next_chunk += 1;
        self.nodes.extend(nodes);
        if last {
            let nodes = std::mem::take(&mut self.nodes);
            LuaMirror::from_snapshot(self.epoch, self.root, self.serial, &nodes).map(Some)
        } else {
            Ok(None)
        }
    }
}

/// Why a write was refused by the shell's own checks, before it was sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TxError {
    /// The node is not in the tree (or was destroyed earlier in this tick).
    UnknownNode(NodeId),
    /// The id is already in use.
    Exists(NodeId),
    /// The insert would make a node its own ancestor.
    Cycle(NodeId),
    /// The root cannot be destroyed or reparented.
    Root(NodeId),
}

impl std::fmt::Display for TxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownNode(id) => write!(f, "no node {id}"),
            Self::Exists(id) => write!(f, "node {id} already exists"),
            Self::Cycle(id) => write!(f, "inserting {id} would make it its own ancestor"),
            Self::Root(id) => write!(f, "node {id} is the root"),
        }
    }
}

impl std::error::Error for TxError {}

/// Collects the writes of one tick. The builder owns only its pending ops and an overlay of
/// the nodes it wrote, so it can sit beside the mirror it reads in one struct without a
/// self-reference. Every method that reads or writes takes the mirror the tick started from;
/// reads see this tick's writes first, and that mirror is never changed.
#[derive(Debug, Default)]
pub struct LuaTxBuilder {
    overlay: BTreeMap<NodeId, Option<NodeRecord>>,
    ops: Vec<LuaOp>,
}

impl LuaTxBuilder {
    /// An empty builder for one tick.
    pub fn new() -> Self {
        Self::default()
    }

    /// The node as this tick sees it, against the mirror the tick started from.
    pub fn node<'a>(&'a self, base: &'a LuaMirror, id: NodeId) -> Option<&'a NodeRecord> {
        match self.overlay.get(&id) {
            Some(entry) => entry.as_ref(),
            None => base.node(id),
        }
    }

    /// A property as this tick sees it.
    pub fn get<'a>(&'a self, base: &'a LuaMirror, id: NodeId, key: &str) -> Option<&'a LuaValue> {
        self.node(base, id)?.props.get(key)
    }

    /// The children of a node as this tick sees them, in order.
    pub fn children<'a>(&'a self, base: &'a LuaMirror, id: NodeId) -> Option<&'a [NodeId]> {
        self.node(base, id).map(|record| record.children.as_slice())
    }

    /// Copy a node into the overlay on first write, so the base is never touched.
    fn touch(&mut self, base: &LuaMirror, id: NodeId) -> Option<&mut NodeRecord> {
        if let std::collections::btree_map::Entry::Vacant(slot) = self.overlay.entry(id) {
            let record = base.node(id)?.clone();
            slot.insert(Some(record));
        }
        self.overlay.get_mut(&id)?.as_mut()
    }

    fn require(&self, base: &LuaMirror, id: NodeId) -> Result<(), TxError> {
        self.node(base, id).map(|_| ()).ok_or(TxError::UnknownNode(id))
    }

    /// Create a node with a pre-allocated id.
    pub fn new_node(&mut self, base: &LuaMirror, id: NodeId, class: &str) -> Result<(), TxError> {
        if self.node(base, id).is_some() || self.overlay.contains_key(&id) {
            return Err(TxError::Exists(id));
        }
        self.overlay.insert(
            id,
            Some(NodeRecord {
                class: class.to_owned(),
                ..NodeRecord::default()
            }),
        );
        self.ops.push(LuaOp::New {
            id,
            class: wire_class(class),
        });
        Ok(())
    }

    /// Set a property. `seconds` and `samples` describe an animated set; `None` is instant.
    pub fn set(
        &mut self,
        base: &LuaMirror,
        node: NodeId,
        key: &str,
        value: LuaValue,
        seconds: Option<f64>,
        samples: Option<Vec<f64>>,
    ) -> Result<(), TxError> {
        let record = self.touch(base, node).ok_or(TxError::UnknownNode(node))?;
        record.props.insert(key.to_owned(), value.clone());
        self.ops.push(LuaOp::Set {
            node,
            key: key.to_owned(),
            value,
            seconds,
            samples,
        });
        Ok(())
    }

    /// Insert `node` under `parent` at `index`, moving it from its current parent.
    pub fn insert(&mut self, base: &LuaMirror, node: NodeId, parent: NodeId, index: i64) -> Result<(), TxError> {
        self.require(base, node)?;
        self.require(base, parent)?;
        if node == base.root() {
            return Err(TxError::Root(node));
        }
        if self.is_ancestor_or_self(base, node, parent) {
            return Err(TxError::Cycle(node));
        }
        if let Some(old) = self.node(base, node).map(|r| r.parent)
            && old != NIL
            && let Some(record) = self.touch(base, old)
        {
            record.children.retain(|child| *child != node);
        }
        if let Some(record) = self.touch(base, node) {
            record.parent = parent;
        }
        if let Some(record) = self.touch(base, parent) {
            let at = usize::try_from(index).map_or(record.children.len(), |i| i.min(record.children.len()));
            record.children.insert(at, node);
        }
        self.ops.push(LuaOp::Insert { node, parent, index });
        Ok(())
    }

    fn is_ancestor_or_self(&self, base: &LuaMirror, ancestor: NodeId, mut id: NodeId) -> bool {
        while id != NIL {
            if id == ancestor {
                return true;
            }
            id = self.node(base, id).map_or(NIL, |r| r.parent);
        }
        false
    }

    /// Destroy a node and its subtree. Returns the ids removed beneath it.
    pub fn destroy(&mut self, base: &LuaMirror, node: NodeId) -> Result<Vec<NodeId>, TxError> {
        self.require(base, node)?;
        if node == base.root() {
            return Err(TxError::Root(node));
        }
        let mut subtree = Vec::new();
        let mut pending = vec![node];
        while let Some(id) = pending.pop() {
            if let Some(record) = self.node(base, id) {
                pending.extend(record.children.iter().copied());
            }
            subtree.push(id);
        }
        let parent = self.node(base, node).map_or(NIL, |r| r.parent);
        if parent != NIL
            && let Some(record) = self.touch(base, parent)
        {
            record.children.retain(|child| *child != node);
        }
        for id in &subtree {
            self.overlay.insert(*id, None);
        }
        self.ops.push(LuaOp::Destroy { node });
        Ok(subtree.into_iter().filter(|id| *id != node).collect())
    }

    /// Call a method on a node. The answer arrives later, by token.
    pub fn call(
        &mut self,
        base: &LuaMirror,
        node: NodeId,
        method: &str,
        args: Vec<u8>,
        text: String,
    ) -> Result<(), TxError> {
        self.require(base, node)?;
        self.ops.push(LuaOp::Call {
            node,
            method: method.to_owned(),
            args,
            text,
        });
        Ok(())
    }

    pub fn ops(&self) -> &[LuaOp] {
        &self.ops
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// The transaction for these writes, stamped with the serial of the mirror `base`, which
    /// must be the mirror this tick started from.
    pub fn into_transaction(self, base: &LuaMirror, serial: u64) -> LuaTransaction {
        LuaTransaction {
            base_serial: base.serial(),
            serial,
            ops: self.ops,
        }
    }
}

#[cfg(test)]
mod class_name_tests {
    use super::{lua_class, wire_class};

    #[test]
    fn a_wire_class_is_the_shells_capitalised_name() {
        assert_eq!(lua_class("seat"), "Seat");
        assert_eq!(lua_class("node"), "Node");
        assert_eq!(lua_class("View"), "View");
    }

    #[test]
    fn a_shell_class_is_sent_by_its_schema_name() {
        assert_eq!(wire_class("Screen"), "screen");
        assert_eq!(wire_class("Node"), "node");
        assert_eq!(lua_class(&wire_class("Workspace")), "Workspace");
    }
}
