//! A holder parents a mapped window under its own node, then a fresh observer reads the tree.
//!
//! Asserts, in order: the window is a parentless `window` node in the holder's snapshot; the
//! holder's transaction creating `P` under the root applies; the holder's transaction inserting
//! the window under `P` applies; the changes that arrive after that reply are recorded, with
//! whether the holder itself gets a `parented` change for the window; a fresh observer's
//! snapshot shows the window under `P`.

mod support;

use std::time::{Duration, Instant};

use gnoblin_proto::tree::tree::{
    AttachArgs, AttachedPayload, ChangesPayload, ReplyPayload, SnapshotPayload, TransactArgs,
};
use gnoblin_proto::tree::{change, op, snapshot_node};
use gnoblin_proto::{Envelope, Kind};
use support::{Client, Session, Wire};

const NAMESPACE: &str = "tree";
const SHELL_PARENT: u64 = 2000;

fn attach(wire: &mut Wire, request_id: u64) {
    wire.send(&Envelope::request_message(
        NAMESPACE,
        "attach",
        request_id,
        &AttachArgs { version: 1 },
    ));
}

fn reply<M: prost::Message + Default>(wire: &mut Wire, member: &str) -> M {
    let envelope = wire.receive().expect("a frame");
    assert!(
        matches!(envelope.kind, Some(Kind::Reply(_))),
        "a Reply frame for `{member}`, got {envelope:?}"
    );
    assert_eq!(envelope.member, member);
    envelope.payload_as::<M>().expect("the payload decodes")
}

fn event<M: prost::Message + Default>(wire: &mut Wire, member: &str) -> M {
    let envelope = wire.receive().expect("a frame");
    assert!(
        matches!(envelope.kind, Some(Kind::Event(_))),
        "an Event frame for `{member}`, got {envelope:?}"
    );
    assert_eq!(envelope.member, member);
    envelope.payload_as::<M>().expect("the payload decodes")
}

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

/// Re-attaches on this connection until the snapshot holds the window titled `title`.
fn snapshot_holding_window(wire: &mut Wire, title: &str) -> (AttachedPayload, Vec<snapshot_node::Properties>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut request_id = 1;
    loop {
        attach(wire, request_id);
        let attached: AttachedPayload = reply(wire, "attach");
        let nodes = snapshot_nodes(wire, attached.serial);
        if nodes.iter().any(|node| node.class == "window" && node.title == title) {
            return (attached, nodes);
        }
        assert!(Instant::now() < deadline, "no window titled `{title}`: {nodes:?}");
        request_id += 1;
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Attaches a fresh connection once the previous holder has released the tree.
fn fresh_observer(session: &Session) -> (Wire, AttachedPayload, Vec<snapshot_node::Properties>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut wire = Wire::connect(session.socket());
        attach(&mut wire, 1);
        let envelope = wire.receive().expect("a frame");
        if envelope.error_code().is_none() {
            let attached: AttachedPayload = envelope.payload_as().expect("the payload decodes");
            let nodes = snapshot_nodes(&mut wire, attached.serial);
            return (wire, attached, nodes);
        }
        assert!(Instant::now() < deadline, "the holder was never released");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Sends a transaction and reads frames until its reply, collecting the change batches before it.
fn transact(
    wire: &mut Wire,
    request_id: u64,
    base_serial: u64,
    serial: u64,
    ops: Vec<op::Properties>,
) -> (ReplyPayload, Vec<change::Properties>) {
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
    let mut changes = Vec::new();
    loop {
        let envelope = wire.receive().expect("a frame");
        match envelope.kind {
            Some(Kind::Event(_)) => {
                assert_eq!(envelope.member, "changes");
                let batch: ChangesPayload = envelope.payload_as().expect("the payload decodes");
                changes.extend(batch.changes);
            }
            Some(Kind::Reply(_)) => {
                assert_eq!(envelope.member, "transact");
                let reply: ReplyPayload = envelope.payload_as().expect("the payload decodes");
                return (reply, changes);
            }
            _ => panic!("an unexpected frame in the transact reply loop"),
        }
    }
}

/// Reads whatever change batches arrive in the next 300 ms after a reply.
fn drain_changes(wire: &mut Wire) -> Vec<change::Properties> {
    wire.set_read_timeout(Duration::from_millis(300));
    let mut changes = Vec::new();
    while let Some(envelope) = wire.receive_within() {
        if matches!(envelope.kind, Some(Kind::Event(_))) && envelope.member == "changes" {
            let batch: ChangesPayload = envelope.payload_as().expect("the payload decodes");
            changes.extend(batch.changes);
        }
    }
    changes
}

fn create_node(id: u64, parent: u64) -> Vec<op::Properties> {
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

fn insert(node: u64, parent: u64) -> op::Properties {
    op::Properties {
        kind: "insert".to_owned(),
        node,
        parent,
        index: 0,
        ..Default::default()
    }
}

#[test]
fn the_holder_parents_a_window_and_a_fresh_observer_sees_it_under_its_node() {
    let session = Session::start();
    let mut client = Client::connect(session.wayland());
    let _window = client.map_window("org.example.observer", "Observed");
    let mut holder = Wire::connect(session.socket());

    // A1: the window is a parentless window node in the holder's snapshot.
    let (attached, nodes) = snapshot_holding_window(&mut holder, "Observed");
    let window = nodes
        .iter()
        .find(|node| node.class == "window" && node.title == "Observed")
        .expect("the window node");
    let window_id = window.id;
    assert_eq!(window.parent, 0, "A1: the window is parentless");

    // A2: the holder creates P under the root.
    let (reply, _) = transact(
        &mut holder,
        10,
        attached.serial,
        attached.serial + 1,
        create_node(SHELL_PARENT, attached.root),
    );
    assert!(reply.applied, "A2: P applies: {reply:?}");
    let parent_serial = reply.serial;

    // A3: the holder inserts the window under P.
    let (reply, mut changes) = transact(
        &mut holder,
        11,
        parent_serial,
        parent_serial + 1,
        vec![insert(window_id, SHELL_PARENT)],
    );
    assert!(reply.applied, "A3: insert W into P applies: {reply:?}");
    changes.extend(drain_changes(&mut holder));

    // Recorded, not asserted: whether the author gets its own reparent back.
    let echoed = changes
        .iter()
        .any(|change| change.kind == "parented" && change.node == window_id && change.parent == SHELL_PARENT);
    eprintln!("OBSERVER echo_to_author={echoed} batches_after_reply={changes:?}");

    // A4: a fresh observer's snapshot shows the window under P, and P under the root.
    drop(holder);
    let (_fresh, _attached, nodes) = fresh_observer(&session);
    let window_after = nodes
        .iter()
        .find(|node| node.id == window_id)
        .expect("W in the snapshot");
    assert_eq!(
        window_after.parent, SHELL_PARENT,
        "A4: W is under P in a fresh snapshot"
    );
    let parent_after = nodes
        .iter()
        .find(|node| node.id == SHELL_PARENT)
        .expect("P in the snapshot");
    assert_eq!(parent_after.parent, attached.root, "A4: P is under the root");
    session.stop().expect("the session stops cleanly");
}
