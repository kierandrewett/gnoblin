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
use std::time::Instant;

use gnoblin_input::GestureDelivery;
use gnoblin_paint::SharedPainter;
use gnoblin_proto::tree::seat::{ChordPayload, GesturePayload};
use gnoblin_proto::tree::tree::{
    AnswerPayload, AttachedPayload, ChangesPayload, EventPayload, ReplyPayload, SnapshotPayload, TransactArgs,
};
use gnoblin_proto::tree::{change, op, snapshot_node};
use gnoblin_render::transaction::{
    ClassInfo, Code, PointerInput, Timing, TreeChange, TreeOp, TreeRefusal, TreeTransaction, TreeValue,
};
use gnoblin_secret::SecretText;
use gnoblin_types::{ClientId, NodeId};
use gnoblin_wayland::{
    ChordEvent, ChordState, FrameTreeEvent, TreeAnswer, TreeBatch, TreeFrameInput, TreeHooks, TreeInput, TreeQueue,
    keysym_name, keysym_text,
};

use super::server::IpcEvent;
use super::tree::{
    AttachMode, TreeAttached, TreeClasses, TreeHolder, TreeReply, TreeSession, TreeSnapshot, observer_ops, snapshot_of,
    tree_listing, writes_head_camera,
};
use super::tree_effects::{pass_from_wire, wire_from_pass};

/// The namespace the tree answers on.
pub const NAMESPACE: &str = "tree";

/// The members the tree answers on: `attach` and `transact` are requests, `answer` is the
/// shell's reply to an `authenticate` change, and `snapshot`, `changes`, `event` and `chord`
/// are the events that follow them.
pub mod member {
    pub const ATTACH: &str = "attach";
    pub const TRANSACT: &str = "transact";
    pub const SNAPSHOT: &str = "snapshot";
    pub const CHANGES: &str = "changes";
    pub const EVENT: &str = "event";
    pub const CHORD: &str = "chord";
    pub const GESTURE: &str = "gesture";
    pub const ANSWER: &str = "answer";
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
    /// Measures text for `Canvas.measure` and paints canvases: the system fonts, found on first use.
    painter: SharedPainter,
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
            painter: SharedPainter::system(),
        })
    }

    /// The hooks the Wayland session's frame runs the tree with. The frame publishes each batch
    /// it applies; a batch goes to the shell's connection when one holds the tree.
    #[must_use]
    pub fn hooks(&self) -> TreeHooks {
        let session = Arc::clone(&self.session);
        let serial = Arc::clone(&self.serial);
        let events = self.events.clone();
        let attached_session = Arc::clone(&self.session);
        TreeHooks {
            queue: self.queue.clone(),
            classes: Arc::clone(&self.classes) as Arc<dyn ClassInfo + Send + Sync>,
            attached: Arc::new(move || lock(&attached_session).has_attachments()),
            painter: self.painter.clone(),
            publish: Arc::new(move |batch: TreeBatch| {
                serial.store(batch.serial, Ordering::Release);
                let (holder, observers) = {
                    let session = lock(&session);
                    (session.holder(), session.observers())
                };
                // An observer hears the changes in serial order and nothing else: the answers, events and
                // chords of a batch are the shell's.
                if !batch.changes.is_empty() {
                    for observer in observers {
                        let _ = events.send(IpcEvent::TreeChanged {
                            client: ClientId::new(observer.get()),
                            batch: TreeBatch {
                                serial: batch.serial,
                                changes: batch.changes.clone(),
                                ..TreeBatch::default()
                            },
                        });
                    }
                }
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
    /// `IpcEvent::TreeAttached` with the outcome and, on success, the snapshot to send. A holder
    /// takes the tree; an observer only reads it and never moves where the shell places windows.
    pub fn attach(&self, client: ClientId, request_id: u64, is_owner: bool, mode: AttachMode) {
        let session = Arc::clone(&self.session);
        let classes = Arc::clone(&self.classes);
        let events = self.events.clone();
        self.queue.read(move |scene, serial| {
            let granted = match mode {
                AttachMode::Holder => lock(&session).attach(holder(client), is_owner),
                AttachMode::Observer => lock(&session).attach_observer(holder(client), is_owner),
            };
            let outcome = granted.map(|attachment| {
                if mode == AttachMode::Holder {
                    scene.set_shell_places(true);
                }
                (
                    TreeAttached {
                        epoch: attachment.epoch,
                        root: scene.root(),
                        serial,
                    },
                    snapshot_of(scene, &classes, serial),
                )
            });
            let _ = events.send(IpcEvent::TreeAttached {
                client,
                request_id,
                outcome,
            });
        });
    }

    /// Release what `client` has: the hold, when it is the holder, or its observation, when it is an observer. An
    /// observer's seats are destroyed with its connection, and so are the nodes a holder made (design/node-core.md,
    /// section 4.6 step 5), except the seats observers made.
    pub fn detach(&self, client: ClientId) {
        let shared = Arc::clone(&self.session);
        let (released, seats) = {
            let mut session = lock(&shared);
            let before = session.holder();
            session.detach(holder(client));
            (before != session.holder(), session.release_observer(holder(client)))
        };
        if released {
            let seats = Arc::clone(&shared);
            self.queue.reap_shell_nodes(move || {
                let session = lock(&seats);
                session
                    .observers()
                    .into_iter()
                    .flat_map(|observer| session.seats_of(observer))
                    .collect()
            });
            // The frame clears the flag at its next read, so the wm places windows again.
            self.queue
                .read(move |scene, _| scene.set_shell_places(lock(&shared).holder().is_some()));
        }
        for seat in seats.into_iter().flatten() {
            // One transaction for each seat, so one already gone does not keep the rest alive. It is admitted
            // whoever holds the tree, and nobody waits for its answer.
            let tx = TreeTransaction::new(vec![TreeOp::Destroy { node: seat }]);
            self.queue
                .submit(self.serial.load(Ordering::Acquire), tx, || true, |_, _| {});
        }
    }

    /// Answers `args` as `client`, or queues it for the frame. `Some` is the answer now: a
    /// connection that does not hold the tree is refused `not-privileged`, and a transaction
    /// that does not convert is refused with its op. `None` means the frame answers later,
    /// with `IpcEvent::TreeReply`.
    pub fn transact(&self, client: ClientId, request_id: u64, mut args: TransactArgs) -> Option<TreeReply> {
        let me = holder(client);
        let (is_holder, owned) = {
            let session = lock(&self.session);
            (
                session.holder() == Some(me),
                session.is_observer(me).then(|| session.seats_of(me)),
            )
        };
        if !is_holder && owned.is_none() {
            return Some(self.refusal(Code::NotPrivileged, -1));
        }
        let base_serial = args.base_serial;
        // Moved out, so the ops (and any call text they carry) leave `args` and are wiped by the
        // transaction that takes them, not kept alive by the request.
        let mut tx = match transaction(std::mem::take(&mut args.ops)) {
            Ok(tx) => tx,
            Err(refusal) => return Some(self.refusal(refusal.code, refusal.failed_op)),
        };
        // An observer's seats, as the transaction would make and destroy them. They are recorded once it applies.
        let mut seats = None;
        if let Some(owned) = owned.filter(|_| !is_holder) {
            match observer_ops(&tx.ops, &owned) {
                Ok(touched) => seats = Some(touched),
                Err(index) => {
                    return Some(self.refusal(Code::NotHolder, i32::try_from(index).unwrap_or(-1)));
                }
            }
            // The head pose is a measurement that arrives every frame: each observer may write it only so fast.
            if let Some(index) = writes_head_camera(&tx.ops)
                && !lock(&self.session).admit_pose(me, Instant::now())
            {
                return Some(self.refusal(Code::RateLimited, i32::try_from(index).unwrap_or(-1)));
            }
            tx = tx.from_observer();
        }
        let events = self.events.clone();
        // Asked again when the frame applies: a holder that detached while the transaction
        // waited for the frame has lost the tree, and the frame refuses it. An observer that
        // detached has lost its place the same way.
        let session = Arc::clone(&self.session);
        let admitted = {
            let session = Arc::clone(&session);
            move || {
                let session = lock(&session);
                session.holder() == Some(me) || session.is_observer(me)
            }
        };
        self.queue.submit(base_serial, tx, admitted, move |serial, result| {
            if let (Some((made, destroyed)), Ok(_)) = (&seats, &result) {
                lock(&session).record_seats(me, made, destroyed);
            }
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

/// One answer to a `call` the compositor settled, as the `answer` event.
#[must_use]
pub fn answer_payload(answer: &TreeAnswer) -> AnswerPayload {
    AnswerPayload {
        token: answer.token,
        result: answer.result.clone().into_bytes(),
        error: answer.error.clone(),
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
        TreeFrameInput::Ready => EventPayload {
            kind: "ready".to_owned(),
            ..base
        },
        TreeFrameInput::Requested => EventPayload {
            kind: "requested".to_owned(),
            ..base
        },
        // A claimed chord is sent as the `chord` signal, see `chord_payload`.
        TreeFrameInput::Chord(_) => EventPayload {
            kind: "chord".to_owned(),
            ..base
        },
        // A claimed gesture is sent as the `gesture` signal, see `gesture_payload`.
        TreeFrameInput::Gesture(_) => EventPayload {
            kind: "gesture".to_owned(),
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
                TreeInput::Key { pressed, keysym } => EventPayload {
                    kind,
                    key: keysym_name(keysym),
                    text: keysym_text(keysym),
                    pressed,
                    ..base
                },
            }
        }
    }
}

/// The `chord` signal for a claimed chord that changed state.
#[must_use]
pub fn chord_payload(chord: &ChordEvent) -> ChordPayload {
    ChordPayload {
        id: chord.id,
        chord: chord.chord.clone(),
        state: match chord.state {
            ChordState::Press => "press",
            ChordState::Release => "release",
        }
        .to_owned(),
    }
}

/// The `gesture` signal for a claimed touchpad gesture that advanced.
#[must_use]
pub fn gesture_payload(delivery: &GestureDelivery) -> GesturePayload {
    GesturePayload {
        id: delivery.id,
        kind: delivery.kind.as_str().to_owned(),
        fingers: i64::from(delivery.fingers),
        phase: delivery.phase.as_str().to_owned(),
        dx: delivery.dx,
        dy: delivery.dy,
        scale: delivery.scale,
        rotation: delivery.rotation,
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
        TreeChange::Set { node, key, value } => set_properties(*node, key, value),
        TreeChange::Destroyed { node, removed } => change::Properties {
            kind: "destroyed".to_owned(),
            node: node.get(),
            removed: removed.iter().map(|removed| removed.get()).collect(),
            ..Default::default()
        },
    }
}

/// The `set` record that carries `key` of `node` as `value`. The live `set` event and the
/// snapshot's property list both use it, so a value reads the same in either.
fn set_properties(node: NodeId, key: &str, value: &TreeValue) -> change::Properties {
    let mut properties = change::Properties {
        kind: "set".to_owned(),
        node: node.get(),
        key: key.to_owned(),
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
        TreeValue::ListText(value) => {
            "list_text".clone_into(&mut properties.value_kind);
            properties.value_texts.clone_from(value);
        }
        TreeValue::Effects(value) => {
            "list_effect".clone_into(&mut properties.value_kind);
            properties.value_effects = value.iter().map(wire_from_pass).collect();
        }
        TreeValue::Bytes(value) => {
            "bytes".clone_into(&mut properties.value_kind);
            properties.value_bytes.clone_from(value);
        }
    }
    properties
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
                    props: node
                        .props
                        .iter()
                        .map(|(key, value)| set_properties(node.id, key, value))
                        .collect(),
                })
                .collect(),
        })
        .collect()
}

/// A transaction from its wire ops. The first op that does not convert refuses the batch,
/// with its position as `failed_op`.
fn transaction(ops: Vec<op::Properties>) -> Result<TreeTransaction, TreeRefusal> {
    let converted = ops
        .into_iter()
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
fn tree_op(mut op: op::Properties) -> Result<TreeOp, Code> {
    let node = NodeId::new(op.node);
    Ok(match op.kind.as_str() {
        "new" => TreeOp::New {
            class: std::mem::take(&mut op.class),
            id: NodeId::new(op.id),
        },
        "insert" => TreeOp::Insert {
            node,
            parent: NodeId::new(op.parent),
            // A negative index appends.
            index: usize::try_from(op.index).ok(),
        },
        "destroy" => TreeOp::Destroy { node },
        "input" => TreeOp::Input {
            node,
            seat: NodeId::new(op.seat),
            input: pointer_input(&op)?,
        },
        "set" => TreeOp::Set {
            node,
            key: op.key.clone(),
            value: set_value(&op)?,
            timing: timing(&op),
        },
        "call" => {
            // The password (or chord) moves out of the wire op into a value that never prints and
            // is wiped on drop, before anything else can fail; the wire op is left empty.
            let text = SecretText::new(std::mem::take(&mut op.text));
            TreeOp::TreeCall {
                node,
                method: std::mem::take(&mut op.method),
                args: call_args(&op.args)?,
                text,
                token: op.token,
            }
        }
        _ => return Err(Code::BadValue),
    })
}

/// The value a `set` writes. The kinds the wire carries are bool, int, uint, double, text,
/// node (`0` is nil), a list of doubles, a list of texts, a list of effects and bytes; any other
/// kind is `bad-value`. A list of effects refuses the whole set when one entry is not a
/// pass the renderer can draw, so a partial chain is never written.
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
        "list_text" => Ok(TreeValue::ListText(op.value_texts.clone())),
        "bytes" => Ok(TreeValue::Bytes(op.value_bytes.clone())),
        "list_effect" => op
            .value_effects
            .iter()
            .map(|entry| pass_from_wire(entry).map_err(|_| Code::BadValue))
            .collect::<Result<Vec<_>, _>>()
            .map(TreeValue::Effects),
        _ => Err(Code::BadValue),
    }
}

/// The pointer input an `input` op carries. An unknown `input_kind`, or a button or state that
/// does not fit, is `bad-value`.
fn pointer_input(op: &op::Properties) -> Result<PointerInput, Code> {
    let button = u32::try_from(op.button).map_err(|_| Code::BadValue)?;
    Ok(match op.input_kind.as_str() {
        "motion" => PointerInput::Motion { x: op.x, y: op.y },
        "button" => PointerInput::Button {
            x: op.x,
            y: op.y,
            button,
            pressed: match op.state {
                0 => false,
                1 => true,
                _ => return Err(Code::BadValue),
            },
        },
        "scroll" => PointerInput::Scroll {
            x: op.x,
            y: op.y,
            dx: op.dx,
            dy: op.dy,
        },
        "leave" => PointerInput::Leave,
        _ => return Err(Code::BadValue),
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::tree::{TreeReply, snapshot_of};
    use gnoblin_render::{Actor, NodeSpec, Scene};
    use gnoblin_types::Rect;
    use gnoblin_wayland::TreeFrame;
    use prost::Message;
    use std::time::Instant;

    /// A secret a shell would send as a call's text argument.
    const PASSWORD: &str = "correct-horse-battery-staple";
    const ROOT: NodeId = NodeId::new(1);
    const SEAT: NodeId = NodeId::new(0x200);

    /// Fails if `needle` is in `bytes` or in the Debug text of the payload.
    fn assert_no_password(label: &str, wire: &[u8], debug: &str) {
        let needle = PASSWORD.as_bytes();
        assert!(
            !wire.windows(needle.len()).any(|window| window == needle),
            "{label}: the password is in the encoded bytes"
        );
        assert!(!debug.contains(PASSWORD), "{label}: the password is in the Debug text");
    }

    /// A seat claim carrying the password, applied through a real frame with the real schema.
    /// Every payload the compositor sends for it (reply, changes, snapshot) is encoded and
    /// scanned: the password must be in none of them, though the frame does keep it for the
    /// claim it runs.
    #[test]
    fn a_call_password_is_in_no_encoded_payload() {
        let (events, _events_rx) = std::sync::mpsc::channel();
        let host = TreeHostHandle::new(events).expect("host builds");
        let mut frame = TreeFrame::with_hooks(host.hooks());
        let mut scene = Scene::new();
        scene
            .insert_as(Actor::Compositor, ROOT, NodeSpec::new(Rect::new(0, 0, 100, 100)))
            .expect("root inserts");
        scene.set_root(Some(ROOT)).expect("root names");

        type Replies = Vec<(u64, Result<gnoblin_render::transaction::TreeApplied, TreeRefusal>)>;
        let replies: Arc<Mutex<Replies>> = Arc::default();
        let sink = Arc::clone(&replies);
        frame.queue().submit(
            0,
            TreeTransaction::new(vec![
                TreeOp::New {
                    class: "seat".to_owned(),
                    id: SEAT,
                },
                TreeOp::Insert {
                    node: SEAT,
                    parent: ROOT,
                    index: None,
                },
                TreeOp::TreeCall {
                    node: SEAT,
                    method: "claim".to_owned(),
                    args: vec![7],
                    text: SecretText::new(PASSWORD),
                    token: 1,
                },
            ]),
            || true,
            move |serial, result| sink.lock().unwrap().push((serial, result)),
        );
        frame.drain(&mut scene, Instant::now);

        let (serial, result) = replies.lock().unwrap().remove(0);
        let applied = result.expect("the transaction applies");
        assert_eq!(frame.take_claims().len(), 1, "the frame keeps the claim it runs");

        let changes = changes_payload(serial, &applied.changes);
        let reply = reply_payload(&TreeReply {
            serial,
            result: Ok(applied),
        });
        let snapshot = snapshot_payloads(&snapshot_of(&scene, &host.classes, serial));

        let changes_wire = changes.encode_to_vec();
        assert_no_password("changes", &changes_wire, &format!("{changes:?}"));
        let reply_wire = reply.encode_to_vec();
        assert_no_password("reply", &reply_wire, &format!("{reply:?}"));
        for payload in &snapshot {
            assert_no_password("snapshot", &payload.encode_to_vec(), &format!("{payload:?}"));
        }
    }
}
