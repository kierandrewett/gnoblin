//! The byte form of the shell's link, for a link that crosses the C ABI.
//!
//! A language runtime (`libgnoblin-runtime-js.so`) is a separate library from the core. It must not receive a Rust trait object
//! from the core (no Rust type crosses the boundary: design section 5.2), but a runtime that keeps its own mirror needs the
//! compositor's link as the core holds it. The core therefore relays: it hands the runtime the transactions it sends and the
//! messages it receives as bytes, and this module is the one place that turns a [`TreeTx`] and a list of [`TreeIncoming`] into
//! bytes and back. Both sides link this crate; a version skew between them would be a version skew of the two libraries, which
//! ship as one package, and the encoding starts with a format number so a mismatch is refused rather than misread.
//!
//! The form is little-endian, length-prefixed and self-delimiting: a `u32` count or length before every list and string, one
//! tag byte before every variant, fixed-width numbers. A decode that runs past the end, meets an unknown tag, or leaves bytes
//! over is an error, never a panic.

use gnoblin_proto::tree::effect;
use gnoblin_secret::SecretText;
use prost::Message as _;

use crate::tree_mirror::{
    Attached, EventFields, MirrorChange, MirrorChangeKind, MirrorValue, SnapshotNode, TreeChord, TreeEvent,
    TreeGesture, TreeIncoming, TreePointerButton, TreePointerChord, TreeTx, TxOp, TxReply,
};

/// The first byte of every encoding. Bumped when the form changes.
pub const FORMAT: u8 = 1;

/// Why bytes are not a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayError(pub String);

impl std::fmt::Display for RelayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cannot decode a relayed tree message: {}", self.0)
    }
}

impl std::error::Error for RelayError {}

type Result<T, E = RelayError> = std::result::Result<T, E>;

struct Encoder(Vec<u8>);

impl Encoder {
    fn new() -> Self {
        Self(vec![FORMAT])
    }

    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn i64(&mut self, value: i64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn f64(&mut self, value: f64) {
        self.u64(value.to_bits());
    }

    fn len(&mut self, len: usize) {
        self.u32(u32::try_from(len).unwrap_or(u32::MAX));
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.len(bytes.len());
        self.0.extend_from_slice(bytes);
    }

    fn str(&mut self, text: &str) {
        self.bytes(text.as_bytes());
    }

    fn option<T>(&mut self, value: Option<&T>, write: impl FnOnce(&mut Self, &T)) {
        match value {
            Some(value) => {
                self.u8(1);
                write(self, value);
            }
            None => self.u8(0),
        }
    }

    fn list<T>(&mut self, values: &[T], mut write: impl FnMut(&mut Self, &T)) {
        self.len(values.len());
        for value in values {
            write(self, value);
        }
    }
}

struct Decoder<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Decoder<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self> {
        match bytes.first() {
            Some(&FORMAT) => Ok(Self { bytes, at: 1 }),
            Some(other) => Err(RelayError(format!(
                "format {other}, this build reads format {FORMAT}: the runtime and the core are different versions"
            ))),
            None => Err(RelayError("no bytes".to_owned())),
        }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| {
                RelayError(format!(
                    "{count} bytes wanted at {}, {} in all",
                    self.at,
                    self.bytes.len()
                ))
            })?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    fn bool(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(RelayError(format!("{other} is not a boolean"))),
        }
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.array()?))
    }

    fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.u64()?))
    }

    fn len(&mut self) -> Result<usize> {
        Ok(self.u32()? as usize)
    }

    fn bytes(&mut self) -> Result<&'a [u8]> {
        let len = self.len()?;
        self.take(len)
    }

    fn string(&mut self) -> Result<String> {
        String::from_utf8(self.bytes()?.to_vec()).map_err(|_| RelayError("text that is not UTF-8".to_owned()))
    }

    fn option<T>(&mut self, read: impl FnOnce(&mut Self) -> Result<T>) -> Result<Option<T>> {
        match self.u8()? {
            0 => Ok(None),
            1 => read(self).map(Some),
            other => Err(RelayError(format!("{other} is not an option tag"))),
        }
    }

    fn list<T>(&mut self, mut read: impl FnMut(&mut Self) -> Result<T>) -> Result<Vec<T>> {
        let count = self.len()?;
        // A count the rest of the buffer cannot hold (each element is at least a byte) is corrupt: refuse before reserving.
        if count > self.bytes.len() - self.at {
            return Err(RelayError(format!(
                "a list of {count} in {} bytes",
                self.bytes.len() - self.at
            )));
        }
        (0..count).map(|_| read(self)).collect()
    }

    fn finish(&self) -> Result<()> {
        if self.at == self.bytes.len() {
            Ok(())
        } else {
            Err(RelayError(format!("{} bytes left over", self.bytes.len() - self.at)))
        }
    }
}

// ---- values ---------------------------------------------------------------------------------------------------------------

fn put_value(w: &mut Encoder, value: &MirrorValue) {
    match value {
        MirrorValue::Bool(v) => {
            w.u8(0);
            w.bool(*v);
        }
        MirrorValue::Int(v) => {
            w.u8(1);
            w.i64(*v);
        }
        MirrorValue::Uint(v) => {
            w.u8(2);
            w.u64(*v);
        }
        MirrorValue::Double(v) => {
            w.u8(3);
            w.f64(*v);
        }
        MirrorValue::Text(v) => {
            w.u8(4);
            w.str(v);
        }
        MirrorValue::Node(v) => {
            w.u8(5);
            w.u64(*v);
        }
        MirrorValue::List(v) => {
            w.u8(6);
            w.list(v, |w, n| w.f64(*n));
        }
        MirrorValue::Texts(v) => {
            w.u8(7);
            w.list(v, |w, t| w.str(t));
        }
        MirrorValue::Effects(v) => {
            w.u8(8);
            w.list(v, |w, e| w.bytes(&e.encode_to_vec()));
        }
        MirrorValue::Bytes(v) => {
            w.u8(9);
            w.bytes(v);
        }
    }
}

fn get_value(r: &mut Decoder<'_>) -> Result<MirrorValue> {
    Ok(match r.u8()? {
        0 => MirrorValue::Bool(r.bool()?),
        1 => MirrorValue::Int(r.i64()?),
        2 => MirrorValue::Uint(r.u64()?),
        3 => MirrorValue::Double(r.f64()?),
        4 => MirrorValue::Text(r.string()?),
        5 => MirrorValue::Node(r.u64()?),
        6 => MirrorValue::List(r.list(Decoder::f64)?),
        7 => MirrorValue::Texts(r.list(Decoder::string)?),
        8 => MirrorValue::Effects(r.list(|r| {
            effect::Properties::decode(r.bytes()?).map_err(|error| RelayError(format!("an effect: {error}")))
        })?),
        9 => MirrorValue::Bytes(r.bytes()?.to_vec()),
        other => return Err(RelayError(format!("{other} is not a value tag"))),
    })
}

// ---- a transaction --------------------------------------------------------------------------------------------------------

fn put_op(w: &mut Encoder, op: &TxOp) {
    match op {
        TxOp::New { id, class } => {
            w.u8(0);
            w.u64(*id);
            w.str(class);
        }
        TxOp::Set {
            node,
            key,
            value,
            seconds,
            delay,
            tween,
            samples,
        } => {
            w.u8(1);
            w.u64(*node);
            w.str(key);
            put_value(w, value);
            w.option(seconds.as_ref(), |w, v| w.f64(*v));
            w.option(delay.as_ref(), |w, v| w.f64(*v));
            w.option(tween.as_ref(), |w, v| w.u64(*v));
            w.option(samples.as_ref(), |w, v| w.list(v, |w, n| w.f64(*n)));
        }
        TxOp::Insert { node, parent, index } => {
            w.u8(2);
            w.u64(*node);
            w.u64(*parent);
            w.i64(*index);
        }
        TxOp::Destroy { node } => {
            w.u8(3);
            w.u64(*node);
        }
        TxOp::Call {
            node,
            method,
            args,
            token,
            text,
        } => {
            w.u8(4);
            w.u64(*node);
            w.str(method);
            w.bytes(args);
            w.u64(*token);
            w.str(text.expose());
        }
    }
}

fn get_op(r: &mut Decoder<'_>) -> Result<TxOp> {
    Ok(match r.u8()? {
        0 => TxOp::New {
            id: r.u64()?,
            class: r.string()?,
        },
        1 => TxOp::Set {
            node: r.u64()?,
            key: r.string()?,
            value: get_value(r)?,
            seconds: r.option(Decoder::f64)?,
            delay: r.option(Decoder::f64)?,
            tween: r.option(Decoder::u64)?,
            samples: r.option(|r| r.list(Decoder::f64))?,
        },
        2 => TxOp::Insert {
            node: r.u64()?,
            parent: r.u64()?,
            index: r.i64()?,
        },
        3 => TxOp::Destroy { node: r.u64()? },
        4 => TxOp::Call {
            node: r.u64()?,
            method: r.string()?,
            args: r.bytes()?.to_vec(),
            token: r.u64()?,
            text: SecretText::new(r.string()?),
        },
        other => return Err(RelayError(format!("{other} is not an op tag"))),
    })
}

/// The bytes of one transaction.
#[must_use]
pub fn encode_tx(tx: &TreeTx) -> Vec<u8> {
    let mut w = Encoder::new();
    w.u64(tx.base_serial);
    w.u64(tx.serial);
    w.list(&tx.ops, put_op);
    w.0
}

/// The transaction `bytes` encode.
///
/// # Errors
///
/// The bytes are not a transaction of this format.
pub fn decode_tx(bytes: &[u8]) -> Result<TreeTx> {
    let mut r = Decoder::new(bytes)?;
    let tx = TreeTx {
        base_serial: r.u64()?,
        serial: r.u64()?,
        ops: r.list(get_op)?,
    };
    r.finish()?;
    Ok(tx)
}

// ---- what the compositor sent ---------------------------------------------------------------------------------------------

fn put_change(w: &mut Encoder, change: &MirrorChange) {
    w.u8(match change.kind {
        MirrorChangeKind::Created => 0,
        MirrorChangeKind::Parented => 1,
        MirrorChangeKind::Set => 2,
        MirrorChangeKind::Destroyed => 3,
    });
    w.u64(change.node);
    w.str(&change.class);
    w.u64(change.parent);
    w.i64(change.index);
    w.str(&change.key);
    w.option(change.value.as_ref(), put_value);
    w.list(&change.removed, |w, id| w.u64(*id));
}

fn get_change(r: &mut Decoder<'_>) -> Result<MirrorChange> {
    let kind = match r.u8()? {
        0 => MirrorChangeKind::Created,
        1 => MirrorChangeKind::Parented,
        2 => MirrorChangeKind::Set,
        3 => MirrorChangeKind::Destroyed,
        other => return Err(RelayError(format!("{other} is not a change kind"))),
    };
    Ok(MirrorChange {
        kind,
        node: r.u64()?,
        class: r.string()?,
        parent: r.u64()?,
        index: r.i64()?,
        key: r.string()?,
        value: r.option(get_value)?,
        removed: r.list(Decoder::u64)?,
    })
}

fn put_snapshot_node(w: &mut Encoder, node: &SnapshotNode) {
    w.u64(node.id);
    w.u64(node.parent);
    w.i64(node.index);
    w.str(&node.class);
    w.str(&node.title);
    w.str(&node.app_id);
    w.list(&node.props, |w, (key, value)| {
        w.str(key);
        put_value(w, value);
    });
}

fn get_snapshot_node(r: &mut Decoder<'_>) -> Result<SnapshotNode> {
    Ok(SnapshotNode {
        id: r.u64()?,
        parent: r.u64()?,
        index: r.i64()?,
        class: r.string()?,
        title: r.string()?,
        app_id: r.string()?,
        props: r.list(|r| Ok((r.string()?, get_value(r)?)))?,
    })
}

fn put_incoming(w: &mut Encoder, message: &TreeIncoming) {
    match message {
        TreeIncoming::Attached(a) => {
            w.u8(0);
            w.u64(a.epoch);
            w.u64(a.root);
            w.u64(a.serial);
        }
        TreeIncoming::Snapshot {
            serial,
            chunk,
            last,
            nodes,
        } => {
            w.u8(1);
            w.u64(*serial);
            w.u32(*chunk);
            w.bool(*last);
            w.list(nodes, put_snapshot_node);
        }
        TreeIncoming::Changes { serial, changes } => {
            w.u8(2);
            w.u64(*serial);
            w.list(changes, put_change);
        }
        TreeIncoming::Reply(reply) => {
            w.u8(3);
            w.u64(reply.serial);
            w.bool(reply.applied);
            w.i64(reply.failed_op);
            w.str(&reply.code);
            w.list(&reply.removed, |w, id| w.u64(*id));
        }
        TreeIncoming::Answer { token, result, error } => {
            w.u8(4);
            w.u64(*token);
            w.bytes(result);
            w.str(error);
        }
        TreeIncoming::TrackEnded {
            node,
            property,
            completed,
            tween,
        } => {
            w.u8(5);
            w.u64(*node);
            w.str(property);
            w.bool(*completed);
            w.option(tween.as_ref(), |w, id| w.u64(*id));
        }
        TreeIncoming::Event(event) => {
            w.u8(6);
            w.u64(event.frame);
            w.u64(event.seq);
            w.u64(event.node);
            w.str(&event.kind);
            let f = &event.fields;
            w.f64(f.x);
            w.f64(f.y);
            w.u64(f.button);
            w.f64(f.dx);
            w.f64(f.dy);
            w.str(&f.key);
            w.str(&f.text);
            w.bool(f.pressed);
            w.str(&f.request);
        }
        TreeIncoming::Chord(chord) => {
            w.u8(7);
            w.i64(chord.claim);
            w.str(&chord.chord);
            w.str(&chord.state);
        }
        TreeIncoming::Gesture(g) => {
            w.u8(8);
            w.i64(g.claim);
            w.str(&g.kind);
            w.i64(g.fingers);
            w.str(&g.phase);
            w.f64(g.dx);
            w.f64(g.dy);
            w.f64(g.scale);
            w.f64(g.rotation);
        }
        TreeIncoming::PointerChord(p) => {
            w.u8(9);
            w.i64(p.claim);
            w.str(&p.phase);
            w.f64(p.x);
            w.f64(p.y);
            w.f64(p.dx);
            w.f64(p.dy);
            w.i64(p.button);
            w.str(&p.modifiers);
            w.i64(p.time);
        }
        TreeIncoming::PointerButton(p) => {
            w.u8(10);
            w.i64(p.claim);
            w.i64(p.button);
            w.str(&p.state);
            w.f64(p.x);
            w.f64(p.y);
            w.i64(p.time);
        }
    }
}

fn get_incoming(r: &mut Decoder<'_>) -> Result<TreeIncoming> {
    Ok(match r.u8()? {
        0 => TreeIncoming::Attached(Attached {
            epoch: r.u64()?,
            root: r.u64()?,
            serial: r.u64()?,
        }),
        1 => TreeIncoming::Snapshot {
            serial: r.u64()?,
            chunk: r.u32()?,
            last: r.bool()?,
            nodes: r.list(get_snapshot_node)?,
        },
        2 => TreeIncoming::Changes {
            serial: r.u64()?,
            changes: r.list(get_change)?,
        },
        3 => TreeIncoming::Reply(TxReply {
            serial: r.u64()?,
            applied: r.bool()?,
            failed_op: r.i64()?,
            code: r.string()?,
            removed: r.list(Decoder::u64)?,
        }),
        4 => TreeIncoming::Answer {
            token: r.u64()?,
            result: r.bytes()?.to_vec(),
            error: r.string()?,
        },
        5 => TreeIncoming::TrackEnded {
            node: r.u64()?,
            property: r.string()?,
            completed: r.bool()?,
            tween: r.option(Decoder::u64)?,
        },
        6 => TreeIncoming::Event(TreeEvent {
            frame: r.u64()?,
            seq: r.u64()?,
            node: r.u64()?,
            kind: r.string()?,
            fields: EventFields {
                x: r.f64()?,
                y: r.f64()?,
                button: r.u64()?,
                dx: r.f64()?,
                dy: r.f64()?,
                key: r.string()?,
                text: r.string()?,
                pressed: r.bool()?,
                request: r.string()?,
            },
        }),
        7 => TreeIncoming::Chord(TreeChord {
            claim: r.i64()?,
            chord: r.string()?,
            state: r.string()?,
        }),
        8 => TreeIncoming::Gesture(TreeGesture {
            claim: r.i64()?,
            kind: r.string()?,
            fingers: r.i64()?,
            phase: r.string()?,
            dx: r.f64()?,
            dy: r.f64()?,
            scale: r.f64()?,
            rotation: r.f64()?,
        }),
        9 => TreeIncoming::PointerChord(TreePointerChord {
            claim: r.i64()?,
            phase: r.string()?,
            x: r.f64()?,
            y: r.f64()?,
            dx: r.f64()?,
            dy: r.f64()?,
            button: r.i64()?,
            modifiers: r.string()?,
            time: r.i64()?,
        }),
        10 => TreeIncoming::PointerButton(TreePointerButton {
            claim: r.i64()?,
            button: r.i64()?,
            state: r.string()?,
            x: r.f64()?,
            y: r.f64()?,
            time: r.i64()?,
        }),
        other => return Err(RelayError(format!("{other} is not a message tag"))),
    })
}

/// The bytes of a batch of incoming messages, in arrival order. An empty batch is a valid encoding.
#[must_use]
pub fn encode_incoming(messages: &[TreeIncoming]) -> Vec<u8> {
    let mut w = Encoder::new();
    w.list(messages, put_incoming);
    w.0
}

/// The messages `bytes` encode.
///
/// # Errors
///
/// The bytes are not a batch of this format.
pub fn decode_incoming(bytes: &[u8]) -> Result<Vec<TreeIncoming>> {
    let mut r = Decoder::new(bytes)?;
    let messages = r.list(get_incoming)?;
    r.finish()?;
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_value_kinds() -> Vec<MirrorValue> {
        vec![
            MirrorValue::Bool(true),
            MirrorValue::Int(-7),
            MirrorValue::Uint(u64::MAX),
            MirrorValue::Double(1.5),
            MirrorValue::Text("caf\u{e9}".to_owned()),
            MirrorValue::Node(1 << 63),
            MirrorValue::List(vec![0.0, -0.0, 3.25]),
            MirrorValue::Texts(vec!["a".to_owned(), String::new()]),
            MirrorValue::Effects(vec![effect::Properties::default()]),
            MirrorValue::Bytes(vec![0, 255, 7]),
        ]
    }

    #[test]
    fn a_transaction_round_trips_every_op_and_value_kind() {
        let mut ops = vec![
            TxOp::New {
                id: 9,
                class: "canvas".to_owned(),
            },
            TxOp::Insert {
                node: 9,
                parent: 1,
                index: -1,
            },
            TxOp::Destroy { node: 9 },
            TxOp::Call {
                node: 3,
                method: "claim".to_owned(),
                args: vec![1, 2, 3],
                token: 44,
                text: SecretText::new("Super+Return"),
            },
        ];
        for value in sample_value_kinds() {
            ops.push(TxOp::Set {
                node: 9,
                key: "k".to_owned(),
                value,
                seconds: Some(0.25),
                delay: None,
                tween: Some(5),
                samples: Some(vec![0.0, 1.0]),
            });
        }
        let tx = TreeTx {
            base_serial: 10,
            serial: 11,
            ops,
        };
        assert_eq!(decode_tx(&encode_tx(&tx)), Ok(tx));
    }

    #[test]
    fn every_incoming_message_round_trips() {
        let messages = vec![
            TreeIncoming::Attached(Attached {
                epoch: 2,
                root: 1,
                serial: 5,
            }),
            TreeIncoming::Snapshot {
                serial: 5,
                chunk: 0,
                last: true,
                nodes: vec![SnapshotNode {
                    id: 4,
                    parent: 1,
                    index: 0,
                    class: "window".to_owned(),
                    title: "t".to_owned(),
                    app_id: "a".to_owned(),
                    props: sample_value_kinds().into_iter().map(|v| ("p".to_owned(), v)).collect(),
                }],
            },
            TreeIncoming::Changes {
                serial: 6,
                changes: vec![MirrorChange {
                    kind: MirrorChangeKind::Set,
                    node: 4,
                    class: "window".to_owned(),
                    parent: 1,
                    index: -1,
                    key: "x".to_owned(),
                    value: Some(MirrorValue::Double(2.0)),
                    removed: vec![7, 8],
                }],
            },
            TreeIncoming::Reply(TxReply {
                serial: 6,
                applied: false,
                failed_op: 2,
                code: "stale-base".to_owned(),
                removed: vec![3],
            }),
            TreeIncoming::Answer {
                token: 1,
                result: vec![1],
                error: "no".to_owned(),
            },
            TreeIncoming::TrackEnded {
                node: 4,
                property: "x".to_owned(),
                completed: true,
                tween: None,
            },
            TreeIncoming::Event(TreeEvent {
                frame: 1,
                seq: 2,
                node: 4,
                kind: "key".to_owned(),
                fields: EventFields {
                    x: 1.0,
                    y: 2.0,
                    button: 3,
                    dx: 4.0,
                    dy: 5.0,
                    key: "a".to_owned(),
                    text: "b".to_owned(),
                    pressed: true,
                    request: "move".to_owned(),
                },
            }),
            TreeIncoming::Chord(TreeChord {
                claim: 1,
                chord: "Super+a".to_owned(),
                state: "press".to_owned(),
            }),
            TreeIncoming::Gesture(TreeGesture {
                claim: 2,
                kind: "swipe".to_owned(),
                fingers: 3,
                phase: "update".to_owned(),
                dx: 1.0,
                dy: 2.0,
                scale: 1.0,
                rotation: 0.5,
            }),
            TreeIncoming::PointerChord(TreePointerChord {
                claim: 3,
                phase: "start".to_owned(),
                x: 1.0,
                y: 2.0,
                dx: 0.0,
                dy: 0.0,
                button: 1,
                modifiers: "Super".to_owned(),
                time: 99,
            }),
            TreeIncoming::PointerButton(TreePointerButton {
                claim: 4,
                button: 272,
                state: "press".to_owned(),
                x: 1.0,
                y: 2.0,
                time: 100,
            }),
        ];
        assert_eq!(decode_incoming(&encode_incoming(&messages)), Ok(messages));
        assert_eq!(decode_incoming(&encode_incoming(&[])), Ok(Vec::new()));
    }

    #[test]
    fn corrupt_bytes_are_an_error_and_never_a_panic() {
        let good = encode_incoming(&[TreeIncoming::Chord(TreeChord {
            claim: 1,
            chord: "x".to_owned(),
            state: "press".to_owned(),
        })]);
        for cut in 0..good.len() {
            assert!(decode_incoming(&good[..cut]).is_err(), "cut at {cut}");
        }
        let mut extra = good.clone();
        extra.push(0);
        assert!(decode_incoming(&extra).is_err(), "trailing bytes");
        let mut unknown = good.clone();
        unknown[5] = 200;
        assert!(decode_incoming(&unknown).is_err(), "an unknown tag");
        let mut other_format = good;
        other_format[0] = FORMAT + 1;
        let error = decode_incoming(&other_format).unwrap_err();
        assert!(error.0.contains("different versions"), "{error}");
        assert!(
            decode_incoming(&[FORMAT, 255, 255, 255, 255]).is_err(),
            "a count past the end"
        );
    }
}
