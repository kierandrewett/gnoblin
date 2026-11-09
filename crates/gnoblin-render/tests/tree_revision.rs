//! The stale-base rule (design/node-core.md §4.5): a transaction built against an older
//! serial is refused only when it names a node that changed since its base.

use gnoblin_render::transaction::{Code, TreeChange, TreeOp, TreeRefusal, TreeTransaction, TreeValue};
use gnoblin_render::tree_revision::{TREE_REVISION_HISTORY, TreeRevisions};
use gnoblin_types::NodeId;

const ROOT: NodeId = NodeId::new(1);
const A: NodeId = NodeId::new(0x100);
const B: NodeId = NodeId::new(0x101);
const C: NodeId = NodeId::new(0x102);

fn set(node: NodeId) -> TreeOp {
    TreeOp::Set {
        node,
        key: "opacity".to_owned(),
        value: TreeValue::Double(0.5),
        timing: None,
    }
}

fn set_change(node: NodeId) -> TreeChange {
    TreeChange::Set {
        node,
        key: "opacity".to_owned(),
        value: TreeValue::Double(0.5),
    }
}

fn stale_at(failed_op: i32) -> TreeRefusal {
    TreeRefusal {
        failed_op,
        code: Code::StaleBase,
    }
}

/// Revisions for batches published at serials 1, 2, … in the order given.
fn revisions(batches: &[Vec<TreeChange>]) -> TreeRevisions {
    let mut revisions = TreeRevisions::new();
    for (index, changes) in batches.iter().enumerate() {
        revisions.record(index as u64 + 1, changes);
    }
    revisions
}

#[test]
fn a_fresh_base_applies() {
    let revisions = revisions(&[]);
    let tx = TreeTransaction::new(vec![set(A)]);
    assert_eq!(revisions.check(0, 0, &[], &tx), Ok(()));
}

#[test]
fn a_stale_write_to_a_node_changed_since_base_is_refused_at_the_first_conflicting_op() {
    let revisions = revisions(&[vec![set_change(A)]]);
    let tx = TreeTransaction::new(vec![set(B), set(A), set(A)]);
    assert_eq!(revisions.check(0, 1, &[], &tx), Err(stale_at(1)));
}

#[test]
fn a_stale_write_to_untouched_nodes_applies() {
    let revisions = revisions(&[vec![set_change(A)]]);
    let tx = TreeTransaction::new(vec![set(B)]);
    assert_eq!(revisions.check(0, 1, &[], &tx), Ok(()));
}

#[test]
fn a_stale_write_to_a_node_destroyed_since_base_is_refused() {
    let revisions = revisions(&[vec![TreeChange::Destroyed {
        node: A,
        removed: vec![A, B],
    }]]);
    let tx = TreeTransaction::new(vec![set(B)]);
    assert_eq!(revisions.check(0, 1, &[], &tx), Err(stale_at(0)));
}

#[test]
fn a_stale_move_of_a_node_moved_since_base_is_refused() {
    let revisions = revisions(&[vec![TreeChange::Parented {
        node: C,
        parent: ROOT,
        index: 0,
    }]]);
    let tx = TreeTransaction::new(vec![TreeOp::Insert {
        node: C,
        parent: ROOT,
        index: None,
    }]);
    assert_eq!(revisions.check(0, 1, &[], &tx), Err(stale_at(0)));
}

#[test]
fn creating_a_node_never_conflicts_on_its_own_id() {
    let revisions = revisions(&[vec![TreeChange::Destroyed {
        node: A,
        removed: vec![A],
    }]]);
    let tx = TreeTransaction::new(vec![
        TreeOp::New {
            class: "node".to_owned(),
            id: A,
        },
        TreeOp::Insert {
            node: A,
            parent: ROOT,
            index: None,
        },
    ]);
    assert_eq!(revisions.check(0, 1, &[], &tx), Ok(()));
}

#[test]
fn the_window_spans_every_batch_after_base() {
    let revisions = revisions(&[vec![set_change(B)], vec![set_change(A)]]);
    let tx = TreeTransaction::new(vec![set(A)]);
    assert_eq!(revisions.check(0, 2, &[], &tx), Err(stale_at(0)));
    assert_eq!(revisions.check(1, 2, &[], &tx), Err(stale_at(0)));
    assert_eq!(revisions.check(2, 2, &[], &tx), Ok(()));
}

#[test]
fn changes_applied_earlier_in_the_same_frame_are_in_the_window() {
    let revisions = revisions(&[]);
    let pending = [set_change(A)];
    assert_eq!(
        revisions.check(0, 0, &pending, &TreeTransaction::new(vec![set(A)])),
        Err(stale_at(0))
    );
    assert_eq!(
        revisions.check(0, 0, &pending, &TreeTransaction::new(vec![set(B)])),
        Ok(())
    );
}

#[test]
fn a_base_older_than_the_retained_history_is_refused() {
    let batches: Vec<Vec<TreeChange>> = (0..=TREE_REVISION_HISTORY).map(|_| Vec::new()).collect();
    let revisions = revisions(&batches);
    let current = TREE_REVISION_HISTORY as u64 + 1;
    let tx = TreeTransaction::new(vec![set(A)]);
    assert_eq!(revisions.check(0, current, &[], &tx), Err(stale_at(-1)));
    assert_eq!(revisions.check(1, current, &[], &tx), Ok(()));
}

#[test]
fn a_base_ahead_of_current_is_refused_as_stale() {
    let revisions = revisions(&[]);
    let tx = TreeTransaction::new(vec![set(A)]);
    assert_eq!(revisions.check(5, 0, &[], &tx), Err(stale_at(-1)));
}
