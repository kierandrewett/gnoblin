//! The host of the `tree` namespace: the one tree session the server serves, and the mapping
//! between its wire payloads and the transaction types.
//!
//! Every request runs on the dispatcher thread, so the session's lock is never contended. It
//! is a mutex so the session can be read from another thread once observers need it.

use std::sync::{Mutex, MutexGuard, PoisonError};

use gnoblin_proto::tree::tree::{AttachedPayload, ChangesPayload, ReplyPayload, SnapshotPayload, TransactArgs};
use gnoblin_proto::tree::{change, op, snapshot_node};
use gnoblin_render::transaction::{
    Code, Timing, TreeApplied, TreeChange, TreeOp, TreeRefusal, TreeTransaction, TreeValue,
};
use gnoblin_render::{Actor, NodeSpec, Scene};
use gnoblin_types::{ClientId, NodeId, Rect};

use super::tree::{TreeAttached, TreeClasses, TreeHolder, TreeReply, TreeSession, TreeSnapshot, tree_listing};

/// The namespace the tree answers on.
pub const NAMESPACE: &str = "tree";

/// The members the tree answers on: `attach` and `transact` are requests, and
/// `snapshot` and `changes` are the events that follow them.
pub mod member {
    pub const ATTACH: &str = "attach";
    pub const TRANSACT: &str = "transact";
    pub const SNAPSHOT: &str = "snapshot";
    pub const CHANGES: &str = "changes";
}

/// The root node of the tree the compositor starts with. The shell places its windows under it.
const ROOT: NodeId = NodeId::new(1);

/// The shell's tree, served by the dispatcher.
#[derive(Debug)]
pub struct TreeHostHandle {
    session: Mutex<TreeSession<TreeClasses>>,
}

impl TreeHostHandle {
    /// A host over a fresh scene whose only node is the root.
    ///
    /// # Errors
    ///
    /// Returns the reason the scene, the schema catalog or the session could not be built.
    pub fn new() -> Result<Self, String> {
        let mut scene = Scene::new();
        scene
            .insert_as(Actor::Compositor, ROOT, NodeSpec::new(Rect::new(0, 0, 100, 100)))
            .map_err(|error| error.to_string())?;
        scene.set_root(Some(ROOT)).map_err(|error| error.to_string())?;
        let catalog = tree_listing(true).map_err(|error| error.to_string())?;
        let classes = TreeClasses::new(catalog)?;
        let session =
            TreeSession::new(scene, classes, ROOT).ok_or_else(|| "the tree root is not in the scene".to_owned())?;
        Ok(Self {
            session: Mutex::new(session),
        })
    }

    fn session(&self) -> MutexGuard<'_, TreeSession<TreeClasses>> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Attach `client` as the shell. `is_owner` is its owner-grant classification.
    pub fn attach(&self, client: ClientId, is_owner: bool) -> Result<TreeAttached, TreeRefusal> {
        self.session().attach(holder(client), is_owner)
    }

    /// Release the hold `client` has, when it is the holder.
    pub fn detach(&self, client: ClientId) {
        self.session().detach(holder(client));
    }

    /// Snapshot the tree at the current serial.
    pub fn snapshot(&self) -> TreeSnapshot {
        self.session().snapshot()
    }

    /// Apply `args` as `client`. Only the holder may transact: any other connection is refused
    /// `not-privileged`, whether or not a shell is attached.
    pub fn transact(&self, client: ClientId, args: &TransactArgs) -> TreeReply {
        let mut session = self.session();
        if session.holder() != Some(holder(client)) {
            return TreeReply {
                serial: session.serial(),
                result: Err(TreeRefusal {
                    failed_op: -1,
                    code: Code::NotPrivileged,
                }),
            };
        }
        match transaction(&args.ops) {
            Ok(transaction) => session.transact(args.base_serial, &transaction),
            Err(refusal) => TreeReply {
                serial: session.serial(),
                result: Err(refusal),
            },
        }
    }
}

fn holder(client: ClientId) -> TreeHolder {
    TreeHolder::new(client.get())
}

/// The attach answer as its wire payload.
#[must_use]
pub fn attached_payload(attached: TreeAttached) -> AttachedPayload {
    AttachedPayload {
        epoch: u64::from(attached.epoch),
        root: attached.root.get(),
        serial: attached.serial,
    }
}

/// The transact answer as its wire payload: the serial after it, and either the applied
/// flag with the nodes it removed, or the refused op and its code.
#[must_use]
pub fn reply_payload(reply: &TreeReply) -> ReplyPayload {
    match &reply.result {
        Ok(applied) => ReplyPayload {
            serial: reply.serial,
            applied: true,
            failed_op: -1,
            code: String::new(),
            removed: applied.changes.iter().flat_map(removed_nodes).collect(),
        },
        Err(refusal) => ReplyPayload {
            serial: reply.serial,
            applied: false,
            failed_op: i64::from(refusal.failed_op),
            code: refusal.code.as_str().to_owned(),
            removed: Vec::new(),
        },
    }
}

fn removed_nodes(change: &TreeChange) -> Vec<u64> {
    match change {
        TreeChange::Destroyed { removed, .. } => removed.iter().map(|node| node.get()).collect(),
        TreeChange::Created { .. } | TreeChange::Parented { .. } | TreeChange::Set { .. } => Vec::new(),
    }
}

/// The changes a committed transaction published, as the `changes` event.
#[must_use]
pub fn changes_payload(serial: u64, applied: &TreeApplied) -> ChangesPayload {
    ChangesPayload {
        serial,
        changes: applied.changes.iter().map(change_properties).collect(),
    }
}

fn change_properties(change: &TreeChange) -> change::Properties {
    match change {
        TreeChange::Created { node, class, index } => change::Properties {
            kind: "created".to_owned(),
            node: node.get(),
            class: class.clone(),
            index: wire_index(*index),
            ..Default::default()
        },
        TreeChange::Parented { node, parent, index } => change::Properties {
            kind: "parented".to_owned(),
            node: node.get(),
            parent: parent.get(),
            index: wire_index(*index),
            ..Default::default()
        },
        TreeChange::Set { node, key, value } => {
            let (value_kind, value_bool, value_uint, value_double) = value_fields(value);
            change::Properties {
                kind: "set".to_owned(),
                node: node.get(),
                key: key.clone(),
                value_kind: value_kind.to_owned(),
                value_bool,
                value_uint,
                value_double,
                ..Default::default()
            }
        }
        TreeChange::Destroyed { node, removed } => change::Properties {
            kind: "destroyed".to_owned(),
            node: node.get(),
            removed: removed.iter().map(|removed| removed.get()).collect(),
            ..Default::default()
        },
    }
}

/// The snapshot as the `snapshot` events, one per chunk. The last carries `last`; a tree with
/// no nodes still sends one empty last chunk, so the client's read always ends.
#[must_use]
pub fn snapshot_payloads(snapshot: &TreeSnapshot) -> Vec<SnapshotPayload> {
    if snapshot.chunks.is_empty() {
        return vec![SnapshotPayload {
            serial: snapshot.serial,
            chunk: 0,
            last: true,
            nodes: Vec::new(),
        }];
    }
    let count = snapshot.chunks.len();
    snapshot
        .chunks
        .iter()
        .enumerate()
        .map(|(index, nodes)| SnapshotPayload {
            serial: snapshot.serial,
            chunk: u64::try_from(index).unwrap_or(u64::MAX),
            last: index + 1 == count,
            nodes: nodes
                .iter()
                .map(|node| snapshot_node::Properties {
                    id: node.id.get(),
                    parent: node.parent.map_or(0, NodeId::get),
                    index: wire_index(node.index),
                    class: node.class.clone().unwrap_or_default(),
                })
                .collect(),
        })
        .collect()
}

/// A transaction from its wire ops. The first op that does not convert refuses the batch,
/// with its position as `failed_op`.
fn transaction(ops: &[op::Properties]) -> Result<TreeTransaction, TreeRefusal> {
    let converted = ops
        .iter()
        .enumerate()
        .map(|(index, op)| {
            tree_op(op).map_err(|code| TreeRefusal {
                failed_op: i32::try_from(index).unwrap_or(i32::MAX),
                code,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(TreeTransaction::new(converted))
}

/// One wire op as a transaction op. A kind this build does not know, or a value it cannot
/// carry, is `bad-value`.
fn tree_op(op: &op::Properties) -> Result<TreeOp, Code> {
    let node = NodeId::new(op.node);
    Ok(match op.kind.as_str() {
        "new" => TreeOp::New {
            class: op.class.clone(),
            id: NodeId::new(op.id),
        },
        "insert" => TreeOp::Insert {
            node,
            parent: NodeId::new(op.parent),
            // A negative index appends.
            index: usize::try_from(op.index).ok(),
        },
        "destroy" => TreeOp::Destroy { node },
        "set" => TreeOp::Set {
            node,
            key: op.key.clone(),
            value: set_value(op)?,
            timing: timing(op),
        },
        "call" => TreeOp::TreeCall {
            node,
            method: op.method.clone(),
            args: call_args(&op.args)?,
        },
        _ => return Err(Code::BadValue),
    })
}

/// The value a `set` writes. Only the kinds the transaction layer can carry are accepted.
fn set_value(op: &op::Properties) -> Result<TreeValue, Code> {
    match op.value_kind.as_str() {
        "bool" => Ok(TreeValue::Bool(op.value_bool)),
        "uint" => Ok(TreeValue::Uint(op.value_uint)),
        "double" => Ok(TreeValue::Double(op.value_double)),
        _ => Err(Code::BadValue),
    }
}

/// The timing of a `set`: an instant set has neither a duration nor samples. A duration of
/// zero with samples is kept, so the layer refuses it as out of range.
fn timing(op: &op::Properties) -> Option<Timing> {
    if op.seconds == 0.0 && op.samples.is_empty() {
        None
    } else {
        Some(Timing {
            seconds: op.seconds,
            samples: op.samples.clone(),
        })
    }
}

/// A call's arguments: little-endian `i64`s packed back to back. Bytes that do not make whole
/// `i64`s are `bad-value`.
fn call_args(bytes: &[u8]) -> Result<Vec<i64>, Code> {
    let chunks = bytes.chunks_exact(8);
    if !chunks.remainder().is_empty() {
        return Err(Code::BadValue);
    }
    chunks
        .map(|chunk| {
            <[u8; 8]>::try_from(chunk)
                .map(i64::from_le_bytes)
                .map_err(|_| Code::BadValue)
        })
        .collect()
}

/// A value as the wire's typed fields: its kind, then the one field that carries it.
fn value_fields(value: &TreeValue) -> (&'static str, bool, u64, f64) {
    match *value {
        TreeValue::Bool(value) => ("bool", value, 0, 0.0),
        TreeValue::Uint(value) => ("uint", false, value, 0.0),
        TreeValue::Double(value) => ("double", false, 0, value),
    }
}

fn wire_index(index: usize) -> i64 {
    i64::try_from(index).unwrap_or(i64::MAX)
}
