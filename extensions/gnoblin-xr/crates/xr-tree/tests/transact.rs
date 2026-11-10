//! Owner transactions (`new`, `set`, `input`) against a fake owner socket.

use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::thread;

use gnoblin_proto::tree::change;
use gnoblin_proto::tree::snapshot_node;
use gnoblin_proto::tree::tree::{AttachedPayload, ReplyPayload, SnapshotPayload, TransactArgs};
use gnoblin_proto::{Envelope, read_frame, write_frame};
use xr_tree::{PointerInput, TreeClient, TreeError};

/// The owner epoch the fake attach reply carries. Seat ids start with it.
const EPOCH: u64 = 1;
/// The request id of the client's attach request.
const ATTACH_REQUEST_ID: u64 = 1;

/// How the fake owner answers the one transaction the test sends.
enum Answer {
    /// Applies it and reports the serial.
    Applied,
    /// Refuses the request outright with an error envelope and this code.
    Refused(&'static str),
    /// Answers with a reply the owner did not apply, with this code.
    Unapplied(&'static str),
}

/// Starts a fake owner. It answers the attach with an empty snapshot of `nodes`, reads one
/// transaction, answers it as `answer`, and sends the transaction it read down the channel.
fn serve(nodes: Vec<snapshot_node::Properties>, answer: Answer) -> (UnixStream, mpsc::Receiver<TransactArgs>) {
    let (client, mut server) = UnixStream::pair().expect("socket pair");
    let (sent, received) = mpsc::channel();
    thread::spawn(move || {
        let _attach = read_frame(&mut server).expect("read attach").expect("attach frame");
        let attached = Envelope::reply_message(
            "tree",
            "attach",
            ATTACH_REQUEST_ID,
            &AttachedPayload {
                epoch: EPOCH,
                root: 1,
                serial: 1,
            },
        );
        let snapshot = Envelope::event_message(
            "tree",
            "snapshot",
            &SnapshotPayload {
                serial: 1,
                chunk: 0,
                last: true,
                nodes,
            },
        );
        write_frame(&mut server, &attached).expect("write attach");
        write_frame(&mut server, &snapshot).expect("write snapshot");

        let Some(request) = read_frame(&mut server).expect("read transaction") else {
            return;
        };
        let args: TransactArgs = request.payload_as().expect("transaction payload");
        sent.send(args).expect("record transaction");
        let id = request.request_id;
        let reply = match answer {
            Answer::Applied => Envelope::reply_message(
                "tree",
                "transact",
                id,
                &ReplyPayload {
                    serial: 2,
                    applied: true,
                    ..Default::default()
                },
            ),
            Answer::Refused(code) => Envelope::error("tree", "transact", id, code, "refused"),
            Answer::Unapplied(code) => Envelope::reply_message(
                "tree",
                "transact",
                id,
                &ReplyPayload {
                    serial: 2,
                    applied: false,
                    code: code.into(),
                    ..Default::default()
                },
            ),
        };
        let _ = write_frame(&mut server, &reply);
    });
    (client, received)
}

fn text(node: u64, key: &str, value: &str) -> change::Properties {
    change::Properties {
        kind: "set".into(),
        node,
        key: key.into(),
        value_kind: "text".into(),
        value_text: value.into(),
        ..Default::default()
    }
}

fn seat_node(id: u64, name: &str) -> snapshot_node::Properties {
    snapshot_node::Properties {
        id,
        parent: 0,
        index: 0,
        class: "seat".into(),
        props: vec![text(id, "name", name)],
        ..Default::default()
    }
}

fn connect(nodes: Vec<snapshot_node::Properties>, answer: Answer) -> (TreeClient, mpsc::Receiver<TransactArgs>) {
    let (stream, received) = serve(nodes, answer);
    (TreeClient::from_stream(stream).expect("attach"), received)
}

#[test]
fn create_seat_sends_new_and_name_in_one_transaction() {
    let (mut client, received) = connect(vec![], Answer::Applied);

    let seat = client.create_seat("xr").expect("create seat");

    assert_eq!(seat, (EPOCH << 32) | 1, "the seat id is the epoch above a counter");
    let args = received.recv().expect("the transaction arrived");
    assert_eq!(args.ops.len(), 2, "new and set travel in one transaction");
    assert_eq!(args.ops[0].kind, "new");
    assert_eq!(args.ops[0].class, "seat");
    assert_eq!(args.ops[0].id, seat);
    assert_eq!(args.ops[1].kind, "set");
    assert_eq!(args.ops[1].node, seat);
    assert_eq!(args.ops[1].key, "name");
    assert_eq!(args.ops[1].value_text, "xr");
}

#[test]
fn a_refused_request_surfaces_as_refused_with_its_code() {
    let (mut client, _received) = connect(vec![], Answer::Refused("not-owner"));

    let error = client.create_seat("xr").expect_err("the owner refuses");

    assert!(
        matches!(&error, TreeError::Refused { code, .. } if code == "not-owner"),
        "got {error:?}"
    );
}

#[test]
fn an_unapplied_reply_surfaces_as_refused_with_its_code() {
    let (mut client, _received) = connect(vec![], Answer::Unapplied("stale-base"));

    let error = client.create_seat("xr").expect_err("the owner does not apply it");

    assert!(
        matches!(&error, TreeError::Refused { code, .. } if code == "stale-base"),
        "got {error:?}"
    );
}

#[test]
fn a_seat_in_the_snapshot_is_found_by_name() {
    let (client, _received) = connect(vec![seat_node(7, "xr"), seat_node(8, "other")], Answer::Applied);

    assert_eq!(client.seat_named("xr"), Some(7));
    assert_eq!(client.seat_named("missing"), None);
}

#[test]
fn pointer_input_is_an_owner_input_op() {
    let (mut client, received) = connect(vec![], Answer::Applied);

    client
        .send_input(&PointerInput::Button {
            seat: 7,
            node: 9,
            x: 12.5,
            y: 3.0,
            button: 272,
            pressed: true,
        })
        .expect("send input");

    let op = &received.recv().expect("the transaction arrived").ops[0];
    assert_eq!(op.kind, "input");
    assert_eq!(op.input_kind, "button");
    assert_eq!(op.seat, 7);
    assert_eq!(op.node, 9);
    assert_eq!(op.x, 12.5);
    assert_eq!(op.y, 3.0);
    assert_eq!(op.button, 272);
    assert_eq!(op.state, 1, "a press is state 1");
}
