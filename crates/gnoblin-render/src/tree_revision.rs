//! The stale-base rule for tree transactions (design/node-core.md §4.5).
//!
//! A transaction built against an older serial is not refused for being old. It is refused
//! when one of its ops names a node that a batch published since that serial changed, or that
//! an earlier transaction in the same frame changed. Nodes a transaction creates never
//! conflict. The tree session and the frame drain both apply this rule at the moment they
//! apply a transaction, so they cannot disagree about it.

use std::collections::{BTreeSet, VecDeque};

use gnoblin_types::NodeId;

use crate::transaction::{Code, TreeChange, TreeOp, TreeRefusal, TreeTransaction};

/// How many published batches the revision history keeps. A transaction whose base is older
/// than the oldest kept batch cannot be checked, so it is refused `stale-base`.
pub const TREE_REVISION_HISTORY: usize = 64;

/// The nodes each recent published batch changed, so a stale transaction can be checked
/// against the batches published after its base.
#[derive(Debug, Default)]
pub struct TreeRevisions {
    /// Each recorded batch's serial and the nodes it changed, oldest first.
    batches: VecDeque<(u64, BTreeSet<NodeId>)>,
}

impl TreeRevisions {
    /// An empty history: no batch has been published.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the batch published at `serial`. Call it once for every serial advance, in
    /// order, so the history covers every serial after its oldest entry.
    pub fn record(&mut self, serial: u64, changes: &[TreeChange]) {
        let mut nodes = BTreeSet::new();
        for change in changes {
            changed_nodes(change, &mut nodes);
        }
        self.batches.push_back((serial, nodes));
        while self.batches.len() > TREE_REVISION_HISTORY {
            self.batches.pop_front();
        }
    }

    /// Checks `tx`, built against `base`, when the tree is at `current` and the batch being
    /// built has already applied `pending`. `Ok` when no op of `tx` names a node changed
    /// since `base`; otherwise `stale-base` naming the first conflicting op. A `base` ahead of
    /// `current`, or older than the retained history, is `stale-base` with `failed_op` -1.
    pub fn check(
        &self,
        base: u64,
        current: u64,
        pending: &[TreeChange],
        tx: &TreeTransaction,
    ) -> Result<(), TreeRefusal> {
        if base > current {
            return Err(stale(-1));
        }
        if base < current {
            let retained_from = self.batches.front().map_or(current + 1, |(serial, _)| *serial);
            if base + 1 < retained_from {
                return Err(stale(-1));
            }
        }

        let mut changed = BTreeSet::new();
        for (serial, nodes) in &self.batches {
            if *serial > base {
                changed.extend(nodes.iter().copied());
            }
        }
        for change in pending {
            changed_nodes(change, &mut changed);
        }
        if changed.is_empty() {
            return Ok(());
        }

        let created: BTreeSet<NodeId> = tx
            .ops
            .iter()
            .filter_map(|op| match op {
                TreeOp::New { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        for (index, op) in tx.ops.iter().enumerate() {
            if conflicts(op, &changed, &created) {
                return Err(TreeRefusal {
                    failed_op: i32::try_from(index).unwrap_or(i32::MAX),
                    code: Code::StaleBase,
                });
            }
        }
        Ok(())
    }
}

const fn stale(failed_op: i32) -> TreeRefusal {
    TreeRefusal {
        failed_op,
        code: Code::StaleBase,
    }
}

/// The nodes one change touched, added to `out`.
fn changed_nodes(change: &TreeChange, out: &mut BTreeSet<NodeId>) {
    match change {
        TreeChange::Created { node, .. } | TreeChange::Parented { node, .. } | TreeChange::Set { node, .. } => {
            out.insert(*node);
        }
        TreeChange::Destroyed { node, removed } => {
            out.insert(*node);
            out.extend(removed.iter().copied());
        }
    }
}

/// Whether `op` names a node in `changed` that the transaction did not create itself.
fn conflicts(op: &TreeOp, changed: &BTreeSet<NodeId>, created: &BTreeSet<NodeId>) -> bool {
    let hit = |node: &NodeId| !created.contains(node) && changed.contains(node);
    match op {
        TreeOp::Set { node, .. } | TreeOp::Destroy { node } | TreeOp::TreeCall { node, .. } => hit(node),
        TreeOp::Insert { node, parent, .. } => hit(node) || hit(parent),
        TreeOp::New { .. } | TreeOp::Input { .. } => false,
    }
}
