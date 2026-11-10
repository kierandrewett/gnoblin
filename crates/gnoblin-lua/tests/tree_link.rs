//! The socket link against a scripted compositor end. The test holds the other end of a
//! socketpair, reads what the link writes and answers with the frames the compositor sends.
//! Every reply is written before the link is polled, so nothing here waits on a thread.

use std::io::Write;
use std::os::unix::net::UnixStream;

use gnoblin_lua::tree_link::SocketTreeLink;
use gnoblin_lua::tree_mirror::{
    Attached, LuaChange, LuaChord, LuaIncoming, LuaOp, LuaTransaction, LuaValue, MirrorChangeKind, SnapshotNode,
    TreeLink, TxReply,
};
use gnoblin_proto::tree::seat::ChordPayload;
use gnoblin_proto::tree::tree::{
    AttachArgs, AttachedPayload, ChangesPayload, ReplyPayload, SnapshotPayload, TransactArgs,
};
use gnoblin_proto::tree::{change, snapshot_node};
use gnoblin_proto::{Envelope, read_frame, write_frame};

fn pair() -> (SocketTreeLink, UnixStream) {
    let (client, server) = UnixStream::pair().expect("a socketpair");
    (
        SocketTreeLink::new(client).expect("the link wraps the client end"),
        server,
    )
}

/// The next frame the link wrote.
fn receive(server: &mut UnixStream) -> Envelope {
    read_frame(server).expect("a frame").expect("the link has not closed")
}

fn send(server: &mut UnixStream, envelope: &Envelope) {
    write_frame(server, envelope).expect("the frame writes");
}

fn drain(link: &mut SocketTreeLink) -> Vec<LuaIncoming> {
    link.poll().expect("the link polls")
}

/// A change with every field at its empty value, for the test to set the fields it means.
fn bare(kind: MirrorChangeKind, node: u64) -> LuaChange {
    LuaChange {
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

#[test]
fn a_chord_event_arrives_as_a_chord_not_a_node_event() {
    let (mut link, mut server) = pair();
    send(
        &mut server,
        &Envelope::event_message(
            "tree",
            "chord",
            &ChordPayload {
                id: 7,
                chord: "Super+q".to_owned(),
                state: "press".to_owned(),
            },
        ),
    );
    assert_eq!(
        drain(&mut link),
        vec![LuaIncoming::Chord(LuaChord {
            claim: 7,
            chord: "Super+q".to_owned(),
            state: "press".to_owned(),
        })]
    );
}

#[test]
fn poll_with_nothing_arrived_returns_empty_without_blocking() {
    let (mut link, _server) = pair();
    assert_eq!(drain(&mut link), Vec::new());
}

#[test]
fn attach_sends_the_version_and_poll_yields_the_attached_reply_then_the_snapshot() {
    let (mut link, mut server) = pair();
    link.attach(1).expect("attach sends");

    let request = receive(&mut server);
    assert_eq!(request.namespace, "tree");
    assert_eq!(request.member, "attach");
    let args: AttachArgs = request.payload_as().expect("the attach args decode");
    assert_eq!(args.version, 1);

    send(
        &mut server,
        &Envelope::reply_message(
            "tree",
            "attach",
            request.request_id,
            &AttachedPayload {
                epoch: 1,
                root: 1,
                serial: 0,
            },
        ),
    );
    send(
        &mut server,
        &Envelope::event_message(
            "tree",
            "snapshot",
            &SnapshotPayload {
                serial: 0,
                chunk: 0,
                last: true,
                nodes: vec![snapshot_node::Properties {
                    id: 1,
                    parent: 0,
                    index: 0,
                    class: "node".to_owned(),
                    title: String::new(),
                    app_id: String::new(),
                    props: vec![],
                }],
            },
        ),
    );

    assert_eq!(
        drain(&mut link),
        vec![
            LuaIncoming::Attached(Attached {
                epoch: 1,
                root: 1,
                serial: 0,
            }),
            LuaIncoming::Snapshot {
                serial: 0,
                chunk: 0,
                last: true,
                nodes: vec![SnapshotNode {
                    id: 1,
                    parent: 0,
                    index: 0,
                    class: "node".to_owned(),
                    title: String::new(),
                    app_id: String::new(),
                    props: Vec::new(),
                }],
            },
        ]
    );
}

#[test]
fn transact_encodes_every_op_kind_and_poll_decodes_the_reply() {
    let (mut link, mut server) = pair();
    let tx = LuaTransaction {
        base_serial: 0,
        serial: 1,
        ops: vec![
            LuaOp::New {
                id: 1000,
                class: "node".to_owned(),
            },
            LuaOp::Insert {
                node: 1000,
                parent: 1,
                index: -1,
            },
            LuaOp::Set {
                node: 1000,
                key: "visible".to_owned(),
                value: LuaValue::Bool(true),
                seconds: Some(0.5),
                samples: Some(vec![0.0, 1.0]),
            },
            LuaOp::Call {
                node: 1000,
                method: "focus".to_owned(),
                args: vec![1, 2],
                text: String::new(),
            },
            LuaOp::Call {
                node: 2,
                method: "claim".to_owned(),
                args: vec![7, 1],
                text: "Super+q".to_owned(),
            },
            LuaOp::Destroy { node: 1000 },
        ],
    };
    link.transact(&tx).expect("transact sends");

    let request = receive(&mut server);
    assert_eq!(request.member, "transact");
    let args: TransactArgs = request.payload_as().expect("the transact args decode");
    assert_eq!((args.base_serial, args.serial), (0, 1));
    let kinds: Vec<&str> = args.ops.iter().map(|wire| wire.kind.as_str()).collect();
    assert_eq!(kinds, ["new", "insert", "set", "call", "call", "destroy"]);
    assert_eq!(args.ops[0].class, "node");
    assert_eq!(args.ops[0].id, 1000);
    assert_eq!(
        args.ops[1].index, -1,
        "a negative index appends and crosses the wire as-is"
    );
    assert_eq!(args.ops[2].value_kind, "bool");
    assert!(args.ops[2].value_bool);
    assert_eq!(args.ops[2].seconds, 0.5);
    assert_eq!(args.ops[2].samples, vec![0.0, 1.0]);
    assert_eq!(args.ops[3].method, "focus");
    assert_eq!(args.ops[3].args, vec![1, 2]);
    assert_eq!(args.ops[4].method, "claim");
    assert_eq!(
        args.ops[4].text, "Super+q",
        "a claim's chord crosses the wire as the op text"
    );
    assert_eq!(args.ops[5].node, 1000);

    send(
        &mut server,
        &Envelope::reply_message(
            "tree",
            "transact",
            request.request_id,
            &ReplyPayload {
                serial: 1,
                applied: true,
                failed_op: -1,
                code: String::new(),
                removed: vec![1000],
            },
        ),
    );
    assert_eq!(
        drain(&mut link),
        vec![LuaIncoming::Reply(TxReply {
            serial: 1,
            applied: true,
            failed_op: -1,
            code: String::new(),
            removed: vec![1000],
        })]
    );
}

#[test]
fn refused_transact_reply_keeps_the_failed_op_and_the_code() {
    let (mut link, mut server) = pair();
    link.transact(&LuaTransaction {
        base_serial: 7,
        serial: 8,
        ops: Vec::new(),
    })
    .expect("transact sends");
    let request = receive(&mut server);
    send(
        &mut server,
        &Envelope::reply_message(
            "tree",
            "transact",
            request.request_id,
            &ReplyPayload {
                serial: 0,
                applied: false,
                failed_op: 2,
                code: "bad-value".to_owned(),
                removed: Vec::new(),
            },
        ),
    );
    assert_eq!(
        drain(&mut link),
        vec![LuaIncoming::Reply(TxReply {
            serial: 0,
            applied: false,
            failed_op: 2,
            code: "bad-value".to_owned(),
            removed: Vec::new(),
        })]
    );
}

#[test]
fn changes_decode_every_change_kind_and_value_kind() {
    let (mut link, mut server) = pair();
    send(
        &mut server,
        &Envelope::event_message(
            "tree",
            "changes",
            &ChangesPayload {
                serial: 1,
                changes: vec![
                    change::Properties {
                        kind: "created".to_owned(),
                        node: 1000,
                        class: "node".to_owned(),
                        index: 0,
                        ..Default::default()
                    },
                    change::Properties {
                        kind: "parented".to_owned(),
                        node: 1000,
                        parent: 1,
                        index: 0,
                        ..Default::default()
                    },
                    change::Properties {
                        kind: "set".to_owned(),
                        node: 1000,
                        key: "rotation".to_owned(),
                        value_kind: "double".to_owned(),
                        value_double: 90.0,
                        ..Default::default()
                    },
                    change::Properties {
                        kind: "set".to_owned(),
                        node: 1000,
                        key: "owner".to_owned(),
                        value_kind: "node".to_owned(),
                        value_node: 7,
                        ..Default::default()
                    },
                    change::Properties {
                        kind: "set".to_owned(),
                        node: 1000,
                        key: "samples".to_owned(),
                        value_kind: "list_double".to_owned(),
                        value_list: vec![1.0, 2.0],
                        ..Default::default()
                    },
                    change::Properties {
                        kind: "destroyed".to_owned(),
                        node: 1000,
                        removed: vec![1000, 1001],
                        ..Default::default()
                    },
                ],
            },
        ),
    );

    assert_eq!(
        drain(&mut link),
        vec![LuaIncoming::Changes {
            serial: 1,
            changes: vec![
                LuaChange {
                    class: "node".to_owned(),
                    ..bare(MirrorChangeKind::Created, 1000)
                },
                LuaChange {
                    parent: 1,
                    ..bare(MirrorChangeKind::Parented, 1000)
                },
                LuaChange {
                    key: "rotation".to_owned(),
                    value: Some(LuaValue::Double(90.0)),
                    ..bare(MirrorChangeKind::Set, 1000)
                },
                LuaChange {
                    key: "owner".to_owned(),
                    value: Some(LuaValue::Node(7)),
                    ..bare(MirrorChangeKind::Set, 1000)
                },
                LuaChange {
                    key: "samples".to_owned(),
                    value: Some(LuaValue::List(vec![1.0, 2.0])),
                    ..bare(MirrorChangeKind::Set, 1000)
                },
                LuaChange {
                    removed: vec![1000, 1001],
                    ..bare(MirrorChangeKind::Destroyed, 1000)
                },
            ],
        }]
    );
}

#[test]
fn a_refused_attach_is_a_link_error_naming_the_code() {
    let (mut link, mut server) = pair();
    link.attach(1).expect("attach sends");
    let request = receive(&mut server);
    send(
        &mut server,
        &Envelope::error(
            "tree",
            "attach",
            request.request_id,
            "not-privileged",
            "the shell is not privileged",
        ),
    );
    let error = link.poll().expect_err("the refusal is an error");
    assert!(
        error.0.contains("not-privileged"),
        "the error names the code: {}",
        error.0
    );
}

#[test]
fn a_closed_socket_is_a_link_error() {
    let (mut link, server) = pair();
    drop(server);
    link.poll().expect_err("a closed compositor end fails the link");
}

#[test]
fn a_reply_to_an_unknown_member_is_a_link_error() {
    let (mut link, mut server) = pair();
    send(&mut server, &Envelope::reply("tree", "bogus", 1, Vec::new()));
    link.poll().expect_err("an unknown member is not silently dropped");
}

#[test]
fn an_undecodable_payload_is_a_link_error_not_a_panic() {
    let (mut link, mut server) = pair();
    send(&mut server, &Envelope::reply("tree", "attach", 1, vec![0xff, 0xff]));
    link.poll().expect_err("a payload that does not decode fails the link");
}

#[test]
fn an_undecodable_frame_is_a_link_error_not_a_panic() {
    let (mut link, mut server) = pair();
    server
        .write_all(&[0, 0, 0, 3, 0xff, 0xff, 0xff])
        .expect("raw bytes write");
    link.poll().expect_err("a frame that does not decode fails the link");
}

#[test]
fn a_frame_cut_short_by_the_compositor_is_a_link_error() {
    let (mut link, mut server) = pair();
    server.write_all(&[0, 0, 0, 100, 1, 2, 3]).expect("raw bytes write");
    drop(server);
    link.poll().expect_err("a truncated frame fails the link");
}

#[test]
fn list_text_and_list_effect_changes_decode_to_their_lua_values() {
    let (mut link, mut server) = pair();
    let blur = gnoblin_proto::tree::effect::Properties {
        kind: "blur".to_owned(),
        enabled: true,
        blur_radius: 4.0,
        ..Default::default()
    };
    send(
        &mut server,
        &Envelope::event_message(
            "tree",
            "changes",
            &ChangesPayload {
                serial: 1,
                changes: vec![
                    change::Properties {
                        kind: "set".to_owned(),
                        node: 1000,
                        key: "tags".to_owned(),
                        value_kind: "list_text".to_owned(),
                        value_texts: vec!["a".to_owned(), "b".to_owned()],
                        ..Default::default()
                    },
                    change::Properties {
                        kind: "set".to_owned(),
                        node: 1000,
                        key: "effects".to_owned(),
                        value_kind: "list_effect".to_owned(),
                        value_effects: vec![blur.clone()],
                        ..Default::default()
                    },
                ],
            },
        ),
    );

    assert_eq!(
        drain(&mut link),
        vec![LuaIncoming::Changes {
            serial: 1,
            changes: vec![
                LuaChange {
                    key: "tags".to_owned(),
                    value: Some(LuaValue::Texts(vec!["a".to_owned(), "b".to_owned()])),
                    ..bare(MirrorChangeKind::Set, 1000)
                },
                LuaChange {
                    key: "effects".to_owned(),
                    value: Some(LuaValue::Effects(vec![blur])),
                    ..bare(MirrorChangeKind::Set, 1000)
                },
            ],
        }]
    );
}
