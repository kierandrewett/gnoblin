//! A shell's whole view of the tree, with no language in it: the link, the mirror, the per-tick transaction builder and the
//! gate that keeps one transaction in flight.
//!
//! [`TreeSession`] is what `gnoblin-core-impl` holds for a native shell (design decision D4: the mirror and the builder
//! live in the core, once, so no language runtime carries a mirror of its own that could drift). It does three things:
//!
//! - **Attach** ([`boot_mirror`]): ask for the tree, wait for the whole snapshot, build the mirror from it. The JavaScript shell
//!   boots through the same function.
//! - **Pump** ([`TreeSession::pump`]): read what the compositor sent, apply the change batches to the mirror, hand the
//!   gate its replies, and return what happened in the order it arrived: the node events, the answers, the input signals. A
//!   whole batch is applied before the caller sees any of its events, so a callback reads the tree as the batch left it.
//! - **Flush** ([`TreeSession::flush`]): send what the tick wrote as one transaction, once the gate allows one in flight, with the
//!   refusal and resend rules of [`crate::gate`].
//!
//! Reads go through the builder, so a write made in this tick is visible to a read in the same tick before the compositor
//! has seen it. Nothing here is thread-safe: a session belongs to the shell thread.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use gnoblin_secret::SecretText;

use crate::gate::{GateEffects, TxGate};
use crate::tree_mirror::{
    MirrorChangeKind, MirrorError, MirrorValue, NodeId, SnapshotNode, TreeIncoming, TreeLink, TreeMirror, TxAnimation,
    TxBuilder, TxError, wire_class,
};

/// The protocol version the shell attaches with.
pub const ATTACH_VERSION: u64 = 1;

/// How long the boot waits for the compositor's attach reply and its complete snapshot.
pub const BOOT_WAIT: Duration = Duration::from_secs(5);

/// The pause between boot polls while the reply is still on its way.
const BOOT_POLL: Duration = Duration::from_millis(5);

/// The epoch limit and the shift of a node id the shell makes: the epoch and a count, so no two nodes of one shell share
/// an id and none is the id of a node the compositor made.
const EPOCH_SHIFT: u32 = 32;
const EPOCH_LIMIT: u64 = 1 << 31;

/// Why a session could not boot or go on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// The link to the compositor failed or sent something the shell cannot take.
    Link(String),
    /// The mirror refused a snapshot or a change batch: the session cannot go on and must attach again.
    Mirror(String),
    /// The compositor did not answer the attach in time.
    NoAttach,
    /// The snapshot did not complete in time.
    IncompleteSnapshot,
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Link(why) => write!(f, "shell link: {why}"),
            Self::Mirror(why) => write!(f, "shell mirror: {why}"),
            Self::NoAttach => f.write_str("the compositor did not answer the attach"),
            Self::IncompleteSnapshot => f.write_str("the snapshot did not complete"),
        }
    }
}

impl std::error::Error for SessionError {}

/// Drains the link until the complete snapshot has arrived, building the mirror from it. Anything else that arrives meanwhile
/// is returned for the first pump. `attach` must have been sent already.
///
/// # Errors
///
/// The link failed, the compositor said nothing within `wait`, the snapshot did not complete, or the mirror refused it.
pub fn boot_mirror(link: &mut dyn TreeLink, wait: Duration) -> Result<(TreeMirror, Vec<TreeIncoming>), SessionError> {
    let mut attached = None;
    let mut nodes: Vec<SnapshotNode> = Vec::new();
    let mut snapshot_serial = 0;
    let mut rest = Vec::new();
    let mut complete = false;
    let deadline = Instant::now() + wait;
    loop {
        for message in link.poll().map_err(|e| SessionError::Link(e.0))? {
            match message {
                TreeIncoming::Attached(a) => attached = Some(a),
                TreeIncoming::Snapshot {
                    serial,
                    last,
                    nodes: chunk,
                    ..
                } => {
                    snapshot_serial = serial;
                    nodes.extend(chunk);
                    complete |= last;
                }
                other => rest.push(other),
            }
        }
        if (attached.is_some() && complete) || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(BOOT_POLL);
    }
    let attached = attached.ok_or(SessionError::NoAttach)?;
    if !complete {
        return Err(SessionError::IncompleteSnapshot);
    }
    let mirror = TreeMirror::from_snapshot(attached.epoch, attached.root, snapshot_serial, &nodes)
        .map_err(|e| SessionError::Mirror(format!("{e:?}")))?;
    Ok((mirror, rest))
}

/// One change the compositor published, with the facts a listener needs that the mirror no longer holds after the batch.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeEvent {
    /// A node came into the tree (`client_created` for a window). Its properties are those of the batch's end.
    Created {
        /// The new node.
        node: NodeId,
        /// Its schema class (`window`).
        class: String,
    },
    /// A node was placed under a parent.
    Parented {
        /// The node that moved.
        node: NodeId,
        /// Its class.
        class: String,
        /// Its new parent.
        parent: NodeId,
        /// Its place among the parent's children.
        index: i64,
    },
    /// A property of a node that existed before the batch changed. A node made in the same batch raises none: it is born with
    /// its final values.
    Changed {
        /// The node.
        node: NodeId,
        /// Its class.
        class: String,
        /// The property.
        key: String,
        /// The new value.
        value: MirrorValue,
    },
    /// A node and everything beneath it left the tree.
    Destroyed {
        /// The node that was destroyed.
        node: NodeId,
        /// Its class.
        class: String,
        /// The descendants that went with it, deepest first, with their classes.
        removed: Vec<(NodeId, String)>,
    },
}

/// What one pump found, in the order the compositor said it.
#[derive(Debug, Default)]
pub struct Pumped {
    /// The tree's changes, applied to the mirror already.
    pub changes: Vec<NodeEvent>,
    /// The rest of what the compositor sent, in arrival order: call answers, input events, chords, gestures, pointer drags
    /// and the end of timed writes. A call the gate gave up is here too, as an answer that failed.
    pub incoming: Vec<TreeIncoming>,
    /// What the gate reports: refused transactions and lost writes. Its failed calls are already in `incoming`.
    pub effects: GateEffects,
}

impl Pumped {
    /// Whether nothing arrived.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.incoming.is_empty() && self.effects.is_empty()
    }
}

/// A shell's connection to the tree: the link, the mirror, the builder of the tick and the gate.
pub struct TreeSession {
    link: Box<dyn TreeLink>,
    mirror: TreeMirror,
    tick: TxBuilder,
    gate: TxGate,
    /// What arrived with the snapshot, for the first pump.
    early: Vec<TreeIncoming>,
    /// Answers of calls the gate gave up while a flush ran, delivered by the next pump.
    carried: Vec<TreeIncoming>,
    /// Diagnostics of a flush, delivered by the next pump.
    carried_effects: GateEffects,
    /// The count of the last node id this shell made.
    counter: u64,
    /// The last call token handed out.
    tokens: u64,
}

impl std::fmt::Debug for TreeSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeSession")
            .field("serial", &self.mirror.serial())
            .field("nodes", &self.mirror.len())
            .field("settled", &self.gate.is_settled())
            .finish_non_exhaustive()
    }
}

impl TreeSession {
    /// Attaches over `link` and waits up to `wait` for the complete snapshot.
    ///
    /// # Errors
    ///
    /// The link refused the attach or failed, the compositor said nothing in time, or the snapshot was bad.
    pub fn attach(mut link: Box<dyn TreeLink>, wait: Duration) -> Result<Self, SessionError> {
        link.attach(ATTACH_VERSION).map_err(|e| SessionError::Link(e.0))?;
        let (mirror, early) = boot_mirror(link.as_mut(), wait)?;
        Ok(Self {
            link,
            mirror,
            tick: TxBuilder::new(),
            gate: TxGate::new(),
            early,
            carried: Vec::new(),
            carried_effects: GateEffects::default(),
            counter: 0,
            tokens: 0,
        })
    }

    /// The descriptor the link sleeps on, for a loop that waits on several connections.
    pub fn wait_fd(&self) -> Option<std::os::fd::BorrowedFd<'_>> {
        self.link.wait_fd()
    }

    /// The root node.
    pub fn root(&self) -> NodeId {
        self.mirror.root()
    }

    /// The serial of the last batch the mirror applied.
    pub fn serial(&self) -> u64 {
        self.mirror.serial()
    }

    /// The mirror the tick started from (what the compositor said), without this tick's writes.
    pub fn mirror(&self) -> &TreeMirror {
        &self.mirror
    }

    /// How many transactions have been sent.
    pub fn transactions_sent(&self) -> u64 {
        self.gate.transactions_sent()
    }

    /// Whether every transaction has been answered and none waits to go again.
    pub fn is_settled(&self) -> bool {
        self.gate.is_settled()
    }

    // ---- reads: the tree as this tick sees it --------------------------------------------------------------------

    /// Whether `node` is in the tree as this tick sees it.
    pub fn exists(&self, node: NodeId) -> bool {
        self.tick.node(&self.mirror, node).is_some()
    }

    /// The schema class of `node` (`window`, the name the catalog knows), as this tick sees it. The mirror spells a class the
    /// way the JavaScript shell does (`Window`); a native shell and the catalog use the schema's own lowercase name.
    pub fn class_of(&self, node: NodeId) -> Option<String> {
        self.tick
            .node(&self.mirror, node)
            .map(|record| wire_class(&record.class))
    }

    /// The parent of `node`, nil for the root and for a node not inserted yet.
    pub fn parent_of(&self, node: NodeId) -> Option<NodeId> {
        self.tick.node(&self.mirror, node).map(|record| record.parent)
    }

    /// The children of `node`, in order, as this tick sees them.
    pub fn children_of(&self, node: NodeId) -> Option<&[NodeId]> {
        self.tick.children(&self.mirror, node)
    }

    /// A property of `node` as this tick sees it. `None` when the node is gone or has never had the property set: the
    /// reader falls back to the schema default.
    pub fn get(&self, node: NodeId, key: &str) -> Option<&MirrorValue> {
        self.tick.get(&self.mirror, node, key)
    }

    // ---- writes: queued in this tick's builder -------------------------------------------------------------------

    /// The next id this shell may give a node it creates.
    fn mint(&mut self) -> Option<NodeId> {
        let epoch = self.mirror.epoch();
        if epoch >= EPOCH_LIMIT || self.counter + 1 >= 1 << EPOCH_SHIFT {
            return None;
        }
        self.counter += 1;
        Some((epoch << EPOCH_SHIFT) | self.counter)
    }

    /// Makes a node of `class` (the schema name, `canvas`) in this tick, with an id of this shell's.
    ///
    /// # Errors
    ///
    /// [`WriteError::OutOfIds`] when the shell has used every id of its epoch.
    pub fn create(&mut self, class: &str) -> Result<NodeId, WriteError> {
        let id = self.mint().ok_or(WriteError::OutOfIds)?;
        self.tick.new_node(&self.mirror, id, class).map_err(WriteError::Tx)?;
        Ok(id)
    }

    /// Sets a property. `animation` makes it a timed write.
    ///
    /// # Errors
    ///
    /// The node is not in the tree.
    pub fn set(
        &mut self,
        node: NodeId,
        key: &str,
        value: MirrorValue,
        animation: TxAnimation,
    ) -> Result<(), WriteError> {
        self.tick
            .set_animated(&self.mirror, node, key, value, animation)
            .map_err(WriteError::Tx)
    }

    /// Inserts `node` under `parent` at `index` (`-1` appends).
    ///
    /// # Errors
    ///
    /// Either node is not in the tree, `node` is the root, or the insert would make a node its own ancestor.
    pub fn insert(&mut self, node: NodeId, parent: NodeId, index: i64) -> Result<(), WriteError> {
        self.tick
            .insert(&self.mirror, node, parent, index)
            .map_err(WriteError::Tx)
    }

    /// Destroys `node` and its subtree.
    ///
    /// # Errors
    ///
    /// The node is not in the tree or is the root.
    pub fn destroy(&mut self, node: NodeId) -> Result<Vec<NodeId>, WriteError> {
        self.tick
            .destroy(&self.mirror, node)
            .map(|destroyed| destroyed.removed)
            .map_err(WriteError::Tx)
    }

    /// Calls `method` on `node`. `args` are the call's integers as the wire carries them and `text` its text (a chord, a
    /// password). Returns the token the answer will come under.
    ///
    /// # Errors
    ///
    /// The node is not in the tree.
    pub fn call(&mut self, node: NodeId, method: &str, args: Vec<u8>, text: SecretText) -> Result<u64, WriteError> {
        self.tokens += 1;
        let token = self.tokens;
        self.tick
            .call(&self.mirror, node, method, args, token, text)
            .map_err(WriteError::Tx)?;
        Ok(token)
    }

    // ---- the link ------------------------------------------------------------------------------------------------

    /// Reads what the compositor sent and applies it: the change batches go to the mirror (a whole batch before any event of
    /// it is returned), the replies go to the gate, and everything else is returned in arrival order.
    ///
    /// # Errors
    ///
    /// The link failed, or the mirror refused a batch (a batch was missed): the session cannot go on and must attach again.
    pub fn pump(&mut self) -> Result<Pumped, SessionError> {
        let mut pumped = Pumped {
            effects: std::mem::take(&mut self.carried_effects),
            ..Pumped::default()
        };
        pumped.incoming.append(&mut self.carried);
        let mut incoming = std::mem::take(&mut self.early);
        incoming.extend(self.link.poll().map_err(|e| SessionError::Link(e.0))?);
        let mut created: BTreeSet<NodeId> = BTreeSet::new();
        for message in incoming {
            match message {
                TreeIncoming::Changes { serial, changes } => {
                    // A destroyed node's class and its subtree's go before the batch is applied: afterwards the mirror
                    // has forgotten them.
                    let mut destroyed: Vec<(NodeId, String, Vec<(NodeId, String)>)> = Vec::new();
                    for change in changes.iter().filter(|c| c.kind == MirrorChangeKind::Destroyed) {
                        let class_of = |id: NodeId| self.mirror.node(id).map(|record| wire_class(&record.class));
                        let removed = change
                            .removed
                            .iter()
                            .copied()
                            .filter(|id| *id != change.node)
                            .rev()
                            .filter_map(|id| class_of(id).map(|class| (id, class)))
                            .collect();
                        destroyed.push((change.node, class_of(change.node).unwrap_or_default(), removed));
                    }
                    let applied = self
                        .mirror
                        .apply_changes(serial, &changes)
                        .map_err(|e: MirrorError| SessionError::Mirror(e.to_string()))?;
                    if applied == crate::tree_mirror::MirrorApplied::Ignored {
                        continue;
                    }
                    let mut gone = destroyed.into_iter();
                    for change in &changes {
                        match change.kind {
                            MirrorChangeKind::Created => {
                                created.insert(change.node);
                                pumped.changes.push(NodeEvent::Created {
                                    node: change.node,
                                    class: wire_class(&change.class),
                                });
                            }
                            MirrorChangeKind::Parented => {
                                if let Some(class) =
                                    self.mirror.node(change.node).map(|record| wire_class(&record.class))
                                {
                                    pumped.changes.push(NodeEvent::Parented {
                                        node: change.node,
                                        class,
                                        parent: change.parent,
                                        index: change.index,
                                    });
                                }
                            }
                            MirrorChangeKind::Set => {
                                let Some(value) = change.value.clone() else { continue };
                                // A node made in this pump is born with its final values; one the batch also destroyed
                                // has nothing to tell.
                                if created.contains(&change.node) {
                                    continue;
                                }
                                if let Some(class) =
                                    self.mirror.node(change.node).map(|record| wire_class(&record.class))
                                {
                                    pumped.changes.push(NodeEvent::Changed {
                                        node: change.node,
                                        class,
                                        key: change.key.clone(),
                                        value,
                                    });
                                }
                            }
                            MirrorChangeKind::Destroyed => {
                                if let Some((node, class, removed)) = gone.next() {
                                    created.remove(&node);
                                    for (id, _) in &removed {
                                        created.remove(id);
                                    }
                                    pumped.changes.push(NodeEvent::Destroyed { node, class, removed });
                                }
                            }
                        }
                    }
                }
                TreeIncoming::Reply(reply) => {
                    self.gate
                        .handle_reply(&mut self.mirror, &mut self.tick, &reply, &mut pumped.effects);
                }
                TreeIncoming::Attached(_) | TreeIncoming::Snapshot { .. } => {
                    return Err(SessionError::Link("attach or snapshot after boot".to_owned()));
                }
                other => pumped.incoming.push(other),
            }
        }
        for (token, error) in pumped.effects.failed_calls.drain(..) {
            pumped.incoming.push(TreeIncoming::Answer {
                token,
                result: Vec::new(),
                error,
            });
        }
        // The builder's copies of nodes must show what the compositor wrote natively before anything reads them: after the
        // batches, before the next read.
        if self.gate.is_settled() {
            self.tick.settle();
        }
        self.tick.refresh(&self.mirror);
        Ok(pumped)
    }

    /// Sends what the tick wrote as one transaction, if the gate allows one in flight. Returns whether one was sent.
    ///
    /// # Errors
    ///
    /// The link failed.
    pub fn flush(&mut self) -> Result<bool, SessionError> {
        let mut effects = GateEffects::default();
        let tx = self.gate.next_transaction(&self.mirror, &mut self.tick, &mut effects);
        for (token, error) in effects.failed_calls.drain(..) {
            self.carried.push(TreeIncoming::Answer {
                token,
                result: Vec::new(),
                error,
            });
        }
        self.carried_effects.diagnostics.append(&mut effects.diagnostics);
        self.carried_effects.lost_writes.append(&mut effects.lost_writes);
        let Some(tx) = tx else { return Ok(false) };
        self.link.transact(&tx).map_err(|e| SessionError::Link(e.0))?;
        Ok(true)
    }
}

/// Why a write was refused before it was sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteError {
    /// The builder's own check: a node that is not there, a cycle, the root.
    Tx(TxError),
    /// The shell has used every node id of its epoch.
    OutOfIds,
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tx(error) => error.fmt(f),
            Self::OutOfIds => f.write_str("the shell has no node ids left"),
        }
    }
}

impl std::error::Error for WriteError {}
