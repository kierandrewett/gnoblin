//! The shell's tree mirror and its transaction builder, with no Lua involved
//! (design/node-core.md, sections 4.1 and 4.6).

use gnoblin_lua::tree_mirror::{
    Attached, LuaApplied, LuaChange, LuaMirror, LuaOp, LuaTxBuilder, LuaValue, MirrorChangeKind, MirrorError, NIL,
    NodeId, SnapshotBuilder, SnapshotNode, TxError,
};

fn snap(id: NodeId, parent: NodeId, index: i64, class: &str) -> SnapshotNode {
    SnapshotNode {
        id,
        parent,
        index,
        class: class.to_owned(),
        title: String::new(),
        app_id: String::new(),
        props: Vec::new(),
    }
}

fn change(kind: MirrorChangeKind, node: NodeId) -> LuaChange {
    LuaChange {
        kind,
        node,
        class: String::new(),
        parent: NIL,
        index: 0,
        key: String::new(),
        value: None,
        removed: Vec::new(),
    }
}

/// root(1) -> [a(2), b(3)]; a -> [c(4)].
fn base_mirror() -> LuaMirror {
    let nodes = [
        snap(1, NIL, 0, "node"),
        snap(2, 1, 0, "node"),
        snap(3, 1, 1, "node"),
        snap(4, 2, 0, "node"),
    ];
    LuaMirror::from_snapshot(7, 1, 5, &nodes).expect("snapshot builds")
}

fn children(mirror: &LuaMirror, id: NodeId) -> Vec<NodeId> {
    mirror.node(id).expect("node present").children.clone()
}

#[test]
fn snapshot_chunks_rebuild_the_mirror_in_index_order() {
    let mut builder = SnapshotBuilder::new(Attached {
        epoch: 7,
        root: 1,
        serial: 5,
    });
    // Children arrive before their parent's chunk and out of index order.
    assert!(
        builder
            .chunk(5, 0, false, vec![snap(3, 1, 1, "node")])
            .expect("chunk 0")
            .is_none()
    );
    assert!(
        builder
            .chunk(5, 1, false, vec![snap(4, 2, 0, "node")])
            .expect("chunk 1")
            .is_none()
    );
    let mirror = builder
        .chunk(5, 2, true, vec![snap(1, NIL, 0, "node"), snap(2, 1, 0, "node")])
        .expect("chunk 2")
        .expect("last chunk finishes the snapshot");

    assert_eq!(mirror.epoch(), 7);
    assert_eq!(mirror.root(), 1);
    assert_eq!(mirror.serial(), 5);
    assert_eq!(children(&mirror, 1), vec![2, 3]);
    assert_eq!(children(&mirror, 2), vec![4]);
    assert_eq!(mirror.len(), 4);
}

#[test]
fn snapshot_chunks_out_of_order_are_refused() {
    let mut builder = SnapshotBuilder::new(Attached {
        epoch: 7,
        root: 1,
        serial: 5,
    });
    let err = builder
        .chunk(5, 1, true, vec![snap(1, NIL, 0, "node")])
        .expect_err("chunk 1 before chunk 0");
    assert!(matches!(err, MirrorError::Snapshot(_)), "{err:?}");
}

#[test]
fn snapshot_with_a_missing_parent_is_refused() {
    let err = LuaMirror::from_snapshot(7, 1, 5, &[snap(1, NIL, 0, "node"), snap(9, 42, 0, "node")])
        .expect_err("node 9 names a parent that is not in the snapshot");
    assert!(matches!(err, MirrorError::Snapshot(_)), "{err:?}");
}

#[test]
fn a_change_batch_updates_the_mirror() {
    let mut mirror = base_mirror();
    let mut created = change(MirrorChangeKind::Created, 10);
    created.class = "canvas".to_owned();
    let mut parented = change(MirrorChangeKind::Parented, 10);
    parented.parent = 3;
    parented.index = 0;
    let mut set = change(MirrorChangeKind::Set, 10);
    set.key = "opacity".to_owned();
    set.value = Some(LuaValue::Double(0.5));

    let applied = mirror
        .apply_changes(6, &[created, parented, set])
        .expect("batch applies");

    assert_eq!(applied, LuaApplied::LuaApplied);
    assert_eq!(mirror.serial(), 6);
    assert_eq!(children(&mirror, 3), vec![10]);
    assert_eq!(mirror.node(10).expect("created").class, "Canvas");
    assert_eq!(mirror.value(10, "opacity"), Some(&LuaValue::Double(0.5)));
}

#[test]
fn a_destroy_change_removes_the_subtree_and_its_parent_link() {
    let mut mirror = base_mirror();
    let mut destroyed = change(MirrorChangeKind::Destroyed, 2);
    destroyed.removed = vec![4];

    mirror.apply_changes(6, &[destroyed]).expect("batch applies");

    assert!(mirror.node(2).is_none());
    assert!(mirror.node(4).is_none());
    assert_eq!(children(&mirror, 1), vec![3]);
}

#[test]
fn a_destroy_whose_removed_list_names_the_root_removes_each_node_once() {
    // The scene's `removed` is the whole subtree, the root included (`Scene::remove_as`), which is
    // what the compositor sends. The root must not be counted twice.
    let mut mirror = base_mirror();
    let mut destroyed = change(MirrorChangeKind::Destroyed, 2);
    destroyed.removed = vec![2, 4];

    mirror
        .apply_changes(6, &[destroyed])
        .expect("a whole-subtree destroy applies");

    assert!(mirror.node(2).is_none());
    assert!(mirror.node(4).is_none());
    assert_eq!(children(&mirror, 1), vec![3]);
}

#[test]
fn a_stale_batch_is_ignored() {
    // F24: a batch at or before the mirror's serial is stale and changes nothing.
    let mut mirror = base_mirror();
    let before = mirror.dump();
    let mut set = change(MirrorChangeKind::Set, 2);
    set.key = "opacity".to_owned();
    set.value = Some(LuaValue::Double(0.0));

    assert_eq!(
        mirror.apply_changes(5, &[set]).expect("stale batch"),
        LuaApplied::Ignored
    );
    assert_eq!(mirror.dump(), before);
}

#[test]
fn a_missed_batch_is_a_gap_that_needs_a_reattach() {
    // F23: a batch past the next serial means one was missed.
    let mut mirror = base_mirror();
    let before = mirror.dump();
    let err = mirror.apply_changes(7, &[]).expect_err("serial 6 is missing");
    assert_eq!(err, MirrorError::Gap { expected: 6, got: 7 });
    assert_eq!(mirror.dump(), before);
}

#[test]
fn a_refused_transaction_leaves_the_mirror_unchanged() {
    // The shell builds the ops and the compositor refuses them: the mirror only changes
    // when a change batch arrives, so dropping the builder leaves it as it was.
    let mirror = base_mirror();
    let before = mirror.dump();
    {
        let mut tx = LuaTxBuilder::new();
        tx.set(&mirror, 2, "opacity", LuaValue::Double(0.25), None, None)
            .expect("node 2 exists");
        tx.destroy(&mirror, 3).expect("node 3 exists");
        let _ = tx.into_transaction(&mirror, 6);
    }
    assert_eq!(mirror.dump(), before);
    assert_eq!(mirror.serial(), 5);
}

#[test]
fn a_transactions_own_writes_are_visible_to_its_reads() {
    // F23/F24 of node-core.md: reads inside a tick see the writes of that tick.
    let mirror = base_mirror();
    let mut tx = LuaTxBuilder::new();

    tx.new_node(&mirror, 20, "Canvas").expect("fresh id");
    tx.insert(&mirror, 20, 3, 0).expect("insert under b");
    tx.set(&mirror, 20, "opacity", LuaValue::Double(0.75), None, None)
        .expect("set on new node");

    assert_eq!(tx.node(&mirror, 20).expect("new node visible").class, "Canvas");
    assert_eq!(tx.node(&mirror, 20).expect("visible").parent, 3);
    assert_eq!(tx.node(&mirror, 3).expect("b visible").children, vec![20]);
    assert_eq!(tx.get(&mirror, 20, "opacity"), Some(&LuaValue::Double(0.75)));
    // The base is untouched by the overlay.
    assert!(mirror.node(20).is_none());
    assert_eq!(children(&mirror, 3), Vec::<NodeId>::new());
}

#[test]
fn destroy_returns_the_subtree_and_hides_it_from_later_reads() {
    let mirror = base_mirror();
    let mut tx = LuaTxBuilder::new();

    let mut removed = tx.destroy(&mirror, 2).expect("node 2 exists");
    removed.sort_unstable();

    assert_eq!(removed, vec![4]);
    assert!(tx.node(&mirror, 2).is_none());
    assert!(tx.node(&mirror, 4).is_none());
    assert_eq!(tx.node(&mirror, 1).expect("root").children, vec![3]);
    assert_eq!(
        tx.call(&mirror, 4, "measure", Vec::new(), gnoblin_secret::SecretText::default()),
        Err(TxError::UnknownNode(4))
    );
}

#[test]
fn a_write_to_a_node_the_tree_does_not_have_is_refused_locally() {
    let mirror = base_mirror();
    let mut tx = LuaTxBuilder::new();
    assert_eq!(
        tx.set(&mirror, 99, "x", LuaValue::Double(1.0), None, None),
        Err(TxError::UnknownNode(99))
    );
    assert!(tx.ops().is_empty());
}

#[test]
fn inserting_a_node_under_its_own_descendant_is_refused() {
    let mirror = base_mirror();
    let mut tx = LuaTxBuilder::new();
    assert_eq!(tx.insert(&mirror, 2, 4, 0), Err(TxError::Cycle(2)));
}

#[test]
fn the_root_cannot_be_destroyed() {
    let mirror = base_mirror();
    let mut tx = LuaTxBuilder::new();
    assert_eq!(tx.destroy(&mirror, 1), Err(TxError::Root(1)));
}

#[test]
fn the_transaction_carries_the_ops_and_the_serials() {
    let mirror = base_mirror();
    let mut tx = LuaTxBuilder::new();
    tx.new_node(&mirror, 20, "Node").expect("fresh id");
    tx.set(
        &mirror,
        20,
        "name",
        LuaValue::Text("a".to_owned()),
        Some(0.2),
        Some(vec![0.0, 1.0]),
    )
    .expect("set");
    let transaction = tx.into_transaction(&mirror, 6);

    assert_eq!(transaction.base_serial, 5);
    assert_eq!(transaction.serial, 6);
    assert_eq!(transaction.ops.len(), 2);
    assert!(matches!(transaction.ops[0], LuaOp::New { id: 20, .. }));
}

#[test]
fn a_reattach_rebuilds_the_mirror_from_a_fresh_snapshot() {
    // After a gap the old mirror is discarded: nothing from it survives into the new one.
    let mut mirror = base_mirror();
    let mut set = change(MirrorChangeKind::Set, 2);
    set.key = "opacity".to_owned();
    set.value = Some(LuaValue::Double(0.5));
    mirror.apply_changes(6, &[set]).expect("batch applies");
    assert!(mirror.value(2, "opacity").is_some());

    let mut builder = SnapshotBuilder::new(Attached {
        epoch: 8,
        root: 1,
        serial: 9,
    });
    let fresh = builder
        .chunk(9, 0, true, vec![snap(1, NIL, 0, "node"), snap(5, 1, 0, "node")])
        .expect("snapshot")
        .expect("finished");

    assert_eq!(fresh.epoch(), 8);
    assert_eq!(fresh.serial(), 9);
    assert!(fresh.node(2).is_none());
    assert!(fresh.value(2, "opacity").is_none());
    assert_eq!(children(&fresh, 1), vec![5]);
}

#[test]
fn a_mirror_that_applies_the_same_script_twice_dumps_identically() {
    let run = || {
        let mut mirror = base_mirror();
        let mut created = change(MirrorChangeKind::Created, 30);
        created.class = "node".to_owned();
        let mut parented = change(MirrorChangeKind::Parented, 30);
        parented.parent = 1;
        parented.index = 0;
        let mut set = change(MirrorChangeKind::Set, 30);
        set.key = "width".to_owned();
        set.value = Some(LuaValue::Double(12.5));
        mirror.apply_changes(6, &[created, parented, set]).expect("batch");
        mirror.dump()
    };
    assert_eq!(run(), run());
}

/// A parentless window W(5) and a shell node P(6) under the root(1).
fn window_and_shell_mirror() -> LuaMirror {
    let nodes = [
        snap(1, NIL, 0, "node"),
        snap(5, NIL, 0, "window"),
        snap(6, 1, 0, "node"),
    ];
    LuaMirror::from_snapshot(7, 1, 5, &nodes).expect("snapshot builds")
}

fn parented(node: NodeId, parent: NodeId, index: i64) -> LuaChange {
    LuaChange {
        parent,
        index,
        ..change(MirrorChangeKind::Parented, node)
    }
}

#[test]
fn a_parented_batch_places_a_parentless_window_under_its_shell_node() {
    let mut mirror = window_and_shell_mirror();
    assert_eq!(mirror.node(5).expect("window").parent, NIL);
    let applied = mirror.apply_changes(6, &[parented(5, 6, 0)]).expect("batch applies");
    assert_eq!(applied, LuaApplied::LuaApplied);
    assert_eq!(mirror.node(5).expect("window").parent, 6);
    assert_eq!(children(&mirror, 6), vec![5]);
    assert_eq!(mirror.serial(), 6);
}

#[test]
fn the_echo_of_the_authors_own_transaction_is_applied_not_ignored() {
    // The author's transaction was sent at serial 6, so its echo carries serial 6 > mirror
    // serial 5: it must be applied, never reported as stale.
    let mut mirror = window_and_shell_mirror();
    let applied = mirror.apply_changes(6, &[parented(5, 6, 0)]).expect("echo applies");
    assert_eq!(applied, LuaApplied::LuaApplied);
    assert_eq!(mirror.serial(), 6);
}

#[test]
fn a_replayed_parented_batch_leaves_the_window_listed_once() {
    let mut mirror = window_and_shell_mirror();
    mirror.apply_changes(6, &[parented(5, 6, 0)]).expect("first applies");
    assert_eq!(
        mirror.apply_changes(6, &[parented(5, 6, 0)]).expect("replay is stale"),
        LuaApplied::Ignored
    );
    assert_eq!(children(&mirror, 6), vec![5]);
}

#[test]
fn a_pending_insert_and_its_echo_do_not_double_apply_the_window() {
    // The builder holds the insert until the echo arrives; the committed view must list W once.
    let mut mirror = window_and_shell_mirror();
    let mut tx = LuaTxBuilder::default();
    tx.insert(&mirror, 5, 6, 0).expect("the insert is accepted locally");
    assert_eq!(tx.children(&mirror, 6), Some(&[5][..]));
    assert_eq!(children(&mirror, 6), Vec::<NodeId>::new());
    mirror.apply_changes(6, &[parented(5, 6, 0)]).expect("echo applies");
    assert_eq!(children(&mirror, 6), vec![5]);
}

#[test]
fn debug_of_a_lua_call_never_shows_its_text() {
    const SECRET: &str = "hunter2-never-print-me";
    let op = LuaOp::Call {
        node: 5,
        method: "claim".to_owned(),
        args: vec![7],
        text: gnoblin_secret::SecretText::new(SECRET),
    };
    let transaction = gnoblin_lua::tree_mirror::LuaTransaction {
        base_serial: 1,
        serial: 2,
        ops: vec![op.clone()],
    };
    for rendering in [
        format!("{op:?}"),
        format!("{op:#?}"),
        format!("{transaction:?}"),
        format!("{transaction:#?}"),
    ] {
        assert!(
            !rendering.contains(SECRET),
            "a Debug form shows the call text: {rendering}"
        );
    }
}
