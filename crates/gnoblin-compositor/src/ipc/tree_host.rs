//! The host of the `tree` namespace: the wire side of the tree. Requests from the shell are
//! queued on the frame that owns the scene; the frame answers them, and its published batches
//! come back to the dispatcher as events.
//!
//! The dispatcher never touches the scene. It reads the session's holder under its lock, and
//! the frame reads and writes the same session between frames. A transaction pipelined straight
//! after `attach`, before the attach reply, is refused `not-privileged`: the shell waits for
//! the attach reply before it transacts.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gnoblin_proto::tree::tree::{
    AttachedPayload, ChangesPayload, EventPayload, ReplyPayload, SnapshotPayload, TransactArgs,
};
use gnoblin_proto::tree::{change, op, snapshot_node};
use gnoblin_render::transaction::{
    ClassInfo, Code, Timing, TreeChange, TreeOp, TreeRefusal, TreeTransaction, TreeValue,
};
use gnoblin_types::{ClientId, NodeId};
use gnoblin_wayland::{FrameTreeEvent, TreeBatch, TreeFrameInput, TreeHooks, TreeInput, TreeQueue};

use super::server::IpcEvent;
use super::tree::{
    TreeAttached, TreeClasses, TreeHolder, TreeReply, TreeSession, TreeSnapshot, snapshot_of, tree_listing,
};

/// The namespace the tree answers on.
pub const NAMESPACE: &str = "tree";

/// The members the tree answers on: `attach` and `transact` are requests, and
/// `snapshot`, `changes` and `event` are the events that follow them.
pub mod member {
    pub const ATTACH: &str = "attach";
    pub const TRANSACT: &str = "transact";
    pub const SNAPSHOT: &str = "snapshot";
    pub const CHANGES: &str = "changes";
    pub const EVENT: &str = "event";
}

/// The shell's tree as the dispatcher sees it. Clones share one queue, one session and one
/// serial.
#[derive(Clone)]
pub struct TreeHostHandle {
    queue: TreeQueue,
    classes: Arc<TreeClasses>,
    events: Sender<IpcEvent>,
    session: Arc<Mutex<TreeSession>>,
    /// The serial of the last batch the frame published, read when a refusal answers.
    serial: Arc<AtomicU64>,
}

impl std::fmt::Debug for TreeHostHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeHostHandle").finish_non_exhaustive()
    }
}

impl TreeHostHandle {
    /// A host whose events go to `events`.
    ///
    /// # Errors
    ///
    /// Returns the reason the schema catalog or the class table could not be built.
    pub fn new(events: Sender<IpcEvent>) -> Result<Self, String> {
        let catalog = tree_listing(true).map_err(|error| error.to_string())?;
        let classes = Arc::new(TreeClasses::new(catalog)?);
        Ok(Self {
            queue: TreeQueue::default(),
            classes,
            events,
            session: Arc::new(Mutex::new(TreeSession::default())),
            serial: Arc::new(AtomicU64::new(0)),
        })
    }

    /// The hooks the Wayland session's frame runs the tree with. The frame publishes each batch
    /// it applies; a batch goes to the shell's connection when one holds the tree.
    #[must_use]
    pub fn hooks(&self) -> TreeHooks {
        let session = Arc::clone(&self.session);
        let serial = Arc::clone(&self.serial);
        let events = self.events.clone();
        TreeHooks {
            queue: self.queue.clone(),
            classes: Arc::clone(&self.classes) as Arc<dyn ClassInfo + Send + Sync>,
            publish: Arc::new(move |batch: TreeBatch| {
                serial.store(batch.serial, Ordering::Release);
                let holder = lock(&session).holder();
                if let Some(holder) = holder {
                    let _ = events.send(IpcEvent::TreeChanged {
                        client: ClientId::new(holder.get()),
                        batch,
                    });
                }
            }),
        }
    }

    /// Queues the attach of `client`. The frame decides it at the current serial and sends
    /// `IpcEvent::TreeAttached` with the outcome and, on success, the snapshot to send.
    pub fn attach(&self, client: ClientId, request_id: u64, is_owner: bool) {
        let session = Arc::clone(&self.session);
        let events = self.events.clone();
        self.queue.read(move |scene, serial| {
            let outcome = lock(&session).attach(holder(client), is_owner).map(|attachment| {
                scene.set_shell_places(true);
                (
                    TreeAttached {
                        epoch: attachment.epoch,
                        root: scene.root(),
                        serial,
                    },
                    snapshot_of(scene, serial),
                )
            });
            let _ = events.send(IpcEvent::TreeAttached {
                client,
                request_id,
                outcome,
            });
        });
    }

    /// Release the hold `client` has, when it is the holder.
    pub fn detach(&self, client: ClientId) {
        let shared = Arc::clone(&self.session);
        let released = {
            let mut session = lock(&shared);
            let before = session.holder();
            session.detach(holder(client));
            before != session.holder()
        };
        if released {
            // The frame clears the flag at its next read, so the wm places windows again.
            self.queue
                .read(move |scene, _| scene.set_shell_places(lock(&shared).holder().is_some()));
        }
    }

    /// Answers `args` as `client`, or queues it for the frame. `Some` is the answer now: a
    /// connection that does not hold the tree is refused `not-privileged`, and a transaction
    /// that does not convert is refused with its op. `None` means the frame answers later,
    /// with `IpcEvent::TreeReply`.
    pub fn transact(&self, client: ClientId, request_id: u64, args: &TransactArgs) -> Option<TreeReply> {
        let me = holder(client);
        if lock(&self.session).holder() != Some(me) {
            return Some(self.refusal(Code::NotPrivileged, -1));
        }
        let tx = match transaction(&args.ops) {
            Ok(tx) => tx,
            Err(refusal) => return Some(self.refusal(refusal.code, refusal.failed_op)),
        };
        let events = self.events.clone();
        // Asked again when the frame applies: a holder that detached while the transaction
        // waited for the frame has lost the tree, and the frame refuses it.
        let session = Arc::clone(&self.session);
        let admitted = move || lock(&session).holder() == Some(me);
        self.queue
            .submit(args.base_serial, tx, admitted, move |serial, result| {
                let _ = events.send(IpcEvent::TreeReply {
                    client,
                    request_id,
                    reply: TreeReply { serial, result },
                });
            });
        None
    }

    fn refusal(&self, code: Code, failed_op: i32) -> TreeReply {
        TreeReply {
            serial: self.serial.load(Ordering::Acquire),
            result: Err(TreeRefusal { failed_op, code }),
        }
    }
}

fn lock(session: &Mutex<TreeSession>) -> MutexGuard<'_, TreeSession> {
    session.lock().unwrap_or_else(PoisonError::into_inner)
}

fn holder(client: ClientId) -> TreeHolder {
    TreeHolder::new(client.get())
}

/// The attach answer as its wire payload.
#[must_use]
pub fn attached_payload(attached: TreeAttached) -> AttachedPayload {
    AttachedPayload {
        epoch: u64::from(attached.epoch),
        root: attached.root.map_or(0, NodeId::get),
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
pub fn changes_payload(serial: u64, changes: &[TreeChange]) -> ChangesPayload {
    ChangesPayload {
        serial,
        changes: changes.iter().map(change_properties).collect(),
    }
}

/// One tree event as its wire payload: the node it is for, its `kind`, and the fields that kind
/// carries. A `request` names the request; a pointer kind carries node-local `x` and `y`, plus
/// `button` for press and release and `dx`, `dy` for scroll.
#[must_use]
pub fn event_payload(event: &FrameTreeEvent) -> EventPayload {
    let base = EventPayload {
        frame: event.frame,
        seq: event.seq,
        node: event.node.get(),
        ..Default::default()
    };
    match event.event {
        TreeFrameInput::Request(request) => EventPayload {
            kind: "request".to_owned(),
            request: request.name().to_owned(),
            ..base
        },
        TreeFrameInput::Input(input) => {
            let kind = input.name().to_owned();
            match input {
                TreeInput::Press { x, y, button } | TreeInput::Release { x, y, button } => EventPayload {
                    kind,
                    x,
                    y,
                    button: button.into(),
                    ..base
                },
                TreeInput::Motion { x, y } | TreeInput::Enter { x, y } => EventPayload { kind, x, y, ..base },
                TreeInput::Scroll { x, y, dx, dy } => EventPayload {
                    kind,
                    x,
                    y,
                    dx,
                    dy,
                    ..base
                },
                TreeInput::Leave => EventPayload { kind, ..base },
            }
        }
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
            let mut properties = change::Properties {
                kind: "set".to_owned(),
                node: node.get(),
                key: key.clone(),
                ..Default::default()
            };
            match value {
                TreeValue::Bool(value) => {
                    "bool".clone_into(&mut properties.value_kind);
                    properties.value_bool = *value;
                }
                TreeValue::Int(value) => {
                    "int".clone_into(&mut properties.value_kind);
                    properties.value_int = *value;
                }
                TreeValue::Uint(value) => {
                    "uint".clone_into(&mut properties.value_kind);
                    properties.value_uint = *value;
                }
                TreeValue::Double(value) => {
                    "double".clone_into(&mut properties.value_kind);
                    properties.value_double = *value;
                }
                TreeValue::Text(value) => {
                    "text".clone_into(&mut properties.value_kind);
                    properties.value_text.clone_from(value);
                }
                TreeValue::Node(value) => {
                    "node".clone_into(&mut properties.value_kind);
                    properties.value_node = value.map_or(0, NodeId::get);
                }
                TreeValue::ListDouble(value) => {
                    "list_double".clone_into(&mut properties.value_kind);
                    properties.value_list.clone_from(value);
                }
                // No wire field carries these two, and `set_value` refuses them as `bad-value`,
                // so a shell can never write one; the kind names it for a direct caller.
                TreeValue::Bytes(_) => "bytes".clone_into(&mut properties.value_kind),
                TreeValue::ListText(_) => "list_text".clone_into(&mut properties.value_kind),
            }
            properties
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
                    title: node.title.clone(),
                    app_id: node.app_id.clone(),
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

/// The value a `set` writes. The kinds the wire carries are bool, int, uint, double, text,
/// node (`0` is nil) and a list of doubles; any other kind is `bad-value`.
fn set_value(op: &op::Properties) -> Result<TreeValue, Code> {
    match op.value_kind.as_str() {
        "bool" => Ok(TreeValue::Bool(op.value_bool)),
        "int" => Ok(TreeValue::Int(op.value_int)),
        "uint" => Ok(TreeValue::Uint(op.value_uint)),
        "double" => Ok(TreeValue::Double(op.value_double)),
        "text" => Ok(TreeValue::Text(op.value_text.clone())),
        "node" => Ok(TreeValue::Node(
            (op.value_node != 0).then(|| NodeId::new(op.value_node)),
        )),
        "list_double" => Ok(TreeValue::ListDouble(op.value_list.clone())),
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
    let (chunks, rest) = bytes.as_chunks::<8>();
    if !rest.is_empty() {
        return Err(Code::BadValue);
    }
    Ok(chunks.iter().map(|chunk| i64::from_le_bytes(*chunk)).collect())
}

fn wire_index(index: usize) -> i64 {
    i64::try_from(index).unwrap_or(i64::MAX)
}
