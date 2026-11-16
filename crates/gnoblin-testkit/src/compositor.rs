//! An in-memory compositor behind the [`TreeLink`] seam: the fake every shell runtime's tests share.
//!
//! [`FakeCompositor`] applies `New`, `Insert`, `Set` and `Destroy` as the real compositor does, echoes each as a change batch,
//! answers a transaction with a reply, and can be made to misbehave in the ways the real one does ([`Fault`]: stale bases,
//! refusals, a late or dead link). It also keeps what the shell did to it (the ops it sent, the seat claims it holds) so a test
//! asserts on the compositor's side. It is shared by `gnoblin-js`'s suite and the native shell core's (`gnoblin-core-impl`), so
//! there is one fake and not one per runtime.
//!
//! The fake is single-threaded (`Rc`): a test drives it from the thread that runs the shell.
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

use gnoblin_tree_client::tree_mirror::{
    Attached, EventFields, LinkError, MirrorChange, MirrorChangeKind, MirrorValue, SnapshotNode, TreeEvent,
    TreeIncoming, TreeLink, TreeTx, TxOp, TxReply,
};

pub const ROOT: u64 = 1;
pub const LEFT: u64 = 272;

// ---- the fake compositor ------------------------------------------------------------------------------------------

pub struct Node {
    pub class: String,
    pub parent: u64,
    pub children: Vec<u64>,
    pub props: BTreeMap<String, MirrorValue>,
}

/// A call the shell sent, with the secret text it carried.
#[derive(Clone, Debug, PartialEq)]
pub struct SentCall {
    pub node: u64,
    pub method: String,
    pub args: Vec<u8>,
    pub token: u64,
    pub text: String,
}

/// A `measure` call the shell sent.
#[derive(Clone, Debug, PartialEq)]
pub struct Measure {
    pub token: u64,
    pub node: u64,
    pub size: f64,
    pub text: String,
}

/// A claim the shell holds on a seat: a chord, a gesture, a device, the buttons of a pointer or an input grab, by the id it
/// allocated.
#[derive(Clone, Debug, PartialEq)]
pub struct Claim {
    pub id: i64,
    /// `claim`, `claim_gesture`, `claim_device`, `claim_pointer` or `grab`.
    pub method: String,
    /// The call's text: the chord, the gesture's kind, the device name or a grab's scope, behind `device` and a line feed
    /// when scoped.
    pub text: String,
    /// The claim's second integer: a chord's grab flag, a gesture's finger count or a grab's node; 0 for a device.
    pub second: i64,
    /// The seat node the call was made on.
    pub seat: u64,
}

/// A pointer chord's drag as the fake streams it: where it began and what the chord is.
#[derive(Clone, Debug, PartialEq)]
pub struct Drag {
    pub seat: u64,
    pub origin: (f64, f64),
    pub button: i64,
    pub modifiers: String,
}

/// How the fake answers a transaction instead of applying it, in the words of the real compositor
/// (`gnoblin_render::tree_revision`, `gnoblin_render::transaction`), or how the link behaves around it. Faults are
/// queued with [`FakeCompositor::fault`] and each one takes the next transaction the shell sends, in order; once the
/// queue is empty every transaction applies again.
#[derive(Clone, Debug)]
pub enum Fault {
    /// `stale-base`: a batch the shell had not read changed a node the transaction names. `racing` is that batch: the
    /// compositor publishes it (the serial moves, the node's change is announced) and then refuses, applying nothing.
    /// `failed_op` is the index of the first op on a node `racing` changed when it is `None`, as the real check names
    /// it; `Some(-1)` is the refusal of a base older than the history the compositor keeps, which names no op.
    StaleBase {
        failed_op: Option<i64>,
        racing: Vec<MirrorChange>,
    },
    /// `unknown-node` at `failed_op`: the compositor destroyed a node the transaction writes, and the transaction was
    /// built before the shell read that. The destroy is published first, as it is in the real order.
    UnknownNode { failed_op: i64, destroyed: u64 },
    /// A refusal for a reason that is the script's and not the base's, spelled as the compositor's table spells it
    /// (`read-only`, `not-finite`, `too-large`, `bad-value`, ...). `failed_op` is the first op that failed, or `-1` for
    /// the whole batch. It applies nothing and publishes nothing, so no newer base cures it: the same op sent again is
    /// refused again.
    Refuse { code: &'static str, failed_op: i64 },
    /// The compositor applies the transaction and answers it, but nothing it says from now on (this answer and
    /// everything behind it, in order) reaches the shell until the shell has polled `polls` times: a compositor that
    /// is slow to answer.
    Late { polls: usize },
    /// The link dies as the transaction is sent: nothing is applied and `transact` fails, as a write to a socket the
    /// compositor has closed does. Every later call on the link fails too, until [`FakeCompositor::reconnect`].
    Disconnect,
}

pub struct Compositor {
    pub nodes: BTreeMap<u64, Node>,
    /// The claims made and not given up, in the order they were made.
    pub claims: Vec<Claim>,
    pub serial: u64,
    /// The ops of each transaction the shell sent, in order.
    pub sent: Vec<Vec<TxOp>>,
    pub measures: Vec<Measure>,
    pub queue: Vec<TreeIncoming>,
    /// The faults the next transactions meet, oldest first.
    pub faults: VecDeque<Fault>,
    /// How many transactions were refused.
    pub refused: usize,
    /// While above zero, a poll returns nothing and counts it down: what [`Fault::Late`] sets.
    pub stall: usize,
    /// Whether the link is dead: every call on it fails. See [`Fault::Disconnect`].
    pub dead: bool,
    /// How many of the batches the compositor publishes next are lost on the way.
    pub drop_batches: usize,
    /// Refuse every `n`th transaction `stale-base`, naming no op, and apply the others; `0` refuses none. A refusal of
    /// that kind changes nothing the shell can read, so a suite that passes without it passes with it too. Set from
    /// `GNOBLIN_FAKE_REFUSE_EVERY`, so a whole suite can be run with refusals turned on.
    pub refuse_every: usize,
    /// How many transactions have landed (refused ones included), for `refuse_every`.
    pub landed: usize,
    /// The claim calls the shell made, in order, unclaims included: `(method, id, second, text)`.
    pub claim_calls: Vec<(String, i64, i64, String)>,
    /// The claims the compositor refused, as `(method, text)`: while [`Compositor::refuse_duplicate_claims`] is set, a
    /// claim of a chord, a gesture or a device that another live claim on the same seat holds.
    pub refused_claims: Vec<(String, String)>,
    /// Whether a claim another live claim on the same seat already holds is refused, as the real compositor refuses it.
    pub refuse_duplicate_claims: bool,
    /// The pointer chord drags in flight, by claim id.
    pub drags: BTreeMap<i64, Drag>,
    /// How many of the transactions the shell sent [`FakeCompositor::take_ops`] has handed over already.
    pub reported: usize,
    /// While set, a transaction is recorded and held until [`FakeCompositor::release`], as a real compositor echoes
    /// them later.
    pub hold: bool,
    /// The transactions held back.
    pub held: Vec<TreeTx>,
    /// Whether the fake refuses as the real compositor's revision check does (`gnoblin_render::tree_revision`): a write
    /// to a node that is gone is `unknown-node`, and a write to a node that a published batch changed after the
    /// transaction's base is `stale-base`. A fault queued with [`FakeCompositor::fault`] comes first.
    pub strict: bool,
    /// The serial at which each node last changed in a batch the compositor published, for [`Compositor::strict`].
    pub changed: BTreeMap<u64, u64>,
}

#[derive(Clone)]
pub struct FakeCompositor(Rc<RefCell<Compositor>>);

pub fn change(kind: MirrorChangeKind, node: u64) -> MirrorChange {
    MirrorChange {
        kind,
        node,
        class: String::new(),
        parent: 0,
        index: 0,
        key: String::new(),
        value: None,
        removed: Vec::new(),
    }
}

pub fn set_change(node: u64, key: &str, value: MirrorValue) -> MirrorChange {
    MirrorChange {
        key: key.to_owned(),
        value: Some(value),
        ..change(MirrorChangeKind::Set, node)
    }
}

pub fn text(value: &str) -> MirrorValue {
    MirrorValue::Text(value.to_owned())
}

/// The `pointer_chord` signal of `drag` in `phase` with the pointer at `at`.
fn drag_message(
    claim: i64,
    drag: &Drag,
    phase: &str,
    at: (f64, f64),
) -> gnoblin_tree_client::tree_mirror::TreePointerChord {
    gnoblin_tree_client::tree_mirror::TreePointerChord {
        claim,
        phase: phase.to_owned(),
        x: at.0,
        y: at.1,
        dx: at.0 - drag.origin.0,
        dy: at.1 - drag.origin.1,
        button: drag.button,
        modifiers: drag.modifiers.clone(),
        time: 0,
    }
}

/// Where the seat's pointer is, as the fake holds it.
fn pointer_at(c: &Compositor, seat: u64) -> (f64, f64) {
    let props = &c.nodes[&seat].props;
    let axis = |key: &str| match props.get(key) {
        Some(MirrorValue::Double(value)) => *value,
        _ => 0.0,
    };
    (axis("pointer_x"), axis("pointer_y"))
}

impl Default for Compositor {
    fn default() -> Self {
        Self::new()
    }
}

impl Compositor {
    pub fn new() -> Self {
        let root = Node {
            class: "node".to_owned(),
            parent: 0,
            children: Vec::new(),
            props: BTreeMap::new(),
        };
        Self {
            nodes: BTreeMap::from([(ROOT, root)]),
            serial: 1,
            claims: Vec::new(),
            sent: Vec::new(),
            measures: Vec::new(),
            queue: Vec::new(),
            faults: VecDeque::new(),
            refused: 0,
            stall: 0,
            dead: false,
            drop_batches: 0,
            refuse_every: std::env::var("GNOBLIN_FAKE_REFUSE_EVERY")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
            landed: 0,
            claim_calls: Vec::new(),
            refused_claims: Vec::new(),
            refuse_duplicate_claims: false,
            drags: BTreeMap::new(),
            reported: 0,
            hold: false,
            held: Vec::new(),
            strict: false,
            changed: BTreeMap::new(),
        }
    }

    /// `node` and everything beneath it.
    pub fn subtree(&self, node: u64) -> Vec<u64> {
        let mut out = vec![node];
        let mut at = 0;
        while at < out.len() {
            if let Some(entry) = self.nodes.get(&out[at]) {
                out.extend(entry.children.iter().copied());
            }
            at += 1;
        }
        out
    }

    pub fn detach(&mut self, node: u64) {
        let parent = self.nodes.get(&node).map_or(0, |entry| entry.parent);
        if let Some(entry) = self.nodes.get_mut(&parent) {
            entry.children.retain(|child| *child != node);
        }
    }

    /// Applies one op as the compositor does and returns the change it publishes.
    pub fn apply(&mut self, op: &TxOp) -> Option<MirrorChange> {
        match op {
            TxOp::New { id, class } => {
                self.nodes.insert(
                    *id,
                    Node {
                        class: class.clone(),
                        parent: 0,
                        children: Vec::new(),
                        props: BTreeMap::new(),
                    },
                );
                Some(MirrorChange {
                    class: class.clone(),
                    ..change(MirrorChangeKind::Created, *id)
                })
            }
            TxOp::Insert { node, parent, index } => {
                self.detach(*node);
                let held = self.nodes.get_mut(parent)?;
                let at = usize::try_from(*index).map_or(held.children.len(), |i| i.min(held.children.len()));
                held.children.insert(at, *node);
                self.nodes.get_mut(node)?.parent = *parent;
                Some(MirrorChange {
                    parent: *parent,
                    index: at as i64,
                    ..change(MirrorChangeKind::Parented, *node)
                })
            }
            TxOp::Set { node, key, value, .. } => {
                self.nodes.get_mut(node)?.props.insert(key.clone(), value.clone());
                Some(set_change(*node, key, value.clone()))
            }
            TxOp::Destroy { node } => {
                let removed = self.subtree(*node);
                self.detach(*node);
                for id in &removed {
                    self.nodes.remove(id);
                }
                Some(MirrorChange {
                    removed,
                    ..change(MirrorChangeKind::Destroyed, *node)
                })
            }
            TxOp::Call {
                node,
                method,
                args,
                token,
                text,
            } => {
                let id_bits = args.get(0..8).and_then(|bits| <[u8; 8]>::try_from(bits).ok());
                if matches!(
                    method.as_str(),
                    "claim" | "claim_gesture" | "claim_device" | "claim_pointer" | "grab"
                ) && let Some(bits) = id_bits
                {
                    let id = i64::from_le_bytes(bits);
                    let text = text.expose().to_owned();
                    let second = args
                        .get(8..16)
                        .and_then(|bits| <[u8; 8]>::try_from(bits).ok())
                        .map_or(0, i64::from_le_bytes);
                    self.claim_calls.push((method.clone(), id, second, text.clone()));
                    // A gesture is held by its kind and its finger count: three and four fingers are two claims.
                    let held = self.refuse_duplicate_claims
                        && self.claims.iter().any(|other| {
                            other.id != id
                                && other.method == *method
                                && other.text == text
                                && other.seat == *node
                                && (method != "claim_gesture" || other.second == second)
                        });
                    if held {
                        self.refused_claims.push((method.clone(), text));
                    } else {
                        self.claims.push(Claim {
                            id,
                            method: method.clone(),
                            text,
                            second,
                            seat: *node,
                        });
                    }
                }
                if method == "unclaim"
                    && let Some(bits) = id_bits
                {
                    let id = i64::from_le_bytes(bits);
                    self.claims.retain(|claim| claim.id != id);
                    // A drag whose claim goes ends without a word to the owner.
                    self.drags.remove(&id);
                    self.claim_calls.push((method.clone(), id, 0, String::new()));
                }
                if method == "measure" {
                    let bits: [u8; 8] = args.as_slice().try_into().ok()?;
                    self.measures.push(Measure {
                        token: *token,
                        node: *node,
                        size: f64::from_le_bytes(bits),
                        text: text.expose().to_owned(),
                    });
                }
                None
            }
        }
    }
}

impl Default for FakeCompositor {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeCompositor {
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(Compositor::new())))
    }

    /// Queues `fault` for the next transaction that has none yet. See [`Fault`].
    pub fn fault(&self, fault: Fault) {
        self.0.borrow_mut().faults.push_back(fault);
    }

    /// Queues `count` stale-base refusals at `failed_op`, each after a racing batch that writes `node`'s `key`, as the
    /// compositor's own writes to a window do.
    pub fn stale(&self, count: usize, failed_op: Option<i64>, node: u64, key: &str) {
        for round in 0..count {
            self.fault(Fault::StaleBase {
                failed_op,
                racing: vec![set_change(node, key, text(&format!("race{round}")))],
            });
        }
    }

    /// How many transactions the fake refused.
    pub fn refused(&self) -> usize {
        self.0.borrow().refused
    }

    /// The link dies now: every later call on it fails, as one to a compositor that exited does.
    pub fn disconnect(&self) {
        self.0.borrow_mut().dead = true;
    }

    /// A new connection after the old one died. What the compositor held for the dead one is gone, as it is when a
    /// connection ends: the claims it made, and what it had not said yet.
    pub fn reconnect(&self) {
        let mut c = self.0.borrow_mut();
        c.dead = false;
        c.stall = 0;
        c.claims.clear();
        c.queue.clear();
    }

    /// The next `count` batches of changes the compositor publishes are lost on the way to the shell, so the batch
    /// after them skips serials.
    pub fn drop_batches(&self, count: usize) {
        self.0.borrow_mut().drop_batches = count;
    }

    /// Refuses every `n`th transaction from now on, as [`Compositor::refuse_every`] says; `0` stops.
    pub fn refuse_every(&self, n: usize) {
        let mut c = self.0.borrow_mut();
        c.refuse_every = n;
        c.landed = 0;
    }

    /// From now on a claim that another live claim on the same seat already holds is refused, as the real compositor
    /// refuses it. What was refused is in [`Compositor::refused_claims`].
    pub fn refuse_duplicate_claims(&self) {
        self.0.borrow_mut().refuse_duplicate_claims = true;
    }

    /// From now on the fake refuses as the real compositor's revision check does: a write to a node that is gone is
    /// `unknown-node`, and a write to a node that a published batch changed after the transaction's base is
    /// `stale-base`. A fault queued with [`Self::fault`] still comes first.
    pub fn strict(&self) {
        self.0.borrow_mut().strict = true;
    }

    /// From now on a transaction is recorded and held until [`Self::release`], as a real compositor echoes them later.
    pub fn hold(&self) {
        self.0.borrow_mut().hold = true;
    }

    /// Answers every held transaction, in order, as the compositor echoes them. The fake goes on holding.
    pub fn release(&self) {
        let held = std::mem::take(&mut self.0.borrow_mut().held);
        for tx in &held {
            let _ = self.respond(tx);
        }
    }

    /// What the shell sent since the last call, as its ops in order.
    pub fn take_ops(&self) -> Vec<TxOp> {
        let mut c = self.0.borrow_mut();
        let from = c.reported.min(c.sent.len());
        c.reported = c.sent.len();
        c.sent[from..].iter().flatten().cloned().collect()
    }

    /// Moves the pointer of `seat`: writes its `pointer_x` and `pointer_y` and publishes them. A pointer chord drag live
    /// on the seat streams an `update` with the new place.
    pub fn set_pointer(&self, seat: u64, x: f64, y: f64) {
        {
            let mut c = self.0.borrow_mut();
            let node = c.nodes.get_mut(&seat).expect("the seat exists");
            node.props.insert("pointer_x".to_owned(), MirrorValue::Double(x));
            node.props.insert("pointer_y".to_owned(), MirrorValue::Double(y));
        }
        self.publish(vec![
            set_change(seat, "pointer_x", MirrorValue::Double(x)),
            set_change(seat, "pointer_y", MirrorValue::Double(y)),
        ]);
        let mut c = self.0.borrow_mut();
        let updates: Vec<_> = c
            .drags
            .iter()
            .filter(|(_, drag)| drag.seat == seat)
            .map(|(claim, drag)| drag_message(*claim, drag, "update", (x, y)))
            .collect();
        c.queue.extend(updates.into_iter().map(TreeIncoming::PointerChord));
    }

    /// A pointer chord (one that ends in `ButtonN`) pressed or released. Its drag starts at the seat's pointer on `press`
    /// and ends there on `release`, as the compositor's `pointer_chord` signal does.
    pub fn drag(&self, claim: i64, chord: &str, state: &str) {
        let (modifiers, button) = chord.rsplit_once("Button").expect("a pointer chord");
        let mut c = self.0.borrow_mut();
        let seat = c
            .claims
            .iter()
            .find(|held| held.id == claim)
            .expect("a pointer chord claim was made on a seat")
            .seat;
        let at = pointer_at(&c, seat);
        let message = if state == "press" {
            let drag = Drag {
                seat,
                origin: at,
                button: button.parse().expect("a button number"),
                modifiers: modifiers.to_owned(),
            };
            let message = drag_message(claim, &drag, "start", at);
            c.drags.insert(claim, drag);
            message
        } else {
            let drag = c.drags.remove(&claim).expect("a drag is live");
            drag_message(claim, &drag, "end", at)
        };
        c.queue.push(TreeIncoming::PointerChord(message));
    }

    /// The compositor ending a live drag with `cancel`.
    pub fn cancel_drag(&self, claim: i64) {
        let mut c = self.0.borrow_mut();
        let drag = c.drags.remove(&claim).expect("a drag is live");
        let message = drag_message(claim, &drag, "cancel", pointer_at(&c, drag.seat));
        c.queue.push(TreeIncoming::PointerChord(message));
    }

    /// What the link does with a transaction the shell sends: records it, holds it while [`Compositor::hold`] is set, and
    /// otherwise answers it (see [`Self::respond`]).
    ///
    /// # Errors
    ///
    /// The link is dead, or the transaction meets [`Fault::Disconnect`].
    pub fn land(&self, tx: &TreeTx) -> Result<(), LinkError> {
        {
            let mut c = self.0.borrow_mut();
            if c.dead {
                return Err(LinkError("the compositor closed the link".to_owned()));
            }
            c.sent.push(tx.ops.clone());
            c.landed += 1;
            if c.hold {
                c.held.push(tx.clone());
                return Ok(());
            }
        }
        self.respond(tx)
    }

    /// Answers a transaction that reached the compositor: applies it, refuses it or fails, by the next fault, or by the
    /// revision check when [`Compositor::strict`] is set.
    fn respond(&self, tx: &TreeTx) -> Result<(), LinkError> {
        let mut c = self.0.borrow_mut();
        // A password check is never refused by the periodic mode: the lock tests want the check to arrive, and its
        // text cannot be sent again.
        let periodic = c.refuse_every > 0
            && c.landed.is_multiple_of(c.refuse_every)
            && c.faults.is_empty()
            && !tx
                .ops
                .iter()
                .any(|op| matches!(op, TxOp::Call { method, .. } if method == "authenticate"));
        let fault = c.faults.pop_front().or_else(|| {
            periodic.then_some(Fault::StaleBase {
                failed_op: Some(-1),
                racing: Vec::new(),
            })
        });
        let mut stall = 0;
        match fault {
            Some(Fault::StaleBase { failed_op, racing }) => {
                let failed_op = failed_op.unwrap_or_else(|| {
                    let changed: Vec<u64> = racing.iter().map(|change| change.node).collect();
                    tx.ops
                        .iter()
                        .position(|op| match op {
                            TxOp::Set { node, .. } | TxOp::Destroy { node } | TxOp::Call { node, .. } => {
                                changed.contains(node)
                            }
                            TxOp::Insert { node, parent, .. } => changed.contains(node) || changed.contains(parent),
                            TxOp::New { .. } => false,
                        })
                        .map_or(-1, |at| at as i64)
                });
                // The racing batch is published before the refusal and the refusal carries the serial it leaves, as
                // the real frame answers after it publishes.
                for change in &racing {
                    if change.kind == MirrorChangeKind::Destroyed {
                        c.detach(change.node);
                        for id in &change.removed {
                            c.nodes.remove(id);
                        }
                    } else if let Some(value) = change.value.clone()
                        && let Some(entry) = c.nodes.get_mut(&change.node)
                    {
                        entry.props.insert(change.key.clone(), value);
                    }
                }
                // With no racing batch the compositor's serial does not move: the refusal names the serial the shell's
                // mirror holds already, as a base that is ahead of the compositor's gets.
                if !racing.is_empty() {
                    c.serial += 1;
                    let serial = c.serial;
                    Self::deliver(&mut c, serial, racing);
                }
                let serial = c.serial;
                c.refused += 1;
                c.queue.push(TreeIncoming::Reply(TxReply {
                    serial,
                    applied: false,
                    failed_op,
                    code: "stale-base".to_owned(),
                    removed: Vec::new(),
                }));
                return Ok(());
            }
            Some(Fault::UnknownNode { failed_op, destroyed }) => {
                let removed = c.subtree(destroyed);
                c.detach(destroyed);
                for id in &removed {
                    c.nodes.remove(id);
                }
                c.serial += 1;
                let serial = c.serial;
                Self::deliver(
                    &mut c,
                    serial,
                    vec![MirrorChange {
                        removed,
                        ..change(MirrorChangeKind::Destroyed, destroyed)
                    }],
                );
                c.refused += 1;
                c.queue.push(TreeIncoming::Reply(TxReply {
                    serial,
                    applied: false,
                    failed_op,
                    code: "unknown-node".to_owned(),
                    removed: Vec::new(),
                }));
                return Ok(());
            }
            Some(Fault::Refuse { code, failed_op }) => {
                c.refused += 1;
                let serial = c.serial;
                c.queue.push(TreeIncoming::Reply(TxReply {
                    serial,
                    applied: false,
                    failed_op,
                    code: code.to_owned(),
                    removed: Vec::new(),
                }));
                return Ok(());
            }
            Some(Fault::Disconnect) => {
                c.dead = true;
                return Err(LinkError("the compositor closed the link".to_owned()));
            }
            Some(Fault::Late { polls }) => stall = polls,
            None => {}
        }
        // The revision check of the real compositor, when the test asks for it.
        if c.strict {
            let gone = tx.ops.iter().position(|op| match op {
                TxOp::Set { node, .. } | TxOp::Destroy { node } | TxOp::Call { node, .. } => {
                    !c.nodes.contains_key(node)
                }
                _ => false,
            });
            let stale = tx.ops.iter().position(|op| match op {
                TxOp::Set { node, .. } | TxOp::Destroy { node } if tx.base_serial != c.serial => {
                    c.changed.get(node).is_some_and(|serial| *serial > tx.base_serial)
                }
                _ => false,
            });
            let refusal = gone
                .map(|at| ("unknown-node", at, c.serial))
                .or_else(|| stale.map(|at| ("stale-base", at, tx.serial)));
            if let Some((code, failed_op, serial)) = refusal {
                c.refused += 1;
                c.queue.push(TreeIncoming::Reply(TxReply {
                    serial,
                    applied: false,
                    failed_op: failed_op as i64,
                    code: code.to_owned(),
                    removed: Vec::new(),
                }));
                return Ok(());
            }
        }
        Self::apply_whole(&mut c, tx);
        c.queue.push(TreeIncoming::Reply(TxReply {
            serial: tx.serial,
            applied: true,
            failed_op: -1,
            code: String::new(),
            removed: Vec::new(),
        }));
        if stall > 0 {
            c.stall = stall;
        }
        Ok(())
    }

    /// Queues a batch of changes for the shell, unless a drop is pending: then the batch is lost, and the one after it
    /// skips its serial.
    fn deliver(c: &mut Compositor, serial: u64, changes: Vec<MirrorChange>) {
        for change in &changes {
            c.changed.insert(change.node, serial);
        }
        if c.drop_batches > 0 {
            c.drop_batches -= 1;
            return;
        }
        c.queue.push(TreeIncoming::Changes { serial, changes });
    }

    /// Applies every op of `tx`, advances the serial to it and queues the change batch, when there is one.
    fn apply_whole(c: &mut Compositor, tx: &TreeTx) {
        let mut changes = Vec::new();
        for op in &tx.ops {
            if let Some(applied) = c.apply(op) {
                changes.push(applied);
            }
        }
        c.serial = tx.serial;
        // The compositor sends no change message for a transaction that changed no node.
        if !changes.is_empty() {
            Self::deliver(c, tx.serial, changes);
        }
    }

    /// Publishes `changes` as a batch of the compositor's own.
    pub fn publish(&self, changes: Vec<MirrorChange>) {
        let mut c = self.0.borrow_mut();
        c.serial += 1;
        let serial = c.serial;
        Self::deliver(&mut c, serial, changes);
    }

    /// A window that is in the tree already, as at boot. It is server-decorated: the shell's titlebar frames it.
    pub fn add_window(&self, id: u64, title: &str, app_id: &str) {
        self.add_window_decorated(id, title, app_id, "server");
    }

    /// A window that is in the tree already, with the decoration the compositor negotiated (`client`, `server` or
    /// `none`).
    pub fn add_window_decorated(&self, id: u64, title: &str, app_id: &str, decoration: &str) {
        let mut c = self.0.borrow_mut();
        let props = BTreeMap::from([
            ("title".to_owned(), text(title)),
            ("app_id".to_owned(), text(app_id)),
            ("decoration".to_owned(), text(decoration)),
            ("width".to_owned(), MirrorValue::Double(400.0)),
            ("height".to_owned(), MirrorValue::Double(300.0)),
        ]);
        c.nodes.insert(
            id,
            Node {
                class: "window".to_owned(),
                parent: ROOT,
                children: Vec::new(),
                props,
            },
        );
        if let Some(root) = c.nodes.get_mut(&ROOT) {
            root.children.push(id);
        }
    }

    /// A window the compositor maps: created, parented under the root, given its title and app id. It is
    /// server-decorated, as a client that asked for the shell's frame is.
    pub fn create_window(&self, id: u64, title: &str, app_id: &str) {
        self.create_window_decorated(id, title, app_id, "server");
    }

    /// A window the compositor maps with the decoration it negotiated: `client` for one that draws its own title bar.
    pub fn create_window_decorated(&self, id: u64, title: &str, app_id: &str, decoration: &str) {
        self.add_window_decorated(id, title, app_id, decoration);
        self.publish(vec![
            MirrorChange {
                class: "window".to_owned(),
                ..change(MirrorChangeKind::Created, id)
            },
            MirrorChange {
                parent: ROOT,
                index: 99,
                ..change(MirrorChangeKind::Parented, id)
            },
            set_change(id, "title", text(title)),
            set_change(id, "app_id", text(app_id)),
            set_change(id, "decoration", text(decoration)),
            set_change(id, "width", MirrorValue::Double(400.0)),
            set_change(id, "height", MirrorValue::Double(300.0)),
        ]);
    }

    /// A window the compositor maps with a title and an app id and nothing else: no size and no decoration, as a test
    /// about where the shell puts a window of no size of its own wants it. It is created, parented under the root and
    /// given its two properties.
    pub fn create_bare_window(&self, id: u64, title: &str, app_id: &str) {
        self.add_node(
            id,
            ROOT,
            "window",
            vec![("title", text(title)), ("app_id", text(app_id))],
        );
        self.publish(vec![
            MirrorChange {
                class: "window".to_owned(),
                ..change(MirrorChangeKind::Created, id)
            },
            MirrorChange {
                parent: ROOT,
                index: 99,
                ..change(MirrorChangeKind::Parented, id)
            },
            set_change(id, "title", text(title)),
            set_change(id, "app_id", text(app_id)),
        ]);
    }

    /// A screen in the tree already, as at boot: named, placed and sized.
    pub fn add_screen(&self, id: u64, name: &str, (x, y, width, height): (f64, f64, f64, f64), primary: bool) {
        self.add_node(
            id,
            ROOT,
            "screen",
            vec![
                ("name", text(name)),
                ("x", MirrorValue::Double(x)),
                ("y", MirrorValue::Double(y)),
                ("width", MirrorValue::Double(width)),
                ("height", MirrorValue::Double(height)),
                ("primary", MirrorValue::Bool(primary)),
            ],
        );
    }

    /// A screen the compositor makes while the shell runs, as a hotplugged output: created, parented under the root,
    /// then given its name, place, size and primary flag.
    pub fn create_screen(&self, id: u64, name: &str, rect: (f64, f64, f64, f64), primary: bool) {
        let (x, y, width, height) = rect;
        self.add_screen(id, name, rect, primary);
        self.publish(vec![
            MirrorChange {
                class: "screen".to_owned(),
                ..change(MirrorChangeKind::Created, id)
            },
            MirrorChange {
                parent: ROOT,
                index: 99,
                ..change(MirrorChangeKind::Parented, id)
            },
            set_change(id, "name", text(name)),
            set_change(id, "x", MirrorValue::Double(x)),
            set_change(id, "y", MirrorValue::Double(y)),
            set_change(id, "width", MirrorValue::Double(width)),
            set_change(id, "height", MirrorValue::Double(height)),
            set_change(id, "primary", MirrorValue::Bool(primary)),
        ]);
    }

    /// The compositor writes a property of its own.
    pub fn external_set(&self, node: u64, key: &str, value: MirrorValue) {
        if let Some(entry) = self.0.borrow_mut().nodes.get_mut(&node) {
            entry.props.insert(key.to_owned(), value.clone());
        }
        self.publish(vec![set_change(node, key, value)]);
    }

    /// The compositor destroys a window, and its canvases with it.
    pub fn destroy(&self, node: u64) {
        let removed = {
            let mut c = self.0.borrow_mut();
            let removed = c.subtree(node);
            c.detach(node);
            for id in &removed {
                c.nodes.remove(id);
            }
            removed
        };
        self.publish(vec![MirrorChange {
            removed,
            ..change(MirrorChangeKind::Destroyed, node)
        }]);
    }

    /// A pointer event addressed to `node`.
    pub fn pointer(&self, node: u64, kind: &str, x: f64, y: f64, button: u64) {
        self.0.borrow_mut().queue.push(TreeIncoming::Event(TreeEvent {
            frame: 0,
            seq: 0,
            node,
            kind: kind.to_owned(),
            fields: EventFields {
                x,
                y,
                button,
                ..EventFields::default()
            },
        }));
    }

    pub fn answer(&self, token: u64, result: &str, error: &str) {
        self.0.borrow_mut().queue.push(TreeIncoming::Answer {
            token,
            result: result.as_bytes().to_vec(),
            error: error.to_owned(),
        });
    }

    /// The compositor's word that the timed write of `node`'s `property` ended, as `track_ended` carries it.
    pub fn track_ended(&self, node: u64, property: &str, completed: bool, tween: Option<u64>) {
        self.0.borrow_mut().queue.push(TreeIncoming::TrackEnded {
            node,
            property: property.to_owned(),
            completed,
            tween,
        });
    }

    /// Sends the shell `message` as it is, for a test about what the link may carry and not about any one of its kinds.
    pub fn push(&self, message: TreeIncoming) {
        self.0.borrow_mut().queue.push(message);
    }

    /// What the compositor has said that the shell has not read yet.
    pub fn queued(&self) -> Vec<TreeIncoming> {
        self.0.borrow().queue.clone()
    }

    pub fn measures(&self) -> Vec<Measure> {
        self.0.borrow().measures.clone()
    }

    pub fn sent_count(&self) -> usize {
        self.0.borrow().sent.len()
    }

    /// The transactions the shell sent after the first `mark`, each as its ops, refused ones included: `mark` is a
    /// `sent_count` taken before.
    pub fn sent_since(&self, mark: usize) -> Vec<Vec<TxOp>> {
        self.0.borrow().sent.iter().skip(mark).cloned().collect()
    }

    /// The token of every `authenticate` call the shell sent, refused attempts included, in order.
    pub fn checks(&self) -> Vec<u64> {
        self.0
            .borrow()
            .sent
            .iter()
            .flatten()
            .filter_map(|op| match op {
                TxOp::Call { method, token, .. } if method == "authenticate" => Some(*token),
                _ => None,
            })
            .collect()
    }

    /// Whether any property of any node holds `needle` as text, or inside paint bytes.
    pub fn holds(&self, needle: &str) -> bool {
        let c = self.0.borrow();
        c.nodes
            .values()
            .flat_map(|entry| entry.props.values())
            .any(|value| match value {
                MirrorValue::Text(held) => held.contains(needle),
                MirrorValue::Bytes(bytes) => bytes.windows(needle.len()).any(|window| window == needle.as_bytes()),
                _ => false,
            })
    }

    /// The ops of the transactions the shell sent after the first `mark`, in order: `mark` is a `sent_count` taken before.
    pub fn ops_since(&self, mark: usize) -> Vec<TxOp> {
        self.0.borrow().sent.iter().skip(mark).flatten().cloned().collect()
    }

    /// The `(node, method)` of every call but `measure` the shell sent, in order.
    pub fn calls(&self) -> Vec<(u64, String)> {
        self.0
            .borrow()
            .sent
            .iter()
            .flatten()
            .filter_map(|op| match op {
                TxOp::Call { node, method, .. } if method != "measure" => Some((*node, method.clone())),
                _ => None,
            })
            .collect()
    }

    /// The `apply` calls the shell sent, in order: the screen node, the request text and the token.
    pub fn applies(&self) -> Vec<(u64, String, u64)> {
        self.0
            .borrow()
            .sent
            .iter()
            .flatten()
            .filter_map(|op| match op {
                TxOp::Call {
                    node,
                    method,
                    text,
                    token,
                    ..
                } if method == "apply" => Some((*node, text.expose().to_owned(), *token)),
                _ => None,
            })
            .collect()
    }

    /// How many `Destroy` ops the shell sent for `node`.
    pub fn destroys_of(&self, node: u64) -> usize {
        self.0
            .borrow()
            .sent
            .iter()
            .flatten()
            .filter(|op| matches!(op, TxOp::Destroy { node: gone } if *gone == node))
            .count()
    }

    pub fn canvases(&self) -> Vec<u64> {
        let c = self.0.borrow();
        c.nodes
            .iter()
            .filter(|(_, node)| node.class == "canvas")
            .map(|(id, _)| *id)
            .collect()
    }

    pub fn children(&self, node: u64) -> Vec<u64> {
        self.0
            .borrow()
            .nodes
            .get(&node)
            .map_or_else(Vec::new, |entry| entry.children.clone())
    }

    pub fn number(&self, node: u64, key: &str) -> f64 {
        match self.0.borrow().nodes.get(&node).and_then(|entry| entry.props.get(key)) {
            Some(MirrorValue::Double(value)) => *value,
            _ => 0.0,
        }
    }

    /// `x,y,width,height` of a canvas, as whole numbers when they are.
    pub fn rect(&self, node: u64) -> String {
        format!(
            "{},{},{},{}",
            self.number(node, "x"),
            self.number(node, "y"),
            self.number(node, "width"),
            self.number(node, "height")
        )
    }

    pub fn content(&self, node: u64) -> Option<Vec<u8>> {
        match self
            .0
            .borrow()
            .nodes
            .get(&node)
            .and_then(|entry| entry.props.get("content"))
        {
            Some(MirrorValue::Bytes(bytes)) => Some(bytes.clone()),
            _ => None,
        }
    }

    pub fn flag(&self, node: u64, key: &str) -> Option<bool> {
        match self.0.borrow().nodes.get(&node).and_then(|entry| entry.props.get(key)) {
            Some(MirrorValue::Bool(value)) => Some(*value),
            _ => None,
        }
    }
}

impl TreeLink for FakeCompositor {
    fn attach(&mut self, _version: u64) -> Result<(), LinkError> {
        let mut c = self.0.borrow_mut();
        let attached = Attached {
            epoch: 1,
            root: ROOT,
            serial: c.serial,
        };
        let nodes = c
            .nodes
            .iter()
            .map(|(id, node)| SnapshotNode {
                id: *id,
                parent: node.parent,
                index: c
                    .nodes
                    .get(&node.parent)
                    .and_then(|parent| parent.children.iter().position(|child| child == id))
                    .map_or(0, |at| at as i64),
                class: node.class.clone(),
                title: String::new(),
                app_id: String::new(),
                props: node.props.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            })
            .collect();
        let snapshot = TreeIncoming::Snapshot {
            serial: c.serial,
            chunk: 0,
            last: true,
            nodes,
        };
        c.queue.push(TreeIncoming::Attached(attached));
        c.queue.push(snapshot);
        Ok(())
    }

    fn transact(&mut self, tx: &TreeTx) -> Result<(), LinkError> {
        self.land(tx)
    }

    fn poll(&mut self) -> Result<Vec<TreeIncoming>, LinkError> {
        let mut c = self.0.borrow_mut();
        if c.dead {
            return Err(LinkError("the compositor closed the link".to_owned()));
        }
        if c.stall > 0 {
            c.stall -= 1;
            return Ok(Vec::new());
        }
        Ok(std::mem::take(&mut c.queue))
    }
}

// ---- fixtures and the compositor's side of input ---------------------------------------------------------------------

impl FakeCompositor {
    /// The tree a shell boots over: the primary screen of `width` by `height` (node 20), one seat `seat0` (21) and the
    /// lock (22).
    pub fn add_desktop(&self, width: f64, height: f64) {
        self.add_node(
            20,
            ROOT,
            "screen",
            vec![
                ("width", MirrorValue::Double(width)),
                ("height", MirrorValue::Double(height)),
                ("primary", MirrorValue::Bool(true)),
            ],
        );
        self.add_node(21, ROOT, "seat", vec![("name", text("seat0"))]);
        self.add_node(22, ROOT, "lock", Vec::new());
    }

    /// A node of `class` under `parent`, in the tree already, as at boot.
    pub fn add_node(&self, id: u64, parent: u64, class: &str, props: Vec<(&str, MirrorValue)>) {
        let mut c = self.0.borrow_mut();
        c.nodes.insert(
            id,
            Node {
                class: class.to_owned(),
                parent,
                children: Vec::new(),
                props: props.into_iter().map(|(k, v)| (k.to_owned(), v)).collect(),
            },
        );
        if let Some(entry) = c.nodes.get_mut(&parent) {
            entry.children.push(id);
        }
    }

    /// A key sent to `node`, which holds the seat's focus.
    pub fn key(&self, node: u64, key: &str, text: &str, pressed: bool) {
        self.0.borrow_mut().queue.push(TreeIncoming::Event(TreeEvent {
            frame: 0,
            seq: 0,
            node,
            kind: "key".to_owned(),
            fields: EventFields {
                key: key.to_owned(),
                text: text.to_owned(),
                pressed,
                ..EventFields::default()
            },
        }));
    }

    /// The id of the live claim whose call text is `text` (a device and a line feed when scoped).
    pub fn claim_id(&self, text: &str) -> i64 {
        let c = self.0.borrow();
        c.claims
            .iter()
            .find(|claim| claim.text == text)
            .unwrap_or_else(|| panic!("no live claim for {text:?}; live: {:?}", c.claims))
            .id
    }

    /// The id of the live claim of `kind` (`swipe` or `pinch`) with `fingers` contacts.
    pub fn gesture_claim(&self, kind: &str, fingers: i64) -> i64 {
        let c = self.0.borrow();
        c.claims
            .iter()
            .find(|claim| claim.method == "claim_gesture" && claim.text == kind && claim.second == fingers)
            .unwrap_or_else(|| panic!("no live claim for {kind}{fingers}; live: {:?}", c.claims))
            .id
    }

    /// How many `set` ops the shell sent with a duration: what an animation is, as opposed to an instant write.
    pub fn timed_sets(&self) -> usize {
        self.0
            .borrow()
            .sent
            .iter()
            .flatten()
            .filter(|op| matches!(op, TxOp::Set { seconds: Some(_), .. }))
            .count()
    }

    /// A claimed chord changing `state` (`press`, `release` or `repeat`).
    pub fn chord(&self, claim: i64, chord: &str, state: &str) {
        self.0
            .borrow_mut()
            .queue
            .push(TreeIncoming::Chord(gnoblin_tree_client::tree_mirror::TreeChord {
                claim,
                chord: chord.to_owned(),
                state: state.to_owned(),
            }));
    }

    /// One phase of the drag of a claimed pointer chord (`start`, `update`, `end` or `cancel`): the pointer is at `at`
    /// and the press was at `from`. `chord` is the claim's text, such as `<Super>Button1`.
    pub fn pointer_chord(&self, claim: i64, chord: &str, phase: &str, from: (f64, f64), at: (f64, f64)) {
        let (modifiers, button) = chord.rsplit_once("Button").expect("a pointer chord");
        self.0.borrow_mut().queue.push(TreeIncoming::PointerChord(
            gnoblin_tree_client::tree_mirror::TreePointerChord {
                claim,
                phase: phase.to_owned(),
                x: at.0,
                y: at.1,
                dx: at.0 - from.0,
                dy: at.1 - from.1,
                button: button.parse().expect("a button number"),
                modifiers: modifiers.to_owned(),
                time: 0,
            },
        ));
    }

    /// A button of a claimed pointer changing `state` (`press` or `release`).
    pub fn pointer_button(&self, claim: i64, button: i64, state: &str) {
        self.0.borrow_mut().queue.push(TreeIncoming::PointerButton(
            gnoblin_tree_client::tree_mirror::TreePointerButton {
                claim,
                button,
                state: state.to_owned(),
                x: 0.0,
                y: 0.0,
                time: 0,
            },
        ));
    }

    /// The texts of the live claims, in the order they were made, a line feed shown as `|`.
    pub fn claimed(&self) -> Vec<String> {
        self.0
            .borrow()
            .claims
            .iter()
            .map(|claim| claim.text.replace('\n', "|"))
            .collect()
    }

    /// One phase of a touchpad gesture the shell claimed: `kind` is `swipe` or `pinch`.
    #[allow(clippy::too_many_arguments)]
    pub fn gesture(
        &self,
        claim: i64,
        kind: &str,
        fingers: i64,
        phase: &str,
        (dx, dy): (f64, f64),
        scale: f64,
        rotation: f64,
    ) {
        self.0
            .borrow_mut()
            .queue
            .push(TreeIncoming::Gesture(gnoblin_tree_client::tree_mirror::TreeGesture {
                claim,
                kind: kind.to_owned(),
                fingers,
                phase: phase.to_owned(),
                dx,
                dy,
                scale,
                rotation,
            }));
    }

    /// The compositor's request of a window (`menu`), anchored at `x`, `y`.
    pub fn request(&self, node: u64, name: &str, x: f64, y: f64) {
        self.0.borrow_mut().queue.push(TreeIncoming::Event(TreeEvent {
            frame: 0,
            seq: 0,
            node,
            kind: "request".to_owned(),
            fields: EventFields {
                x,
                y,
                request: name.to_owned(),
                ..EventFields::default()
            },
        }));
    }

    /// The value of a property of a node, as the compositor holds it.
    pub fn value(&self, node: u64, key: &str) -> Option<MirrorValue> {
        self.0
            .borrow()
            .nodes
            .get(&node)
            .and_then(|entry| entry.props.get(key).cloned())
    }

    /// What the fake holds, for a test that reads or changes it as the compositor would (the nodes, the queue of what
    /// the shell has yet to be told, the transactions it sent).
    pub fn borrow(&self) -> std::cell::Ref<'_, Compositor> {
        self.0.borrow()
    }

    /// The same, for a test that changes it.
    pub fn borrow_mut(&self) -> std::cell::RefMut<'_, Compositor> {
        self.0.borrow_mut()
    }

    /// Every `authenticate` call the shell sent, with its arguments and the secret text it carried, in order.
    pub fn authentications(&self) -> Vec<SentCall> {
        self.0
            .borrow()
            .sent
            .iter()
            .flatten()
            .filter_map(|op| match op {
                TxOp::Call {
                    node,
                    method,
                    args,
                    token,
                    text,
                } if method == "authenticate" => Some(SentCall {
                    node: *node,
                    method: method.clone(),
                    args: args.clone(),
                    token: *token,
                    text: text.expose().to_owned(),
                }),
                _ => None,
            })
            .collect()
    }
}

// ---- what the XR tests ask of the fake: nodes by class, the ops the shell sent, and nodes that come and go the way the XR
// program's do (the program is an observer: its seat and controllers are unparented) ----------------------------------

impl FakeCompositor {
    /// The ids of every node of `class`, in id order.
    pub fn ids_of_class(&self, class: &str) -> Vec<u64> {
        self.0
            .borrow()
            .nodes
            .iter()
            .filter(|(_, node)| node.class == class)
            .map(|(id, _)| *id)
            .collect()
    }

    /// The node named `name`, if there is one.
    pub fn named(&self, name: &str) -> Option<u64> {
        self.0
            .borrow()
            .nodes
            .iter()
            .find(|(_, node)| matches!(node.props.get("name"), Some(MirrorValue::Text(text)) if text == name))
            .map(|(id, _)| *id)
    }

    pub fn parent_of(&self, node: u64) -> Option<u64> {
        self.0.borrow().nodes.get(&node).map(|entry| entry.parent)
    }

    pub fn exists(&self, node: u64) -> bool {
        self.0.borrow().nodes.contains_key(&node)
    }

    /// Every transaction the shell has sent, oldest first.
    pub fn transactions(&self) -> Vec<Vec<TxOp>> {
        self.0.borrow().sent.clone()
    }

    /// Every op the shell has sent, oldest first.
    pub fn ops(&self) -> Vec<TxOp> {
        self.0.borrow().sent.iter().flatten().cloned().collect()
    }

    /// Forgets what the shell has sent, so the next assertion sees only what comes after.
    pub fn clear_sent(&self) {
        let mut c = self.0.borrow_mut();
        c.sent.clear();
        c.reported = 0;
    }

    /// How many `Set` ops of `node` the shell has sent, per key.
    pub fn sets_of(&self, node: u64) -> Vec<String> {
        self.ops()
            .into_iter()
            .filter_map(|op| match op {
                TxOp::Set { node: target, key, .. } if target == node => Some(key),
                _ => None,
            })
            .collect()
    }

    /// A node the XR program makes while the shell runs: created, optionally parented, with its properties.
    pub fn appear(&self, id: u64, parent: u64, class: &str, props: Vec<(&str, MirrorValue)>) {
        self.add_node(id, parent, class, props.clone());
        let mut changes: Vec<MirrorChange> = vec![MirrorChange {
            class: class.to_owned(),
            ..change(MirrorChangeKind::Created, id)
        }];
        if parent != 0 {
            changes.push(MirrorChange {
                parent,
                index: 99,
                ..change(MirrorChangeKind::Parented, id)
            });
        }
        changes.extend(props.into_iter().map(|(key, value)| set_change(id, key, value)));
        self.publish(changes);
    }

    /// Several properties of one node written in one batch, as the program writes a controller's frame.
    pub fn external_sets(&self, node: u64, writes: Vec<(&str, MirrorValue)>) {
        if let Some(entry) = self.0.borrow_mut().nodes.get_mut(&node) {
            for (key, value) in &writes {
                entry.props.insert((*key).to_owned(), value.clone());
            }
        }
        self.publish(
            writes
                .into_iter()
                .map(|(key, value)| set_change(node, key, value))
                .collect(),
        );
    }
}
