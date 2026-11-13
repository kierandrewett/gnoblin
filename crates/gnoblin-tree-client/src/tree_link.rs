//! The owner socket behind [`TreeLink`]: the `tree` namespace of a session socket, carried in
//! the envelopes `gnoblin_proto` defines. This module owns framing and the wire-to-Lua
//! translation; the mirror owns what the messages mean.
//!
//! Requests are sent and not awaited. `attach` and `transact` return once the frame is
//! written; their answers come back through [`TreeLink::poll`], in arrival order.

use std::io::{self, Read};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use gnoblin_proto::tree::seat::{ChordPayload, GesturePayload};
use gnoblin_proto::tree::tree::{
    AnswerPayload, AttachArgs, AttachedPayload, ChangesPayload, EventPayload, ReplyPayload, SnapshotPayload,
    TransactArgs,
};
use gnoblin_proto::tree::{change, op, snapshot_node};
use gnoblin_proto::{Envelope, Kind, LENGTH_PREFIX_LEN, MAX_FRAME_LEN, read_frame, write_frame};

use crate::tree_mirror::{
    Attached, EventFields, LinkError, MirrorChange, MirrorChangeKind, MirrorValue, SnapshotNode, TreeChord, TreeEvent,
    TreeGesture, TreeIncoming, TreeLink, TreeTx, TxOp, TxReply,
};

const NAMESPACE: &str = "tree";

/// How long one read waits for bytes that have not arrived. Short, so a poll never stalls the
/// caller: whatever has arrived by then is returned, and the rest comes on a later poll.
const READ_WAIT: Duration = Duration::from_millis(1);

/// How long a write may block before the compositor is treated as stuck.
const WRITE_WAIT: Duration = Duration::from_secs(5);

/// The tree namespace over a session socket that has already completed its handshake and
/// grants.
///
/// Bytes are buffered here rather than read with `read_frame` directly: a read that times out
/// part-way through a frame would otherwise lose the bytes it had taken.
#[derive(Debug)]
pub struct SocketTreeLink {
    stream: UnixStream,
    inbox: Vec<u8>,
    pending: Vec<TreeIncoming>,
    next_request: u64,
}

impl SocketTreeLink {
    /// Wrap the client end of a handshaken session socket.
    ///
    /// # Errors
    ///
    /// The socket refuses the read or write timeout it needs.
    pub fn new(stream: UnixStream) -> io::Result<Self> {
        stream.set_read_timeout(Some(READ_WAIT))?;
        stream.set_write_timeout(Some(WRITE_WAIT))?;
        Ok(Self {
            stream,
            inbox: Vec::new(),
            pending: Vec::new(),
            next_request: 1,
        })
    }

    fn take_request_id(&mut self) -> u64 {
        let id = self.next_request;
        self.next_request += 1;
        id
    }

    fn send(&mut self, envelope: &Envelope) -> Result<(), LinkError> {
        write_frame(&mut self.stream, envelope)
            .map_err(|error| LinkError(format!("writing to the tree socket: {error}")))
    }

    /// Read whatever bytes are waiting. `Ok(false)` means nothing arrived before the read timed out.
    fn fill(&mut self) -> Result<bool, LinkError> {
        let mut chunk = [0u8; 4096];
        match self.stream.read(&mut chunk) {
            Ok(0) => Err(LinkError("the compositor closed the tree socket".to_owned())),
            Ok(read) => {
                self.inbox.extend_from_slice(&chunk[..read]);
                Ok(true)
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
                ) =>
            {
                Ok(false)
            }
            Err(error) => Err(LinkError(format!("reading the tree socket: {error}"))),
        }
    }

    /// Take the next complete frame out of the inbox, if one has fully arrived.
    fn next_envelope(&mut self) -> Result<Option<Envelope>, LinkError> {
        let Some(prefix) = self.inbox.first_chunk::<LENGTH_PREFIX_LEN>() else {
            return Ok(None);
        };
        let len = u32::from_be_bytes(*prefix) as usize;
        if len > MAX_FRAME_LEN {
            return Err(LinkError(format!(
                "a tree frame of {len} bytes is over the {MAX_FRAME_LEN} byte limit"
            )));
        }
        let total = LENGTH_PREFIX_LEN + len;
        if self.inbox.len() < total {
            return Ok(None);
        }
        let frame: Vec<u8> = self.inbox.drain(..total).collect();
        match read_frame(&mut frame.as_slice()) {
            Ok(Some(envelope)) => Ok(Some(envelope)),
            Ok(None) => Err(LinkError("an empty tree frame".to_owned())),
            Err(error) => Err(LinkError(format!("a tree frame does not decode: {error}"))),
        }
    }
}

impl TreeLink for SocketTreeLink {
    fn attach(&mut self, version: u64) -> Result<(), LinkError> {
        let id = self.take_request_id();
        self.send(&Envelope::request_message(
            NAMESPACE,
            "attach",
            id,
            &AttachArgs {
                version,
                mode: String::new(),
            },
        ))
    }

    fn transact(&mut self, tx: &TreeTx) -> Result<(), LinkError> {
        let mut args = TransactArgs {
            base_serial: tx.base_serial,
            serial: tx.serial,
            ops: tx.ops.iter().map(to_op).collect(),
        };
        let id = self.take_request_id();
        // The frame and the wire ops carry the call texts, which may be passwords. Both are
        // wiped once the frame has been written, whether or not the write succeeded.
        let mut envelope = Envelope::request_message(NAMESPACE, "transact", id, &args);
        let sent = self.send(&envelope);
        envelope.wipe_secret();
        for op in &mut args.ops {
            gnoblin_secret::wipe(&mut op.text);
        }
        sent
    }

    fn poll(&mut self) -> Result<Vec<TreeIncoming>, LinkError> {
        loop {
            while let Some(envelope) = self.next_envelope()? {
                let incoming = to_incoming(&envelope)?;
                self.pending.push(incoming);
            }
            if !self.fill()? {
                break;
            }
        }
        Ok(std::mem::take(&mut self.pending))
    }
}

/// The wire form of one write. The kind names are the ones the compositor's tree host reads.
fn to_op(op: &TxOp) -> op::Properties {
    match op {
        TxOp::New { id, class } => op::Properties {
            kind: "new".to_owned(),
            id: *id,
            class: class.clone(),
            ..Default::default()
        },
        TxOp::Insert { node, parent, index } => op::Properties {
            kind: "insert".to_owned(),
            node: *node,
            parent: *parent,
            index: *index,
            ..Default::default()
        },
        TxOp::Set {
            node,
            key,
            value,
            seconds,
            samples,
        } => {
            let mut wire = op::Properties {
                kind: "set".to_owned(),
                node: *node,
                key: key.clone(),
                // An instant set has neither a duration nor samples on the wire.
                seconds: seconds.unwrap_or(0.0),
                samples: samples.clone().unwrap_or_default(),
                ..Default::default()
            };
            match value {
                MirrorValue::Bool(v) => {
                    "bool".clone_into(&mut wire.value_kind);
                    wire.value_bool = *v;
                }
                MirrorValue::Int(v) => {
                    "int".clone_into(&mut wire.value_kind);
                    wire.value_int = *v;
                }
                MirrorValue::Uint(v) => {
                    "uint".clone_into(&mut wire.value_kind);
                    wire.value_uint = *v;
                }
                MirrorValue::Double(v) => {
                    "double".clone_into(&mut wire.value_kind);
                    wire.value_double = *v;
                }
                MirrorValue::Text(v) => {
                    "text".clone_into(&mut wire.value_kind);
                    wire.value_text = v.clone();
                }
                MirrorValue::Node(v) => {
                    "node".clone_into(&mut wire.value_kind);
                    wire.value_node = *v;
                }
                MirrorValue::List(v) => {
                    "list_double".clone_into(&mut wire.value_kind);
                    wire.value_list = v.clone();
                }
                MirrorValue::Texts(v) => {
                    "list_text".clone_into(&mut wire.value_kind);
                    wire.value_texts = v.clone();
                }
                MirrorValue::Effects(v) => {
                    "list_effect".clone_into(&mut wire.value_kind);
                    wire.value_effects = v.clone();
                }
                MirrorValue::Bytes(v) => {
                    "bytes".clone_into(&mut wire.value_kind);
                    wire.value_bytes = v.clone();
                }
            }
            wire
        }
        TxOp::Destroy { node } => op::Properties {
            kind: "destroy".to_owned(),
            node: *node,
            ..Default::default()
        },
        TxOp::Call {
            node,
            method,
            args,
            token,
            text,
        } => op::Properties {
            kind: "call".to_owned(),
            node: *node,
            method: method.clone(),
            args: args.clone(),
            token: *token,
            text: text.expose().to_owned(),
            ..Default::default()
        },
    }
}

/// Translate one inbound envelope. Anything the tree namespace does not define, and any
/// compositor refusal, is a [`LinkError`]: the mirror cannot continue past a message it cannot read.
fn to_incoming(envelope: &Envelope) -> Result<TreeIncoming, LinkError> {
    if envelope.namespace != NAMESPACE {
        return Err(LinkError(format!(
            "a message on the {} namespace came over the tree socket",
            envelope.namespace
        )));
    }
    let member = envelope.member.as_str();
    match (&envelope.kind, member) {
        (Some(Kind::Error(_)), _) => Err(LinkError(format!(
            "the compositor refused tree {member}: {}",
            envelope.error_code().unwrap_or("unknown")
        ))),
        (Some(Kind::Reply(_)), "attach") => {
            let payload = envelope
                .payload_as::<AttachedPayload>()
                .ok_or_else(|| undecodable(member))?;
            Ok(TreeIncoming::Attached(Attached {
                epoch: payload.epoch,
                root: payload.root,
                serial: payload.serial,
            }))
        }
        (Some(Kind::Reply(_)), "transact") => {
            let payload = envelope
                .payload_as::<ReplyPayload>()
                .ok_or_else(|| undecodable(member))?;
            Ok(TreeIncoming::Reply(TxReply {
                serial: payload.serial,
                applied: payload.applied,
                failed_op: payload.failed_op,
                code: payload.code,
                removed: payload.removed,
            }))
        }
        (Some(Kind::Event(_)), "snapshot") => {
            let payload = envelope
                .payload_as::<SnapshotPayload>()
                .ok_or_else(|| undecodable(member))?;
            let chunk = u32::try_from(payload.chunk)
                .map_err(|_| LinkError(format!("snapshot chunk {} is out of range", payload.chunk)))?;
            Ok(TreeIncoming::Snapshot {
                serial: payload.serial,
                chunk,
                last: payload.last,
                nodes: payload
                    .nodes
                    .into_iter()
                    .map(to_snapshot_node)
                    .collect::<Result<Vec<_>, _>>()?,
            })
        }
        (Some(Kind::Event(_)), "event") => {
            let payload = envelope
                .payload_as::<EventPayload>()
                .ok_or_else(|| undecodable(member))?;
            Ok(TreeIncoming::Event(TreeEvent {
                frame: payload.frame,
                seq: payload.seq,
                node: payload.node,
                kind: payload.kind,
                fields: EventFields {
                    x: payload.x,
                    y: payload.y,
                    button: payload.button,
                    dx: payload.dx,
                    dy: payload.dy,
                    key: payload.key,
                    text: payload.text,
                    pressed: payload.pressed,
                    request: payload.request,
                },
            }))
        }
        (Some(Kind::Event(_)), "chord") => {
            let payload = envelope
                .payload_as::<ChordPayload>()
                .ok_or_else(|| undecodable(member))?;
            Ok(TreeIncoming::Chord(TreeChord {
                claim: payload.id,
                chord: payload.chord,
                state: payload.state,
            }))
        }
        (Some(Kind::Event(_)), "gesture") => {
            let payload = envelope
                .payload_as::<GesturePayload>()
                .ok_or_else(|| undecodable(member))?;
            Ok(TreeIncoming::Gesture(TreeGesture {
                claim: payload.id,
                kind: payload.kind,
                fingers: payload.fingers,
                phase: payload.phase,
                dx: payload.dx,
                dy: payload.dy,
                scale: payload.scale,
                rotation: payload.rotation,
            }))
        }
        (Some(Kind::Event(_)), "changes") => {
            let payload = envelope
                .payload_as::<ChangesPayload>()
                .ok_or_else(|| undecodable(member))?;
            let changes = payload
                .changes
                .into_iter()
                .map(to_change)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(TreeIncoming::Changes {
                serial: payload.serial,
                changes,
            })
        }
        (Some(Kind::Event(_)), "answer") => {
            let payload = envelope
                .payload_as::<AnswerPayload>()
                .ok_or_else(|| undecodable(member))?;
            Ok(TreeIncoming::Answer {
                token: payload.token,
                result: payload.result,
                error: payload.error,
            })
        }
        _ => Err(LinkError(format!(
            "an unexpected tree message {member} came over the tree socket"
        ))),
    }
}

fn undecodable(member: &str) -> LinkError {
    LinkError(format!("the tree {member} payload does not decode"))
}

fn to_snapshot_node(node: snapshot_node::Properties) -> Result<SnapshotNode, LinkError> {
    let props = node
        .props
        .iter()
        .map(|prop| Ok((prop.key.clone(), to_value(prop)?)))
        .collect::<Result<Vec<_>, LinkError>>()?;
    Ok(SnapshotNode {
        id: node.id,
        parent: node.parent,
        index: node.index,
        class: node.class,
        title: node.title,
        app_id: node.app_id,
        props,
    })
}

fn to_change(change: change::Properties) -> Result<MirrorChange, LinkError> {
    let kind = match change.kind.as_str() {
        "created" => MirrorChangeKind::Created,
        "parented" => MirrorChangeKind::Parented,
        "set" => MirrorChangeKind::Set,
        "destroyed" => MirrorChangeKind::Destroyed,
        other => {
            return Err(LinkError(format!(
                "an unknown tree change kind {other} came over the tree socket"
            )));
        }
    };
    let value = if kind == MirrorChangeKind::Set {
        Some(to_value(&change)?)
    } else {
        None
    };
    Ok(MirrorChange {
        kind,
        node: change.node,
        class: change.class,
        parent: change.parent,
        index: change.index,
        key: change.key,
        value,
        removed: change.removed,
    })
}

/// The value a `set` change carries. A kind the Lua side has no form for is a refusal, not a
/// silent `nil`: the mirror would otherwise disagree with the compositor about the value.
fn to_value(change: &change::Properties) -> Result<MirrorValue, LinkError> {
    match change.value_kind.as_str() {
        "bool" => Ok(MirrorValue::Bool(change.value_bool)),
        "int" => Ok(MirrorValue::Int(change.value_int)),
        "uint" => Ok(MirrorValue::Uint(change.value_uint)),
        "double" => Ok(MirrorValue::Double(change.value_double)),
        "text" => Ok(MirrorValue::Text(change.value_text.clone())),
        "node" => Ok(MirrorValue::Node(change.value_node)),
        "list_double" => Ok(MirrorValue::List(change.value_list.clone())),
        "list_text" => Ok(MirrorValue::Texts(change.value_texts.clone())),
        "list_effect" => Ok(MirrorValue::Effects(change.value_effects.clone())),
        "bytes" => Ok(MirrorValue::Bytes(change.value_bytes.clone())),
        other => Err(LinkError(format!("a tree value of kind {other} has no Lua form"))),
    }
}
