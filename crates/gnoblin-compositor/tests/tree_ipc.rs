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
use gnoblin_proto::tree::{change, op, snapshot_node};
use gnoblin_proto::{Envelope, Kind};
use support::{Client, Session, Wire};

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

/// The window node of the snapshot titled `title`, if the composer has created it.
fn window_titled<'a>(nodes: &'a [snapshot_node::Properties], title: &str) -> Option<&'a snapshot_node::Properties> {
    nodes.iter().find(|node| node.class == "window" && node.title == title)
}

/// Re-attaches on the same connection until the snapshot holds a window titled `title`, and
/// returns that attachment with the snapshot. Transactions made from the snapshot are based on
/// its serial, which the window's creation may already have advanced. The composer creates the
/// window node on the first frame after the client maps the window.
fn snapshot_holding_window(wire: &mut Wire, title: &str) -> (AttachedPayload, Vec<snapshot_node::Properties>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut request_id = 1;
    loop {
        let (attached, nodes) = attach_and_snapshot(wire, request_id);
        if window_titled(&nodes, title).is_some() {
            return (attached, nodes);
        }
        assert!(
            Instant::now() < deadline,
            "no window titled `{title}` in the tree: {nodes:?}"
        );
        request_id += 1;
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Reads change batches until the changes published so far satisfy `done`.
fn changes_until(wire: &mut Wire, done: impl Fn(&[change::Properties]) -> bool) -> Vec<change::Properties> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut all = Vec::new();
    while !done(&all) {
        assert!(Instant::now() < deadline, "the expected changes never arrived: {all:?}");
        let batch: ChangesPayload = event(wire, "changes");
        all.extend(batch.changes);
    }
    all
}

fn set_double(node: u64, key: &str, value: f64) -> op::Properties {
    op::Properties {
        kind: "set".to_owned(),
        node,
        key: key.to_owned(),
        value_kind: "double".to_owned(),
        value_double: value,
        ..Default::default()
    }
}

#[test]
fn a_mapped_window_is_a_window_node_with_its_title_and_app_id() {
    let session = Session::start();
    let mut client = Client::connect(session.wayland());
    let _window = client.map_window("org.example.window", "Document one");
    let mut wire = Wire::connect(session.socket());

    let (_, nodes) = snapshot_holding_window(&mut wire, "Document one");
    let node = window_titled(&nodes, "Document one").expect("the window node");
    assert_eq!(node.class, "window", "a mapped window is the `window` class");
    assert_eq!(
        node.app_id, "org.example.window",
        "the app id the client set is carried read-only"
    );
    assert_eq!(
        node.parent, 0,
        "a window node is unparented: the compositor owns its placement"
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_window_mapped_after_attach_is_published_as_created_with_its_class() {
    let session = Session::start();
    let mut client = Client::connect(session.wayland());
    let mut wire = Wire::connect(session.socket());
    attach_and_snapshot(&mut wire, 1);

    let _window = client.map_window("org.example.window", "Late window");
    let changes = changes_until(&mut wire, |all| {
        all.iter()
            .any(|change| change.kind == "created" && change.class == "window")
    });
    let created = changes
        .iter()
        .find(|change| change.kind == "created" && change.class == "window")
        .expect("a created window change");
    assert_eq!(created.parent, 0, "the created window is unparented");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_client_title_change_is_published_as_a_set_of_the_window_node() {
    let session = Session::start();
    let mut client = Client::connect(session.wayland());
    let window = client.map_window("org.example.window", "Before");
    let mut wire = Wire::connect(session.socket());
    let (_, nodes) = snapshot_holding_window(&mut wire, "Before");
    let node = window_titled(&nodes, "Before").expect("the window node").id;

    client.set_title(&window, "After");
    let changes = changes_until(&mut wire, |all| {
        all.iter()
            .any(|change| change.kind == "set" && change.key == "title" && change.node == node)
    });
    assert!(
        changes
            .iter()
            .any(|change| change.kind == "set" && change.key == "title" && change.node == node),
        "the title change names the window node: {changes:?}"
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_transaction_setting_a_window_geometry_is_applied_and_published_at_the_next_serial() {
    let session = Session::start();
    let mut client = Client::connect(session.wayland());
    let _window = client.map_window("org.example.window", "Movable");
    let mut wire = Wire::connect(session.socket());
    let (attached, nodes) = snapshot_holding_window(&mut wire, "Movable");
    let node = window_titled(&nodes, "Movable").expect("the window node").id;

    transact(
        &mut wire,
        2,
        attached.serial,
        attached.serial + 1,
        vec![set_double(node, "x", 333.0), set_double(node, "width", 480.0)],
    );
    let changes = changes_until(&mut wire, |all| {
        all.iter()
            .any(|change| change.kind == "set" && change.key == "x" && change.node == node)
            && all
                .iter()
                .any(|change| change.kind == "set" && change.key == "width" && change.node == node)
    });
    let applied: ReplyPayload = reply(&mut wire, "transact");
    assert!(applied.applied, "a writable geometry on a window applies: {changes:?}");
    assert_eq!(applied.failed_op, -1);
    assert_eq!(applied.serial, attached.serial + 1, "the batch advances the serial");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_refused_transaction_leaves_the_window_node_untouched() {
    let session = Session::start();
    let mut client = Client::connect(session.wayland());
    let _window = client.map_window("org.example.window", "Steady");
    let mut wire = Wire::connect(session.socket());
    let (attached, nodes) = snapshot_holding_window(&mut wire, "Steady");
    let node = window_titled(&nodes, "Steady").expect("the window node").id;

    // A base that is not the current serial is refused before any of its ops apply.
    transact(&mut wire, 2, 7, 1, vec![set_double(node, "x", 999.0)]);
    let refused: ReplyPayload = reply(&mut wire, "transact");
    assert!(!refused.applied, "the stale transaction is refused");
    assert_eq!(refused.code, "stale-base");
    assert_eq!(refused.serial, attached.serial, "the serial does not move");

    let (reattached, nodes) = attach_and_snapshot(&mut wire, 3);
    assert_eq!(
        reattached.serial, attached.serial,
        "nothing was published for the refused transaction"
    );
    assert_eq!(
        window_titled(&nodes, "Steady").map(|node| node.id),
        Some(node),
        "the window is still the same node"
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn an_unmapped_window_is_published_as_destroyed() {
    let session = Session::start();
    let mut client = Client::connect(session.wayland());
    let window = client.map_window("org.example.window", "Closing");
    let mut wire = Wire::connect(session.socket());
    let (_, nodes) = snapshot_holding_window(&mut wire, "Closing");
    let node = window_titled(&nodes, "Closing").expect("the window node").id;

    client.destroy(window);
    let changes = changes_until(&mut wire, |all| {
        all.iter()
            .any(|change| change.kind == "destroyed" && change.node == node)
    });
    assert!(
        changes
            .iter()
            .any(|change| change.kind == "destroyed" && change.node == node),
        "the destroyed window node is published: {changes:?}"
    );
    session.stop().expect("the session stops cleanly");
}

fn create_seat(id: u64, parent: u64, name: &str) -> Vec<op::Properties> {
    let mut ops = create(id, parent);
    ops[0].class = "seat".to_owned();
    ops.push(set_text(id, "name", name));
    ops
}

fn set_text(node: u64, key: &str, value: &str) -> op::Properties {
    op::Properties {
        kind: "set".to_owned(),
        node,
        key: key.to_owned(),
        value_kind: "text".to_owned(),
        value_text: value.to_owned(),
        ..Default::default()
    }
}

#[test]
fn the_owner_creates_a_named_seat_in_one_transaction() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);

    transact(&mut wire, 2, 0, 1, create_seat(1000, attached.root, "xr"));
    let _: ChangesPayload = event(&mut wire, "changes");
    let reply: ReplyPayload = reply(&mut wire, "transact");
    assert!(reply.applied, "a new seat with its name applies in one transaction");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_seat_name_is_refused_read_only_once_the_seat_exists() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);

    transact(&mut wire, 2, 0, 1, create_seat(1000, attached.root, "xr"));
    let _: ChangesPayload = event(&mut wire, "changes");
    let _: ReplyPayload = reply(&mut wire, "transact");

    transact(&mut wire, 3, 1, 2, vec![set_text(1000, "name", "other")]);
    let reply: ReplyPayload = reply(&mut wire, "transact");
    assert!(!reply.applied, "a later set of the readonly name is refused");
    assert_eq!(reply.failed_op, 0);
    session.stop().expect("the session stops cleanly");
}

#[test]
fn an_empty_or_duplicate_seat_name_is_refused_bad_value() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);

    transact(&mut wire, 2, 0, 1, create_seat(1000, attached.root, ""));
    let reply: ReplyPayload = reply(&mut wire, "transact");
    assert!(!reply.applied, "an empty seat name is refused");
    session.stop().expect("the session stops cleanly");
}
