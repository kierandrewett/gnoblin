//! The `tree` namespace on the owner socket (design/node-core.md, sections 4.1 to 4.6).
//!
//! `attach` and `transact` are answered with Reply frames; `snapshot` and `changes` are Event
//! frames the compositor sends after them. Only one shell holds the tree: a second connection's
//! attach is refused until the holder detaches or its connection closes. A holder that attaches
//! again on the same connection (the mirror's gap recovery) gets a fresh epoch and snapshot.

mod support;

use std::time::{Duration, Instant};

use gnoblin_proto::tree::tree::{
    AttachArgs, AttachedPayload, ChangesPayload, ReplyPayload, SnapshotPayload, TransactArgs,
};
use gnoblin_proto::tree::{op, snapshot_node};
use gnoblin_proto::{Envelope, Kind};
use support::{Session, Wire};

const NAMESPACE: &str = "tree";

fn attach(wire: &mut Wire, request_id: u64) {
    wire.send(&Envelope::request_message(
        NAMESPACE,
        "attach",
        request_id,
        &AttachArgs { version: 1 },
    ));
}

/// The next frame, which must be a Reply to `member`.
fn reply<M: prost::Message + Default>(wire: &mut Wire, member: &str) -> M {
    let envelope = wire.receive().expect("a frame");
    assert!(
        matches!(envelope.kind, Some(Kind::Reply(_))),
        "a Reply frame for `{member}`, got {envelope:?}"
    );
    assert_eq!(envelope.namespace, NAMESPACE);
    assert_eq!(envelope.member, member);
    envelope.payload_as::<M>().expect("the payload decodes")
}

/// The next frame, which must be an Event named `member`.
fn event<M: prost::Message + Default>(wire: &mut Wire, member: &str) -> M {
    let envelope = wire.receive().expect("a frame");
    assert!(
        matches!(envelope.kind, Some(Kind::Event(_))),
        "an Event frame for `{member}`, got {envelope:?}"
    );
    assert_eq!(envelope.namespace, NAMESPACE);
    assert_eq!(envelope.member, member);
    envelope.payload_as::<M>().expect("the payload decodes")
}

/// Attach, then drain the snapshot that follows the reply. Returns the attachment.
fn attach_and_snapshot(wire: &mut Wire, request_id: u64) -> (AttachedPayload, Vec<snapshot_node::Properties>) {
    attach(wire, request_id);
    let attached: AttachedPayload = reply(wire, "attach");
    let nodes = snapshot_nodes(wire, attached.serial);
    (attached, nodes)
}

/// The nodes of the snapshot that follows an attach reply, which must all be at `serial`.
fn snapshot_nodes(wire: &mut Wire, serial: u64) -> Vec<snapshot_node::Properties> {
    let mut nodes = Vec::new();
    loop {
        let chunk: SnapshotPayload = event(wire, "snapshot");
        assert_eq!(chunk.serial, serial, "the snapshot is at the attach serial");
        nodes.extend(chunk.nodes);
        if chunk.last {
            break;
        }
    }
    nodes
}

/// Attaches until a refused attach succeeds, so the previous holder's close has been seen. Returns
/// the wire, which holds the tree until it is dropped, with the attachment and its snapshot.
fn attach_once_released(session: &Session, request_id: u64) -> (Wire, AttachedPayload, Vec<snapshot_node::Properties>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut wire = Wire::connect(session.socket());
        attach(&mut wire, request_id);
        let envelope = wire.receive().expect("a frame");
        if envelope.error_code().is_none() {
            let attached: AttachedPayload = envelope.payload_as().expect("the payload decodes");
            let nodes = snapshot_nodes(&mut wire, attached.serial);
            return (wire, attached, nodes);
        }
        assert!(Instant::now() < deadline, "the hold was never released");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Create one node of class `node` under `parent`, with the shell's pre-allocated `id`.
fn create(id: u64, parent: u64) -> Vec<op::Properties> {
    vec![
        op::Properties {
            kind: "new".to_owned(),
            class: "node".to_owned(),
            id,
            ..Default::default()
        },
        op::Properties {
            kind: "insert".to_owned(),
            node: id,
            parent,
            index: 0,
            ..Default::default()
        },
    ]
}

fn transact(wire: &mut Wire, request_id: u64, base_serial: u64, serial: u64, ops: Vec<op::Properties>) {
    wire.send(&Envelope::request_message(
        NAMESPACE,
        "transact",
        request_id,
        &TransactArgs {
            base_serial,
            serial,
            ops,
        },
    ));
}

#[test]
fn the_owner_attaches_and_is_answered_with_its_root_and_a_snapshot() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, nodes) = attach_and_snapshot(&mut wire, 1);
    assert_eq!(attached.epoch, 1, "the first attach is epoch one");
    assert_ne!(attached.root, 0, "the answer names the root node");
    assert_eq!(attached.serial, 0, "a fresh tree starts at serial zero");
    assert!(
        nodes.iter().any(|node| node.id == attached.root && node.parent == 0),
        "the snapshot holds the root as a root node"
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_second_shell_is_refused_while_the_first_holds_the_tree() {
    let session = Session::start();
    let mut first = Wire::connect(session.socket());
    attach_and_snapshot(&mut first, 1);

    let mut second = Wire::connect(session.socket());
    attach(&mut second, 2);
    let refusal = second.receive().expect("an error frame");
    assert_eq!(
        refusal.error_code(),
        Some("shell-already-attached"),
        "the second shell is refused while the first holds the tree"
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn the_same_shell_attaching_again_gets_a_new_epoch_and_a_fresh_snapshot() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (first, _) = attach_and_snapshot(&mut wire, 1);
    let (again, nodes) = attach_and_snapshot(&mut wire, 2);
    assert_eq!(again.epoch, first.epoch + 1, "each attach raises the epoch");
    assert!(!nodes.is_empty(), "the re-attach sends a snapshot");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn the_hold_is_released_when_the_shell_disconnects() {
    let session = Session::start();
    let mut first = Wire::connect(session.socket());
    attach_and_snapshot(&mut first, 1);
    drop(first);

    // The compositor sees the closed connection on its own thread, so an attach may be refused
    // until it has. The hold is released once a new shell's attach succeeds.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut next = Wire::connect(session.socket());
        attach(&mut next, 2);
        let envelope = next.receive().expect("a frame");
        if envelope.error_code().is_none() {
            let answer: AttachedPayload = envelope.payload_as().expect("the payload decodes");
            assert_eq!(answer.epoch, 2, "the attach after the release is the second epoch");
            break;
        }
        assert!(Instant::now() < deadline, "the hold was never released");
        std::thread::sleep(Duration::from_millis(10));
    }
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_transaction_before_attach_is_refused_not_privileged() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    transact(&mut wire, 1, 0, 1, create(1000, 0));
    let reply: ReplyPayload = reply(&mut wire, "transact");
    assert!(!reply.applied);
    assert_eq!(reply.code, "not-privileged");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_transaction_is_applied_and_its_changes_are_published_at_the_next_serial() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);

    transact(&mut wire, 2, 0, 1, create(1000, attached.root));
    let changes: ChangesPayload = event(&mut wire, "changes");
    let reply: ReplyPayload = reply(&mut wire, "transact");
    assert!(reply.applied, "the transaction applies");
    assert_eq!(reply.failed_op, -1);
    assert_eq!(reply.serial, 1, "a batch that changed something advances the serial");

    assert_eq!(changes.serial, 1);
    assert!(
        changes
            .changes
            .iter()
            .any(|change| change.kind == "created" && change.node == 1000),
        "the batch publishes the created node: {:?}",
        changes.changes
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_stale_base_is_refused_and_the_tree_is_unchanged() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);

    transact(&mut wire, 2, 7, 1, create(1001, attached.root));
    let reply: ReplyPayload = reply(&mut wire, "transact");
    assert!(!reply.applied, "a base that is not the current serial is refused");
    assert_eq!(reply.code, "stale-base");
    assert_eq!(reply.serial, 0, "the serial is unchanged");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_list_and_a_bool_set_travel_the_wire_and_come_back_in_the_change_batch() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);

    let mut ops = create(1000, attached.root);
    ops.push(op::Properties {
        kind: "set".to_owned(),
        node: 1000,
        key: "rotation".to_owned(),
        value_kind: "list_double".to_owned(),
        value_list: vec![0.0, 0.0, 45.0],
        ..Default::default()
    });
    ops.push(op::Properties {
        kind: "set".to_owned(),
        node: 1000,
        key: "visible".to_owned(),
        value_kind: "bool".to_owned(),
        value_bool: false,
        ..Default::default()
    });
    transact(&mut wire, 2, 0, 1, ops);
    let changes: ChangesPayload = event(&mut wire, "changes");
    let reply: ReplyPayload = reply(&mut wire, "transact");
    assert!(reply.applied, "the sets validate: {reply:?}");

    let rotation = changes
        .changes
        .iter()
        .find(|change| change.kind == "set" && change.key == "rotation")
        .expect("the rotation set is published");
    assert_eq!(rotation.value_kind, "list_double");
    assert_eq!(rotation.value_list, vec![0.0, 0.0, 45.0]);
    let visible = changes
        .changes
        .iter()
        .find(|change| change.kind == "set" && change.key == "visible")
        .expect("the visible set is published");
    assert_eq!(visible.value_kind, "bool");
    assert!(!visible.value_bool);
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_set_of_a_kind_the_wire_does_not_carry_is_refused_bad_value() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);

    let mut ops = create(1000, attached.root);
    ops.push(op::Properties {
        kind: "set".to_owned(),
        node: 1000,
        key: "visible".to_owned(),
        value_kind: "bytes".to_owned(),
        ..Default::default()
    });
    transact(&mut wire, 2, 0, 1, ops);
    let reply: ReplyPayload = reply(&mut wire, "transact");
    assert!(!reply.applied);
    assert_eq!(reply.code, "bad-value");
    assert_eq!(reply.failed_op, 2, "the third op is the bad one");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_transaction_is_visible_in_the_snapshot_of_the_next_attach() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);

    transact(&mut wire, 2, 0, 1, create(1000, attached.root));
    let _: ChangesPayload = event(&mut wire, "changes");
    let applied: ReplyPayload = reply(&mut wire, "transact");
    assert!(applied.applied, "the transaction applies");

    attach(&mut wire, 3);
    let reattached: AttachedPayload = reply(&mut wire, "attach");
    assert_eq!(
        reattached.serial, 1,
        "the attach starts from the serial the transaction published"
    );
    let nodes = snapshot_nodes(&mut wire, reattached.serial);
    assert!(
        nodes.iter().any(|node| node.id == 1000 && node.parent == attached.root),
        "the snapshot holds the created node under the root: {nodes:?}"
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_detached_shell_leaves_its_nodes_in_the_tree() {
    let session = Session::start();
    let mut first = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut first, 1);

    transact(&mut first, 2, 0, 1, create(1000, attached.root));
    let _: ChangesPayload = event(&mut first, "changes");
    let _: ReplyPayload = reply(&mut first, "transact");
    drop(first);

    let (_holder, reattached, nodes) = attach_once_released(&session, 3);
    assert_eq!(reattached.serial, 1, "the tree keeps the serial the shell left it at");
    assert!(
        nodes.iter().any(|node| node.id == 1000 && node.parent == attached.root),
        "the next shell's snapshot still holds the node: {nodes:?}"
    );
    session.stop().expect("the session stops cleanly");
}
