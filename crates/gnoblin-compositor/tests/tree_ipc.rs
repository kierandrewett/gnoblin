//! The `tree` namespace on the owner socket (design/node-core.md, sections 4.1 to 4.6).
//!
//! `attach` and `transact` are answered with Reply frames; `snapshot` and `changes` are Event
//! frames the compositor sends after them. Only one shell holds the tree: a second connection's
//! attach is refused until the holder detaches or its connection closes. A holder that attaches
//! again on the same connection (the mirror's gap recovery) gets a fresh epoch and snapshot.

mod support;

use std::time::{Duration, Instant};

use gnoblin_proto::tree::tree::{
    AnswerPayload, AttachArgs, AttachedPayload, ChangesPayload, EventPayload, ReplyPayload, SnapshotPayload,
    TransactArgs,
};
use gnoblin_proto::tree::{change, effect, op, snapshot_node};
use gnoblin_proto::{Envelope, Kind};
use support::{Client, Session, Wire};
use wayland_client::Proxy;
use wayland_protocols::xdg::shell::client::xdg_toplevel;

const NAMESPACE: &str = "tree";

fn attach(wire: &mut Wire, request_id: u64) {
    wire.send(&Envelope::request_message(
        NAMESPACE,
        "attach",
        request_id,
        &AttachArgs {
            version: 1,
            mode: String::new(),
        },
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
fn a_tags_and_an_effects_set_travel_the_wire_and_come_back_in_the_change_batch() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);

    let mut ops = create(1000, attached.root);
    ops.push(op::Properties {
        kind: "set".to_owned(),
        node: 1000,
        key: "tags".to_owned(),
        value_kind: "list_text".to_owned(),
        value_texts: vec!["dock".to_owned(), "pinned".to_owned()],
        ..Default::default()
    });
    ops.push(op::Properties {
        kind: "set".to_owned(),
        node: 1000,
        key: "effects".to_owned(),
        value_kind: "list_effect".to_owned(),
        value_effects: vec![effect::Properties {
            kind: "blur".to_owned(),
            downscale: 2,
            passes: 3,
            blur_radius: 12.0,
            tint: "#10203040".to_owned(),
            ..Default::default()
        }],
        ..Default::default()
    });
    transact(&mut wire, 2, 0, 1, ops);
    let changes: ChangesPayload = event(&mut wire, "changes");
    let reply: ReplyPayload = reply(&mut wire, "transact");
    assert!(reply.applied, "the tags and effects sets validate: {reply:?}");

    let tags = changes
        .changes
        .iter()
        .find(|change| change.kind == "set" && change.key == "tags")
        .expect("the tags set is published");
    assert_eq!(tags.value_kind, "list_text");
    assert_eq!(tags.value_texts, vec!["dock".to_owned(), "pinned".to_owned()]);
    let effects = changes
        .changes
        .iter()
        .find(|change| change.kind == "set" && change.key == "effects")
        .expect("the effects set is published");
    assert_eq!(effects.value_kind, "list_effect");
    assert_eq!(effects.value_effects.len(), 1);
    assert_eq!(effects.value_effects[0].kind, "blur");
    assert_eq!(effects.value_effects[0].downscale, 2);
    assert_eq!(effects.value_effects[0].passes, 3);
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_shader_effect_refuses_the_whole_effects_set_bad_value() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);

    let mut ops = create(1000, attached.root);
    ops.push(op::Properties {
        kind: "set".to_owned(),
        node: 1000,
        key: "effects".to_owned(),
        value_kind: "list_effect".to_owned(),
        value_effects: vec![effect::Properties {
            kind: "shader".to_owned(),
            ..Default::default()
        }],
        ..Default::default()
    });
    transact(&mut wire, 2, 0, 1, ops);
    let reply: ReplyPayload = reply(&mut wire, "transact");
    assert!(!reply.applied, "the compositor runs no shader effects");
    assert_eq!(reply.code, "bad-value");
    session.stop().expect("the session stops cleanly");
}

fn set_number(node: u64, key: &str, value_kind: &str, int: i64, uint: u64, double: f64) -> op::Properties {
    op::Properties {
        kind: "set".to_owned(),
        node,
        key: key.to_owned(),
        value_kind: value_kind.to_owned(),
        value_int: int,
        value_uint: uint,
        value_double: double,
        ..Default::default()
    }
}

/// `fps` is a `uint`: the largest value is accepted and published as it is, a negative or a fractional one is
/// `wrong-type` and the View keeps the value it had.
#[test]
fn a_views_fps_is_an_unsigned_count_and_a_negative_or_fractional_one_is_refused() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);

    let mut ops = create_view(1000, attached.root);
    ops.push(set_number(1000, "fps", "uint", 0, u64::MAX, 0.0));
    transact(&mut wire, 2, 0, 1, ops);
    let changes: ChangesPayload = event(&mut wire, "changes");
    let accepted: ReplyPayload = reply(&mut wire, "transact");
    assert!(accepted.applied, "the largest uint is a legal fps: {accepted:?}");
    let fps = changes
        .changes
        .iter()
        .find(|change| change.kind == "set" && change.key == "fps")
        .expect("the fps set is published");
    assert_eq!(fps.value_uint, u64::MAX);

    for (request, op) in [
        (3, set_number(1000, "fps", "int", -1, 0, 0.0)),
        (4, set_number(1000, "fps", "double", 0, 0, 2.5)),
    ] {
        transact(&mut wire, request, 1, 2, vec![op]);
        let refused: ReplyPayload = reply(&mut wire, "transact");
        assert!(!refused.applied, "request {request}: a non-uint fps is refused");
        assert_eq!(refused.code, "wrong-type", "request {request}");
        assert_eq!(refused.serial, 1, "request {request}: the tree is unchanged");
    }
    session.stop().expect("the session stops cleanly");
}

/// The headless session draws with the software renderer, which draws no blur: the node says so in `skipped`.
#[test]
fn a_blur_on_the_software_renderer_is_reported_skipped_by_the_renderer() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);

    let mut ops = create(1000, attached.root);
    ops.push(op::Properties {
        kind: "set".to_owned(),
        node: 1000,
        key: "effects".to_owned(),
        value_kind: "list_effect".to_owned(),
        value_effects: vec![
            effect::Properties {
                kind: "blur".to_owned(),
                downscale: 2,
                passes: 3,
                blur_radius: 12.0,
                tint: "#10203040".to_owned(),
                ..Default::default()
            },
            effect::Properties {
                kind: "shadow".to_owned(),
                offset_y: 4.0,
                blur_radius: 12.0,
                colour: "#00000080".to_owned(),
                ..Default::default()
            },
        ],
        ..Default::default()
    });
    transact(&mut wire, 2, 0, 1, ops);
    // The reply and the change batches interleave with the frame that publishes the skipped list, so read frames
    // until both the reply and the list have arrived.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut replied = false;
    let mut changes: Vec<change::Properties> = Vec::new();
    while !replied
        || !changes
            .iter()
            .any(|change| change.node == 1000 && change.key == "skipped")
    {
        assert!(Instant::now() < deadline, "no skipped list arrived: {changes:?}");
        let envelope = wire.receive().expect("a frame");
        match envelope.member.as_str() {
            "transact" => {
                let reply = envelope.payload_as::<ReplyPayload>().expect("the reply decodes");
                assert!(reply.applied, "the effects validate: {reply:?}");
                replied = true;
            }
            "changes" => changes.extend(
                envelope
                    .payload_as::<ChangesPayload>()
                    .expect("the batch decodes")
                    .changes,
            ),
            other => panic!("unexpected frame `{other}`"),
        }
    }
    let skipped = changes
        .iter()
        .rev()
        .find(|change| change.node == 1000 && change.key == "skipped")
        .expect("the skipped list is published");
    assert_eq!(skipped.value_kind, "list_text");
    assert_eq!(
        skipped.value_texts,
        vec![
            "blur: unsupported-by-renderer".to_owned(),
            "shadow: unsupported-by-renderer".to_owned()
        ],
        "the software renderer draws neither, and a node with effects says so"
    );
    session.stop().expect("the session stops cleanly");
}

/// Create one node of class `canvas` under `parent`.
fn create_canvas(id: u64, parent: u64) -> Vec<op::Properties> {
    let mut ops = create(id, parent);
    ops[0].class = "canvas".to_owned();
    ops
}

/// The bytes of a paint list of one filled rectangle.
fn paint_list() -> Vec<u8> {
    use gnoblin_paint::{PaintList, PaintOp, Rgba8};
    gnoblin_paint::encode(&PaintList {
        ops: vec![PaintOp::Rect {
            x: 0.0,
            y: 0.0,
            width: 8.0,
            height: 8.0,
            radius: 0.0,
            fill: Rgba8::new(10, 20, 30, 255),
            stroke: Rgba8::TRANSPARENT,
            stroke_width: 0.0,
        }],
    })
}

fn set_content(node: u64, bytes: Vec<u8>) -> op::Properties {
    op::Properties {
        kind: "set".to_owned(),
        node,
        key: "content".to_owned(),
        value_kind: "bytes".to_owned(),
        value_bytes: bytes,
        ..Default::default()
    }
}

#[test]
fn a_canvas_paint_list_is_published_as_bytes_and_read_back_in_the_next_snapshot() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);

    let mut ops = create_canvas(1000, attached.root);
    ops.push(set_content(1000, paint_list()));
    transact(&mut wire, 2, 0, 1, ops);
    let changes: ChangesPayload = event(&mut wire, "changes");
    let reply: ReplyPayload = reply(&mut wire, "transact");
    assert!(reply.applied, "a paint list that decodes is accepted: {reply:?}");
    let content = changes
        .changes
        .iter()
        .find(|change| change.kind == "set" && change.key == "content")
        .expect("the content set is published");
    assert_eq!(content.value_kind, "bytes");
    assert_eq!(content.value_bytes, paint_list());

    // A shell that attaches afterwards reads the content in its snapshot.
    drop(wire);
    let (_wire, _attached, nodes) = attach_once_released(&session, 3);
    let canvas = nodes
        .iter()
        .find(|node| node.id == 1000)
        .expect("the canvas is in the snapshot");
    let prop = canvas
        .props
        .iter()
        .find(|prop| prop.key == "content")
        .expect("the content is a property");
    assert_eq!(
        (prop.value_kind.as_str(), prop.value_bytes.as_slice()),
        ("bytes", paint_list().as_slice())
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn f42_canvas_content_that_does_not_decode_is_refused_bad_paint() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);

    let mut ops = create_canvas(1000, attached.root);
    ops.push(set_content(1000, b"definitely not a paint list".to_vec()));
    transact(&mut wire, 2, 0, 1, ops);
    let reply: ReplyPayload = reply(&mut wire, "transact");
    assert!(!reply.applied);
    assert_eq!(reply.code, "bad-paint");
    assert_eq!(reply.failed_op, 2, "the third op is the bad one");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_canvas_measure_is_answered_with_the_size_of_the_text() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);

    let mut ops = create_canvas(1000, attached.root);
    ops.push(op::Properties {
        kind: "call".to_owned(),
        node: 1000,
        method: "measure".to_owned(),
        args: 16.0_f64.to_bits().to_le_bytes().to_vec(),
        text: "sans\nHello".to_owned(),
        token: 77,
        ..Default::default()
    });
    transact(&mut wire, 2, 0, 1, ops);
    let mut answer = None;
    let mut applied = None;
    for _ in 0..4 {
        let envelope = wire.receive().expect("a frame");
        match (envelope.kind.as_ref(), envelope.member.as_str()) {
            (Some(Kind::Event(_)), "answer") => answer = envelope.payload_as::<AnswerPayload>(),
            (Some(Kind::Reply(_)), "transact") => {
                applied = envelope.payload_as::<ReplyPayload>();
                break;
            }
            _ => {}
        }
    }
    assert!(applied.expect("the transaction is answered").applied);
    let answer = answer.expect("the measure is answered");
    assert_eq!(answer.token, 77);
    assert_eq!(answer.error, "");
    let result = String::from_utf8(answer.result).expect("the result is text");
    let fields: Vec<u32> = result
        .split(' ')
        .map(|field| field.parse().expect("whole pixels"))
        .collect();
    let [width, height, ascent] = fields[..] else {
        panic!("`<width> <height> <ascent>`, got {result:?}");
    };
    assert_eq!(height, 24, "a 16 pixel line box is 24 pixels tall");
    assert!(
        ascent > 12 && ascent < 24,
        "the baseline of a 24 pixel line box lies in its lower half: {ascent}"
    );
    assert!(
        width > 20 && width < 80,
        "five letters at 16 pixels are some tens of pixels wide: {width}"
    );
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
        value_kind: "matrix".to_owned(),
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

/// Attaches a holder once the previous one has let go and re-attaches it until the snapshot satisfies `done`: the
/// compositor reaps a detached shell's nodes in a frame of its own, after the close.
fn snapshot_until(
    session: &Session,
    what: &str,
    done: impl Fn(&[snapshot_node::Properties]) -> bool,
) -> (Wire, AttachedPayload, Vec<snapshot_node::Properties>) {
    let (mut wire, mut attached, mut nodes) = attach_once_released(session, 1);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut request_id = 1;
    while !done(&nodes) {
        assert!(Instant::now() < deadline, "{what}: {nodes:?}");
        std::thread::sleep(Duration::from_millis(10));
        request_id += 1;
        (attached, nodes) = reattach_skipping_changes(&mut wire, request_id);
    }
    (wire, attached, nodes)
}

/// Attaches again on this connection, passing over the change batches published meanwhile.
fn reattach_skipping_changes(wire: &mut Wire, request_id: u64) -> (AttachedPayload, Vec<snapshot_node::Properties>) {
    attach(wire, request_id);
    let attached: AttachedPayload = reply_skipping_changes(wire, "attach");
    let mut nodes = Vec::new();
    loop {
        let envelope = wire.receive().expect("a frame");
        if envelope.member != "snapshot" {
            continue;
        }
        let chunk: SnapshotPayload = envelope.payload_as().expect("the chunk decodes");
        nodes.extend(chunk.nodes);
        if chunk.last {
            return (attached, nodes);
        }
    }
}

/// design/node-core.md, section 4.6 step 5: the nodes the old shell made are destroyed with its connection, so the
/// next shell builds on the compositor's tree and not on a second copy of its own.
#[test]
fn a_detached_shells_nodes_are_destroyed_with_its_connection() {
    let session = Session::start();
    let mut first = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut first, 1);

    let mut ops = create(1000, attached.root);
    ops.extend(create(1001, 1000));
    transact(&mut first, 2, 0, 1, ops);
    let _: ChangesPayload = event(&mut first, "changes");
    let _: ReplyPayload = reply(&mut first, "transact");
    drop(first);

    let (_holder, _, nodes) = snapshot_until(&session, "the old shell's nodes were never destroyed", |nodes| {
        !nodes.iter().any(|node| node.id == 1000 || node.id == 1001)
    });
    assert!(
        nodes.iter().any(|node| node.id == attached.root),
        "the compositor's own nodes stay: {nodes:?}"
    );
    session.stop().expect("the session stops cleanly");
}

/// A window the old shell put under one of its nodes goes back under the root, and is not destroyed with the node.
#[test]
fn a_window_the_detached_shell_parented_goes_back_to_the_root_when_its_node_is_destroyed() {
    let session = Session::start();
    let mut client = Client::connect(session.wayland());
    let _window = client.map_window("org.example.held", "Held");
    let mut first = Wire::connect(session.socket());
    let (attached, nodes) = snapshot_holding_window(&mut first, "Held");
    let window = window_titled(&nodes, "Held").expect("the window node").id;

    let mut ops = create(1000, attached.root);
    ops.push(op::Properties {
        kind: "insert".to_owned(),
        node: window,
        parent: 1000,
        index: 0,
        ..Default::default()
    });
    transact(&mut first, 2, attached.serial, attached.serial + 1, ops);
    let applied: ReplyPayload = reply_skipping_changes(&mut first, "transact");
    assert!(
        applied.applied,
        "the shell parents the window under its node: {applied:?}"
    );
    drop(first);

    let (_holder, _, nodes) = snapshot_until(&session, "the old shell's node was never destroyed", |nodes| {
        !nodes.iter().any(|node| node.id == 1000)
    });
    let held = window_titled(&nodes, "Held").expect("the window outlives the node it sat under");
    assert_eq!(held.id, window);
    assert_eq!(held.parent, attached.root, "the window is back under the root");
    session.stop().expect("the session stops cleanly");
}

/// A seat an observer made belongs to the observer's connection, not to the shell: the shell detaching leaves it.
#[test]
fn an_observers_seat_outlives_the_shell_that_detached() {
    let session = Session::start();
    let mut observer = Wire::connect(session.socket());
    let (watching, _) = attach_observer(&mut observer, 1);
    let seat = u64::from(watching.epoch as u32) << 32 | 1;
    transact(&mut observer, 2, 0, 1, unparented_seat(seat, "xr"));
    let made: ReplyPayload = reply_skipping_changes(&mut observer, "transact");
    assert!(made.applied, "the observer makes its seat: {made:?}");

    let mut first = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut first, 1);
    transact(
        &mut first,
        2,
        attached.serial,
        attached.serial + 1,
        create(1000, attached.root),
    );
    let applied: ReplyPayload = reply_skipping_changes(&mut first, "transact");
    assert!(applied.applied, "the shell makes a node: {applied:?}");
    drop(first);

    let (_holder, _, nodes) = snapshot_until(&session, "the old shell's node was never destroyed", |nodes| {
        !nodes.iter().any(|node| node.id == 1000)
    });
    assert!(
        nodes.iter().any(|node| node.id == seat && node.class == "seat"),
        "the observer's seat is still in the tree: {nodes:?}"
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
    let applied: ReplyPayload = reply_skipping_changes(&mut wire, "transact");
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
    let refused: ReplyPayload = reply_skipping_changes(&mut wire, "transact");
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
fn a_seats_name_is_in_the_snapshot_of_a_client_that_attaches_after_the_seat_was_made() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);
    transact(&mut wire, 2, 0, 1, create_seat(1000, attached.root, "xr"));
    let _: ChangesPayload = event(&mut wire, "changes");
    let _: ReplyPayload = reply(&mut wire, "transact");

    let mut later = Wire::connect(session.socket());
    let (_, nodes) = attach_observer(&mut later, 1);
    let seat = nodes
        .iter()
        .find(|node| node.id == 1000)
        .expect("the seat is in the snapshot");
    assert_eq!(seat.class, "seat");
    assert!(
        seat.props
            .iter()
            .any(|prop| prop.key == "name" && prop.value_text == "xr"),
        "the snapshot carries the seat's name, or a restarted client cannot find the seat: {seat:?}"
    );
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

fn pointer_input(node: u64, seat: u64, input_kind: &str, x: f64, y: f64) -> op::Properties {
    op::Properties {
        kind: "input".to_owned(),
        node,
        seat,
        input_kind: input_kind.to_owned(),
        x,
        y,
        ..Default::default()
    }
}

#[test]
fn the_owner_sends_a_pointer_motion_to_a_window_on_a_seat() {
    let session = Session::start();
    let mut client = Client::connect(session.wayland());
    let _window = client.map_window("org.example.window", "Input target");
    let mut wire = Wire::connect(session.socket());
    let (attached, nodes) = snapshot_holding_window(&mut wire, "Input target");
    let window = window_titled(&nodes, "Input target").expect("the window node").id;
    transact(&mut wire, 2, 0, 1, create_seat(1000, attached.root, "xr"));
    let _: ChangesPayload = event(&mut wire, "changes");
    let _: ReplyPayload = reply_skipping_changes(&mut wire, "transact");

    transact(
        &mut wire,
        3,
        1,
        2,
        vec![pointer_input(window, 1000, "motion", 4.0, 5.0)],
    );
    let reply: ReplyPayload = reply_skipping_changes(&mut wire, "transact");
    assert!(reply.applied, "an owner input to a window on a seat applies");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn an_input_to_a_node_that_is_not_a_window_is_refused_bad_value() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);
    transact(&mut wire, 2, 0, 1, create_seat(1000, attached.root, "xr"));
    let _: ChangesPayload = event(&mut wire, "changes");
    let _: ReplyPayload = reply(&mut wire, "transact");

    transact(
        &mut wire,
        3,
        1,
        2,
        vec![pointer_input(attached.root, 1000, "motion", 0.0, 0.0)],
    );
    let reply: ReplyPayload = reply(&mut wire, "transact");
    assert!(!reply.applied, "an input to a non-window node is refused");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn an_input_on_an_unknown_seat_is_refused_bad_value() {
    let session = Session::start();
    let mut client = Client::connect(session.wayland());
    let _window = client.map_window("org.example.window", "Unknown seat");
    let mut wire = Wire::connect(session.socket());
    let (_, nodes) = snapshot_holding_window(&mut wire, "Unknown seat");
    let window = window_titled(&nodes, "Unknown seat").expect("the window node").id;

    transact(
        &mut wire,
        2,
        0,
        1,
        vec![pointer_input(window, 4242, "motion", 0.0, 0.0)],
    );
    let reply: ReplyPayload = reply_skipping_changes(&mut wire, "transact");
    assert!(!reply.applied, "an input on a seat that does not exist is refused");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn moving_the_pointer_to_another_window_leaves_the_first_and_enters_the_second() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);
    transact(&mut wire, 2, 0, 1, create_seat(1000, attached.root, "xr"));
    let _: ChangesPayload = event(&mut wire, "changes");
    let _: ReplyPayload = reply(&mut wire, "transact");

    let mut client = Client::connect(session.wayland());
    client.map_window("org.example.window", "Switch first");
    client.map_window("org.example.window", "Switch second");
    client.pointers_on_every_seat();
    let first = window_node_after_mapping(&mut wire, "Switch first");
    let second = window_node_after_mapping(&mut wire, "Switch second");

    transact(&mut wire, 3, 1, 2, vec![pointer_input(first, 1000, "motion", 4.0, 5.0)]);
    let _: ReplyPayload = reply_skipping_changes(&mut wire, "transact");
    transact(
        &mut wire,
        4,
        2,
        3,
        vec![pointer_input(second, 1000, "motion", 6.0, 7.0)],
    );
    let _: ReplyPayload = reply_skipping_changes(&mut wire, "transact");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        client.roundtrip();
        let xr: Vec<support::PointerEvent> = client
            .pointer_events()
            .iter()
            .filter(|(seat, _)| seat == "xr")
            .map(|(_, event)| event.clone())
            .collect();
        let leave_first = xr.iter().position(|e| matches!(e, support::PointerEvent::Leave { .. }));
        let enter_second = xr
            .iter()
            .position(|e| matches!(e, support::PointerEvent::Enter { x, y, .. } if *x == 6.0 && *y == 7.0));
        if let (Some(leave), Some(enter)) = (leave_first, enter_second) {
            assert!(
                leave < enter,
                "the leave on the first window precedes the enter on the second: {xr:?}"
            );
            break;
        }
        assert!(Instant::now() < deadline, "no leave then enter on seat xr: {xr:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_pointer_input_from_a_connection_that_is_not_the_owner_is_refused() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    transact(&mut wire, 1, 0, 1, vec![pointer_input(1000, 1001, "motion", 0.0, 0.0)]);
    let reply: ReplyPayload = reply(&mut wire, "transact");
    assert!(!reply.applied);
    assert_eq!(reply.code, "not-privileged");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_pointer_input_reaches_the_window_client_on_the_xr_seat_only() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);
    transact(&mut wire, 2, 0, 1, create_seat(1000, attached.root, "xr"));
    let _: ChangesPayload = event(&mut wire, "changes");
    let _: ReplyPayload = reply(&mut wire, "transact");

    // The xr seat is advertised before the client connects, so the client binds it.
    let mut client = Client::connect(session.wayland());
    client.map_window("org.example.window", "Seat delivery");
    client.pointers_on_every_seat();
    let node = window_node_after_mapping(&mut wire, "Seat delivery");

    transact(&mut wire, 3, 1, 2, vec![pointer_input(node, 1000, "motion", 4.0, 5.0)]);
    let _: ReplyPayload = reply_skipping_changes(&mut wire, "transact");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        client.roundtrip();
        let entered = client.pointer_events().iter().any(|(seat, event)| {
            seat == "xr" && matches!(event, support::PointerEvent::Enter { x, y, .. } if *x == 4.0 && *y == 5.0)
        });
        if entered {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no enter on seat xr: {:?}",
            client.pointer_events()
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    let press = op::Properties {
        input_kind: "button".to_owned(),
        button: 272,
        state: 1,
        ..pointer_input(node, 1000, "button", 4.0, 5.0)
    };
    let release = op::Properties {
        state: 0,
        ..press.clone()
    };
    let scroll = op::Properties {
        dy: 3.0,
        ..pointer_input(node, 1000, "scroll", 4.0, 5.0)
    };
    transact(&mut wire, 4, 2, 3, vec![press, release, scroll]);
    let _: ReplyPayload = reply_skipping_changes(&mut wire, "transact");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        client.roundtrip();
        let events = client.pointer_events();
        let pressed = events.iter().any(|(seat, e)| {
            seat == "xr"
                && *e
                    == support::PointerEvent::Button {
                        button: 272,
                        pressed: true,
                    }
        });
        let released = events.iter().any(|(seat, e)| {
            seat == "xr"
                && *e
                    == support::PointerEvent::Button {
                        button: 272,
                        pressed: false,
                    }
        });
        let scrolled = events.iter().any(|(seat, e)| {
            seat == "xr" && matches!(e, support::PointerEvent::Axis { vertical: true, value } if *value > 0.0)
        });
        if pressed && released && scrolled {
            break;
        }
        assert!(Instant::now() < deadline, "no button or axis on seat xr: {events:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        client.pointer_events().iter().all(|(seat, _)| seat == "xr"),
        "the main seat pointer focus moved: {:?}",
        client.pointer_events()
    );
    session.stop().expect("the session stops cleanly");
}

/// The next Reply frame to `member`, skipping the change batches the compositor publishes
/// meanwhile.
fn reply_skipping_changes<M: prost::Message + Default>(wire: &mut Wire, member: &str) -> M {
    loop {
        let envelope = wire.receive().expect("a frame");
        if matches!(envelope.kind, Some(Kind::Reply(_))) {
            assert_eq!(envelope.member, member);
            return envelope.payload_as::<M>().expect("the payload decodes");
        }
    }
}

/// Sets the main seat's `focus` to the window node `target`, or clears it with `0`, and
/// checks the transaction applied.
fn focus_seat(wire: &mut Wire, request_id: u64, base_serial: u64, seat: u64, target: u64) {
    transact(
        wire,
        request_id,
        base_serial,
        base_serial + 1,
        vec![set_node_ref(seat, "focus", target)],
    );
    let reply: ReplyPayload = reply_skipping_changes(wire, "transact");
    assert!(reply.applied, "the seat focus write applies");
}

/// Reads the keyboard focus changes after `from` until the last of them satisfies `done`.
fn keyboard_changes_until(
    client: &mut Client,
    from: usize,
    done: impl Fn(&[support::KeyboardFocus]) -> bool,
) -> Vec<support::KeyboardFocus> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        client.roundtrip();
        let changes = client.keyboard_focus()[from..].to_vec();
        if done(&changes) {
            return changes;
        }
        assert!(Instant::now() < deadline, "the keyboard focus never moved: {changes:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_shell_write_of_the_seat_focus_moves_the_keyboard_to_that_window() {
    use support::KeyboardFocus::{Enter, Leave};

    let session = Session::start();
    let mut client = Client::connect(session.wayland());
    let first = client.map_window("org.example.window", "Focus one");
    let second = client.map_window("org.example.window", "Focus two");
    let one_surface = first.surface.id().protocol_id();
    let two_surface = second.surface.id().protocol_id();
    let mut wire = Wire::connect(session.socket());
    snapshot_holding_window(&mut wire, "Focus one");
    let (attached, nodes) = snapshot_holding_window(&mut wire, "Focus two");
    let seat = nodes
        .iter()
        .find(|node| node.class == "seat")
        .expect("the main seat node")
        .id;
    let one = window_titled(&nodes, "Focus one").expect("the first window node").id;
    let two = window_titled(&nodes, "Focus two").expect("the second window node").id;

    focus_seat(&mut wire, 2, attached.serial, seat, one);
    keyboard_changes_until(&mut client, 0, |changes| {
        changes.last() == Some(&Enter { surface: one_surface })
    });

    let mark = client.keyboard_focus().len();
    focus_seat(&mut wire, 3, attached.serial + 1, seat, two);
    let moved = keyboard_changes_until(&mut client, mark, |changes| {
        changes.last() == Some(&Enter { surface: two_surface })
    });
    assert_eq!(
        moved,
        vec![Leave { surface: one_surface }, Enter { surface: two_surface }],
        "the first window leaves before the second enters"
    );

    let mark = client.keyboard_focus().len();
    focus_seat(&mut wire, 4, attached.serial + 2, seat, 0);
    let cleared = keyboard_changes_until(&mut client, mark, |changes| {
        changes.last() == Some(&Leave { surface: two_surface })
    });
    assert_eq!(
        cleared,
        vec![Leave { surface: two_surface }],
        "a null focus clears the keyboard"
    );
    session.stop().expect("the session stops cleanly");
}

/// The id of the window titled `title`, read from re-attached snapshots; stray change batches
/// before each attach reply are skipped.
fn window_node_after_mapping(wire: &mut Wire, title: &str) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut request_id = 1;
    loop {
        attach(wire, request_id);
        let attached: AttachedPayload = reply_skipping_changes(wire, "attach");
        let nodes = snapshot_nodes(wire, attached.serial);
        if let Some(node) = window_titled(&nodes, title) {
            return node.id;
        }
        assert!(
            Instant::now() < deadline,
            "no window titled `{title}` in the tree: {nodes:?}"
        );
        request_id += 1;
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn set_bool(node: u64, key: &str, value: bool) -> op::Properties {
    op::Properties {
        kind: "set".to_owned(),
        node,
        key: key.to_owned(),
        value_kind: "bool".to_owned(),
        value_bool: value,
        ..Default::default()
    }
}

fn set_list(node: u64, key: &str, value: Vec<f64>) -> op::Properties {
    op::Properties {
        kind: "set".to_owned(),
        node,
        key: key.to_owned(),
        value_kind: "list_double".to_owned(),
        value_list: value,
        ..Default::default()
    }
}

fn set_node_ref(node: u64, key: &str, target: u64) -> op::Properties {
    op::Properties {
        kind: "set".to_owned(),
        node,
        key: key.to_owned(),
        value_kind: "node".to_owned(),
        value_node: target,
        ..Default::default()
    }
}

/// Create one `view` node under `parent`, with the shell's pre-allocated `id`.
fn create_view(id: u64, parent: u64) -> Vec<op::Properties> {
    vec![
        op::Properties {
            kind: "new".to_owned(),
            class: "view".to_owned(),
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

/// The snapshot's entry for property `key` of node `id`, if the snapshot carries one.
fn prop_of<'a>(nodes: &'a [snapshot_node::Properties], id: u64, key: &str) -> Option<&'a change::Properties> {
    nodes
        .iter()
        .find(|node| node.id == id)?
        .props
        .iter()
        .find(|prop| prop.key == key)
}

#[test]
fn a_fresh_attach_carries_the_current_value_of_every_non_default_property() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);

    let mut ops = create(1000, attached.root);
    ops.push(set_double(1000, "opacity", 0.5));
    ops.push(set_bool(1000, "visible", false));
    ops.push(set_list(1000, "rotation", vec![0.0, 0.0, 45.0]));
    ops.push(set_text(1000, "name", "hero"));
    ops.push(set_double(1000, "width", 0.0));
    ops.extend(create_view(1001, attached.root));
    ops.push(set_node_ref(1001, "source", 1000));
    transact(&mut wire, 2, 0, 1, ops);
    let _: ChangesPayload = event(&mut wire, "changes");
    let applied: ReplyPayload = reply(&mut wire, "transact");
    assert!(applied.applied, "the sets validate: {applied:?}");

    let (_, nodes) = attach_and_snapshot(&mut wire, 3);
    let opacity = prop_of(&nodes, 1000, "opacity").expect("a non-default double is carried");
    assert_eq!(opacity.value_kind, "double");
    assert_eq!(opacity.value_double, 0.5);
    let visible = prop_of(&nodes, 1000, "visible").expect("a non-default bool is carried");
    assert_eq!(visible.value_kind, "bool");
    assert!(!visible.value_bool);
    let rotation = prop_of(&nodes, 1000, "rotation").expect("a non-default list is carried");
    assert_eq!(rotation.value_kind, "list_double");
    assert_eq!(rotation.value_list, vec![0.0, 0.0, 45.0]);
    let name = prop_of(&nodes, 1000, "name").expect("a non-default text is carried");
    assert_eq!(name.value_kind, "text");
    assert_eq!(name.value_text, "hero");
    let source = prop_of(&nodes, 1001, "source").expect("a node reference is carried");
    assert_eq!(source.value_kind, "node");
    assert_eq!(source.value_node, 1000);

    for key in ["width", "height", "x", "scale", "clip", "tags"] {
        assert!(
            prop_of(&nodes, 1000, key).is_none(),
            "a property at its default is not carried: {key}"
        );
    }
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_value_set_back_to_its_default_is_no_longer_carried() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);

    let mut ops = create(1000, attached.root);
    ops.push(set_double(1000, "opacity", 0.5));
    transact(&mut wire, 2, 0, 1, ops);
    let _: ChangesPayload = event(&mut wire, "changes");
    let _: ReplyPayload = reply(&mut wire, "transact");

    transact(&mut wire, 3, 1, 2, vec![set_double(1000, "opacity", 1.0)]);
    let _: ChangesPayload = event(&mut wire, "changes");
    let applied: ReplyPayload = reply(&mut wire, "transact");
    assert!(applied.applied, "the set validates: {applied:?}");

    let (_, nodes) = attach_and_snapshot(&mut wire, 4);
    assert!(
        prop_of(&nodes, 1000, "opacity").is_none(),
        "opacity back at its default is not carried"
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_value_set_after_the_snapshot_is_published_once_and_not_repeated_in_it() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, _) = attach_and_snapshot(&mut wire, 1);
    let mut ops = create(1000, attached.root);
    ops.push(set_double(1000, "opacity", 0.5));
    transact(&mut wire, 2, 0, 1, ops);
    let _: ChangesPayload = event(&mut wire, "changes");
    let _: ReplyPayload = reply(&mut wire, "transact");

    let (attached, nodes) = attach_and_snapshot(&mut wire, 3);
    assert_eq!(attached.serial, 1, "the snapshot is at the serial the value was set");
    assert_eq!(
        prop_of(&nodes, 1000, "opacity").map(|prop| prop.value_double),
        Some(0.5),
        "the snapshot holds the value at its serial"
    );

    transact(&mut wire, 4, 1, 2, vec![set_double(1000, "opacity", 0.25)]);
    let changes = changes_until(&mut wire, |all| {
        all.iter().any(|change| change.kind == "set" && change.key == "opacity")
    });
    let applied: ReplyPayload = reply(&mut wire, "transact");
    assert!(applied.applied, "the set validates: {applied:?}");
    let opacity: Vec<&change::Properties> = changes
        .iter()
        .filter(|change| change.kind == "set" && change.key == "opacity")
        .collect();
    assert_eq!(opacity.len(), 1, "the new value is published once: {changes:?}");
    assert_eq!(opacity[0].value_double, 0.25);
    session.stop().expect("the session stops cleanly");
}

// The shell tells a client how big it is and how it is placed by writing its window node: `width` and `height` are
// the configure size, and `tiled` is the four tiled edge states with the work area as the bounds
// (design/node-api-refactor.md, the `Window` members table). These tests read what the client receives.

/// Sends a transaction of `ops` and returns the serial it was applied at, waiting for the compositor's reply.
fn apply(wire: &mut Wire, request_id: u64, base_serial: u64, ops: Vec<op::Properties>) -> u64 {
    transact(wire, request_id, base_serial, base_serial + 1, ops);
    let applied: ReplyPayload = reply_skipping_changes(wire, "transact");
    assert!(applied.applied, "the shell's write applies: {applied:?}");
    applied.serial
}

/// The client's view of a window after the shell's writes: its size, states and bounds.
type Configured = ((i32, i32), Vec<xdg_toplevel::State>, Option<(i32, i32)>);

/// What the client last received for `window`.
fn configured(client: &mut Client, window: &support::Window) -> Configured {
    client.roundtrip();
    (
        client.configured_size(window).unwrap_or_default(),
        client.configured_states(window).map(<[_]>::to_vec).unwrap_or_default(),
        client.configured_bounds(window),
    )
}

/// Waits until the client's last configure has `size`.
fn wait_for_size(client: &mut Client, window: &support::Window, size: (i32, i32)) {
    support::wait_for(&format!("a configure of {size:?}"), || {
        (configured(client, window).0 == size).then_some(())
    });
}

const TILED_EDGES: [xdg_toplevel::State; 4] = [
    xdg_toplevel::State::TiledLeft,
    xdg_toplevel::State::TiledRight,
    xdg_toplevel::State::TiledTop,
    xdg_toplevel::State::TiledBottom,
];

#[test]
fn a_shell_write_of_a_windows_size_reaches_its_client_as_a_configure() {
    let session = Session::start();
    let mut client = Client::connect(session.wayland());
    let window = client.map_window("org.example.window", "Sized");
    let mut wire = Wire::connect(session.socket());
    let (attached, nodes) = snapshot_holding_window(&mut wire, "Sized");
    let node = window_titled(&nodes, "Sized").expect("the window node").id;

    apply(
        &mut wire,
        2,
        attached.serial,
        vec![set_double(node, "width", 400.0), set_double(node, "height", 300.0)],
    );

    wait_for_size(&mut client, &window, (400, 300));
    let (_, states, bounds) = configured(&mut client, &window);
    assert!(
        TILED_EDGES.iter().all(|edge| !states.contains(edge)),
        "a window the shell did not tile has no tiled edge: {states:?}"
    );
    // A configure_bounds of 0 by 0 is the protocol's "no bounds".
    assert!(
        bounds.is_none_or(|bounds| bounds == (0, 0)),
        "a window the shell did not tile is sent no bounds: {bounds:?}"
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_shell_write_of_tiled_sends_the_four_tiled_edges_and_the_work_area_as_bounds() {
    let session = Session::start();
    let mut client = Client::connect(session.wayland());
    let window = client.map_window("org.example.window", "Tiled");
    let mut wire = Wire::connect(session.socket());
    let (attached, nodes) = snapshot_holding_window(&mut wire, "Tiled");
    let node = window_titled(&nodes, "Tiled").expect("the window node").id;

    apply(
        &mut wire,
        2,
        attached.serial,
        vec![
            set_double(node, "width", 500.0),
            set_double(node, "height", 400.0),
            set_bool(node, "tiled", true),
        ],
    );

    wait_for_size(&mut client, &window, (500, 400));
    let (_, states, bounds) = configured(&mut client, &window);
    for edge in TILED_EDGES {
        assert!(states.contains(&edge), "the tiled window is told {edge:?}: {states:?}");
    }
    // Nothing reserves space on the session's one output, so its work area is the screen the shell reads.
    let screen = nodes
        .iter()
        .find(|node| node.class == "screen")
        .expect("the session has a screen node")
        .id;
    let screen_side = |key: &str| {
        prop_of(&nodes, screen, key)
            .expect("the screen has its size")
            .value_double as i32
    };
    assert_eq!(
        bounds,
        Some((screen_side("width"), screen_side("height"))),
        "a tiled window is sent the work area as its bounds"
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_shell_write_of_tiled_false_takes_the_tiled_edges_off_again() {
    let session = Session::start();
    let mut client = Client::connect(session.wayland());
    let window = client.map_window("org.example.window", "Untiled");
    let mut wire = Wire::connect(session.socket());
    let (attached, nodes) = snapshot_holding_window(&mut wire, "Untiled");
    let node = window_titled(&nodes, "Untiled").expect("the window node").id;

    let serial = apply(
        &mut wire,
        2,
        attached.serial,
        vec![
            set_double(node, "width", 500.0),
            set_double(node, "height", 400.0),
            set_bool(node, "tiled", true),
        ],
    );
    wait_for_size(&mut client, &window, (500, 400));
    assert!(
        configured(&mut client, &window)
            .1
            .contains(&xdg_toplevel::State::TiledLeft),
        "the window is tiled first"
    );

    apply(
        &mut wire,
        3,
        serial,
        vec![
            set_double(node, "width", 320.0),
            set_double(node, "height", 240.0),
            set_bool(node, "tiled", false),
        ],
    );
    wait_for_size(&mut client, &window, (320, 240));
    let (_, states, _) = configured(&mut client, &window);
    assert!(
        TILED_EDGES.iter().all(|edge| !states.contains(edge)),
        "a window the shell untiled has no tiled edge: {states:?}"
    );
    session.stop().expect("the session stops cleanly");
}

// ---- Observers (design/node-core.md, F2 and F2b) ----------------------------------------------

/// Attaches as an observer: it reads the tree and may only make its own seats and send input.
fn attach_observer(wire: &mut Wire, request_id: u64) -> (AttachedPayload, Vec<snapshot_node::Properties>) {
    wire.send(&Envelope::request_message(
        NAMESPACE,
        "attach",
        request_id,
        &AttachArgs {
            version: 1,
            mode: "observer".to_owned(),
        },
    ));
    let attached: AttachedPayload = reply(wire, "attach");
    let nodes = snapshot_nodes(wire, attached.serial);
    (attached, nodes)
}

fn destroy_op(node: u64) -> op::Properties {
    op::Properties {
        kind: "destroy".to_owned(),
        node,
        ..Default::default()
    }
}

/// A seat made the way the XR extension makes it: new, then its name, and no parent.
fn unparented_seat(id: u64, name: &str) -> Vec<op::Properties> {
    vec![
        op::Properties {
            kind: "new".to_owned(),
            class: "seat".to_owned(),
            id,
            ..Default::default()
        },
        set_text(id, "name", name),
    ]
}

#[test]
fn an_observer_attaches_while_the_shell_holds_the_tree_and_hears_its_changes() {
    let session = Session::start();
    let mut shell = Wire::connect(session.socket());
    let (held, _) = attach_and_snapshot(&mut shell, 1);
    transact(&mut shell, 2, 0, 1, create(1000, held.root));
    let _: ChangesPayload = event(&mut shell, "changes");
    let _: ReplyPayload = reply(&mut shell, "transact");

    let mut observer = Wire::connect(session.socket());
    let (attached, nodes) = attach_observer(&mut observer, 1);
    assert_ne!(attached.epoch, held.epoch, "an observer has an epoch of its own");
    assert_eq!(attached.serial, 1);
    assert!(
        nodes.iter().any(|node| node.id == 1000),
        "the observer's snapshot holds what the shell made: {nodes:?}"
    );

    transact(&mut shell, 3, 1, 2, create(1001, held.root));
    let from_shell: ChangesPayload = event(&mut shell, "changes");
    let _: ReplyPayload = reply(&mut shell, "transact");
    let heard: ChangesPayload = event(&mut observer, "changes");
    assert_eq!(heard.serial, from_shell.serial, "the observer hears the same batch");
    assert!(heard.changes.iter().any(|c| c.kind == "created" && c.node == 1001));
    session.stop().expect("the session stops cleanly");
}

#[test]
fn any_number_of_observers_attach_and_a_shell_still_attaches_after_them() {
    let session = Session::start();
    let mut first = Wire::connect(session.socket());
    let (a, _) = attach_observer(&mut first, 1);
    let mut second = Wire::connect(session.socket());
    let (b, _) = attach_observer(&mut second, 1);
    assert_ne!(a.epoch, b.epoch);

    let mut shell = Wire::connect(session.socket());
    let (held, _) = attach_and_snapshot(&mut shell, 1);
    assert_ne!(held.epoch, a.epoch);
    assert_ne!(held.epoch, b.epoch);

    let mut again = Wire::connect(session.socket());
    attach(&mut again, 1);
    assert_eq!(
        again.receive().expect("an error frame").error_code(),
        Some("shell-already-attached"),
        "a second holder is still refused (F2)"
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn an_observer_is_refused_a_structural_op_and_the_whole_transaction_with_it() {
    let session = Session::start();
    let mut shell = Wire::connect(session.socket());
    let (held, _) = attach_and_snapshot(&mut shell, 1);
    let mut observer = Wire::connect(session.socket());
    let (attached, _) = attach_observer(&mut observer, 1);
    let id = u64::from(attached.epoch as u32) << 32 | 1;

    // The seat is allowed; the node after it is not. Nothing of the transaction lands.
    let mut ops = unparented_seat(id, "xr");
    ops.extend(create(id + 1, held.root));
    transact(&mut observer, 2, 0, 1, ops);
    let refused: ReplyPayload = reply(&mut observer, "transact");
    assert!(!refused.applied);
    assert_eq!(refused.code, "not-holder");
    assert_eq!(
        refused.failed_op, 2,
        "the refusal names the first op the observer may not send"
    );

    for forbidden in [
        vec![set_double(held.root, "x", 1.0)],
        vec![destroy_op(id)],
        create(id + 2, held.root)[1..].to_vec(),
    ] {
        transact(&mut observer, 3, 0, 2, forbidden);
        let refused: ReplyPayload = reply(&mut observer, "transact");
        assert_eq!(refused.code, "not-holder");
    }

    let mut later = Wire::connect(session.socket());
    let (_, nodes) = attach_observer(&mut later, 1);
    assert!(
        nodes.iter().all(|node| node.id != id && node.id != id + 1),
        "a refused transaction changes nothing: {nodes:?}"
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn an_observer_makes_a_seat_and_the_shell_and_the_observer_hear_of_it() {
    let session = Session::start();
    let mut shell = Wire::connect(session.socket());
    attach_and_snapshot(&mut shell, 1);
    let mut observer = Wire::connect(session.socket());
    let (attached, _) = attach_observer(&mut observer, 1);
    let id = u64::from(attached.epoch as u32) << 32 | 1;

    transact(&mut observer, 2, 0, 0, unparented_seat(id, "xr"));
    let applied: ReplyPayload = reply_skipping_changes(&mut observer, "transact");
    assert!(
        applied.applied,
        "an observer's new seat with its name applies: {applied:?}"
    );

    let heard = changes_until(&mut shell, |all| {
        all.iter().any(|c| c.kind == "created" && c.node == id)
            && all.iter().any(|c| c.kind == "set" && c.node == id && c.key == "name")
    });
    assert!(
        heard
            .iter()
            .any(|c| c.kind == "set" && c.node == id && c.key == "name" && c.value_text == "xr"),
        "the shell hears the seat's name: {heard:?}"
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn an_observer_sends_pointer_input_on_its_seat_to_a_window() {
    let session = Session::start();
    let mut shell = Wire::connect(session.socket());
    attach_and_snapshot(&mut shell, 1);
    let mut observer = Wire::connect(session.socket());
    let (attached, _) = attach_observer(&mut observer, 1);
    let id = u64::from(attached.epoch as u32) << 32 | 1;
    transact(&mut observer, 2, 0, 0, unparented_seat(id, "xr"));
    let made: ReplyPayload = reply_skipping_changes(&mut observer, "transact");
    assert!(made.applied, "{made:?}");

    // The seat is advertised before the client connects, so the client binds it.
    let mut client = Client::connect(session.wayland());
    client.map_window("org.example.window", "Observed target");
    client.pointers_on_every_seat();
    let window = window_node_after_mapping(&mut shell, "Observed target");

    transact(
        &mut observer,
        3,
        made.serial,
        0,
        vec![pointer_input(window, id, "motion", 4.0, 5.0)],
    );
    let sent: ReplyPayload = reply_skipping_changes(&mut observer, "transact");
    assert!(sent.applied, "an observer's input applies: {sent:?}");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        client.roundtrip();
        let entered = client.pointer_events().iter().any(|(seat, event)| {
            seat == "xr" && matches!(event, support::PointerEvent::Enter { x, y, .. } if *x == 4.0 && *y == 5.0)
        });
        if entered {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no enter on seat xr: {:?}",
            client.pointer_events()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    session.stop().expect("the session stops cleanly");
}

#[test]
fn an_observer_destroys_its_own_seat_and_not_another_connections() {
    let session = Session::start();
    let mut shell = Wire::connect(session.socket());
    let (held, _) = attach_and_snapshot(&mut shell, 1);
    transact(&mut shell, 2, 0, 1, create_seat(1000, held.root, "shell-seat"));
    let _: ChangesPayload = event(&mut shell, "changes");
    let _: ReplyPayload = reply(&mut shell, "transact");

    let mut observer = Wire::connect(session.socket());
    let (attached, _) = attach_observer(&mut observer, 1);
    let id = u64::from(attached.epoch as u32) << 32 | 1;
    transact(&mut observer, 2, 0, 0, unparented_seat(id, "mine"));
    let made: ReplyPayload = reply_skipping_changes(&mut observer, "transact");
    assert!(made.applied, "{made:?}");

    transact(&mut observer, 3, made.serial, 0, vec![destroy_op(1000)]);
    let refused: ReplyPayload = reply_skipping_changes(&mut observer, "transact");
    assert_eq!(
        refused.code, "not-holder",
        "a seat the observer did not make is not its to destroy"
    );

    transact(&mut observer, 4, made.serial, 0, vec![destroy_op(id)]);
    let destroyed: ReplyPayload = reply_skipping_changes(&mut observer, "transact");
    assert!(destroyed.applied, "{destroyed:?}");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn an_observers_seats_are_destroyed_when_it_disconnects() {
    let session = Session::start();
    let mut shell = Wire::connect(session.socket());
    attach_and_snapshot(&mut shell, 1);
    let mut observer = Wire::connect(session.socket());
    let (attached, _) = attach_observer(&mut observer, 1);
    let id = u64::from(attached.epoch as u32) << 32 | 1;
    transact(&mut observer, 2, 0, 0, unparented_seat(id, "xr"));
    let made: ReplyPayload = reply_skipping_changes(&mut observer, "transact");
    assert!(made.applied, "{made:?}");
    changes_until(&mut shell, |all| {
        all.iter().any(|c| c.kind == "created" && c.node == id)
    });

    drop(observer);
    let gone = changes_until(&mut shell, |all| {
        all.iter().any(|c| c.kind == "destroyed" && c.node == id)
    });
    assert!(gone.iter().any(|c| c.kind == "destroyed" && c.node == id));
    session.stop().expect("the session stops cleanly");
}

#[test]
fn an_unknown_attach_mode_is_malformed() {
    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    wire.send(&Envelope::request_message(
        NAMESPACE,
        "attach",
        1,
        &AttachArgs {
            version: 1,
            mode: "spectator".to_owned(),
        },
    ));
    let refusal = wire.receive().expect("an error frame");
    assert!(refusal.error_code().is_some(), "{refusal:?}");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn an_observer_is_refused_a_call_a_second_seat_of_the_same_name_and_input_on_a_seat_it_did_not_make() {
    let session = Session::start();
    let mut shell = Wire::connect(session.socket());
    let (held, _) = attach_and_snapshot(&mut shell, 1);
    transact(&mut shell, 2, 0, 1, create_seat(1000, held.root, "shell-seat"));
    let _: ChangesPayload = event(&mut shell, "changes");
    let _: ReplyPayload = reply(&mut shell, "transact");

    let mut observer = Wire::connect(session.socket());
    let (attached, _) = attach_observer(&mut observer, 1);
    let id = u64::from(attached.epoch as u32) << 32 | 1;
    transact(&mut observer, 2, 0, 0, unparented_seat(id, "xr"));
    let made: ReplyPayload = reply_skipping_changes(&mut observer, "transact");
    assert!(made.applied, "{made:?}");

    // A call on any node, even its own seat.
    let call = op::Properties {
        kind: "call".to_owned(),
        node: id,
        method: "claim".to_owned(),
        ..Default::default()
    };
    transact(&mut observer, 3, made.serial, 0, vec![call]);
    let refused: ReplyPayload = reply_skipping_changes(&mut observer, "transact");
    assert_eq!(refused.code, "not-holder", "a call is the holder's");

    // Input on the shell's seat, which the observer did not make: refused whole.
    transact(
        &mut observer,
        4,
        made.serial,
        0,
        vec![pointer_input(1, 1000, "motion", 1.0, 1.0)],
    );
    let refused: ReplyPayload = reply_skipping_changes(&mut observer, "transact");
    assert_eq!(
        refused.code, "not-holder",
        "input goes out only on a seat the observer made"
    );

    // A second seat of a name that exists is refused and nothing of the transaction lands.
    let mut ops = unparented_seat(id + 1, "xr");
    ops.extend(unparented_seat(id + 2, "fresh"));
    transact(&mut observer, 5, made.serial, 0, ops);
    let duplicate: ReplyPayload = reply_skipping_changes(&mut observer, "transact");
    assert!(!duplicate.applied, "a second seat named xr is refused: {duplicate:?}");
    assert_eq!(duplicate.failed_op, 1, "the refusal names the duplicate's name");

    let mut later = Wire::connect(session.socket());
    let (_, nodes) = attach_observer(&mut later, 1);
    assert!(
        nodes.iter().all(|node| node.id != id + 1 && node.id != id + 2),
        "a refused transaction leaves no seat behind: {nodes:?}"
    );
    session.stop().expect("the session stops cleanly");
}

#[test]
fn an_observer_does_not_become_the_holder_when_the_holder_detaches() {
    let session = Session::start();
    let mut shell = Wire::connect(session.socket());
    let (held, _) = attach_and_snapshot(&mut shell, 1);
    let mut observer = Wire::connect(session.socket());
    let (attached, _) = attach_observer(&mut observer, 1);

    drop(shell);
    // The observer still has no hold: a structural op is refused, and a shell can take the tree.
    let (mut next, taken, _) = attach_once_released(&session, 1);
    assert_ne!(taken.epoch, held.epoch);
    transact(
        &mut observer,
        2,
        0,
        0,
        create(u64::from(attached.epoch as u32) << 32 | 9, taken.root),
    );
    let refused: ReplyPayload = reply_skipping_changes(&mut observer, "transact");
    assert_eq!(
        refused.code, "not-holder",
        "an observer stays an observer after the holder is gone"
    );

    transact(&mut next, 2, 0, 1, create(1000, taken.root));
    let applied: ReplyPayload = reply_skipping_changes(&mut next, "transact");
    assert!(applied.applied, "the new shell holds the tree: {applied:?}");
    session.stop().expect("the session stops cleanly");
}

/// `KEY_A` and `KEY_B` from `linux/input-event-codes.h`.
const KEY_A: u32 = 30;
const KEY_B: u32 = 48;

/// The `key` events the shell is sent within `window`, in order, as `(node, key, pressed)`. Every other frame (a change
/// batch, the lock's `requested`) is read and skipped.
fn key_events_within(wire: &mut Wire, window: Duration, want: usize) -> Vec<(u64, String, bool, String)> {
    wire.set_read_timeout(Duration::from_millis(100));
    let deadline = Instant::now() + window;
    let mut keys = Vec::new();
    while Instant::now() < deadline && keys.len() < want {
        let Some(envelope) = wire.receive_within() else {
            continue;
        };
        if envelope.member != "event" {
            continue;
        }
        let event: EventPayload = envelope.payload_as().expect("the payload decodes");
        if event.kind == "key" {
            keys.push((event.node, event.key, event.pressed, event.text));
        }
    }
    keys
}

/// Reads frames until the lock node's `requested` event arrives, which says the compositor took the lock request.
fn wait_for_lock_request(wire: &mut Wire) {
    wire.set_read_timeout(Duration::from_millis(100));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "the lock request never reached the shell");
        let Some(envelope) = wire.receive_within() else {
            continue;
        };
        if envelope.member != "event" {
            continue;
        }
        let event: EventPayload = envelope.payload_as().expect("the payload decodes");
        if event.kind == "requested" {
            return;
        }
    }
}

/// A key from the real input path, an EI client's keyboard, reaches the shell as an `event` of kind `key` on the node
/// that holds the seat focus, press then release; and once the session is locked, a key emits nothing. The session
/// lock's cover owns every key, so none reaches the shell's node.
#[test]
fn a_key_reaches_the_shell_node_holding_the_focus_and_nothing_reaches_it_while_locked() {
    use gnoblin_input_eis::Caps;
    use support::ei::EiClient;

    let session = Session::start();
    let mut wire = Wire::connect(session.socket());
    let (attached, nodes) = attach_and_snapshot(&mut wire, 1);
    let seat = nodes
        .iter()
        .find(|node| node.class == "seat")
        .expect("the main seat node")
        .id;
    // The shell makes a node that takes input (`input` is true by default) and holds the seat's focus on it.
    let mut ops = create(1000, attached.root);
    ops.push(set_node_ref(seat, "focus", 1000));
    transact(&mut wire, 2, attached.serial, attached.serial + 1, ops);
    let applied: ReplyPayload = reply_skipping_changes(&mut wire, "transact");
    assert!(applied.applied, "the shell's node takes the seat focus: {applied:?}");

    // The keyboard is the real input path: an EI client the session grants the keyboard to.
    let socket = session.directory().join("eis.sock");
    let mut remote = EiClient::connect(&socket);
    remote.wait_for("the seat", |client| client.seat.is_some());
    assert!(session.eis().grant(Caps::KEYBOARD), "the endpoint is running");
    remote.wait_for("the keyboard capability", |client| {
        client.capabilities & u64::from(Caps::KEYBOARD.bits()) != 0
    });
    remote.bind_keyboard();
    remote.wait_for("the keyboard device", |client| {
        client
            .devices
            .first()
            .is_some_and(|device| device.done && device.resumed.is_some())
    });

    remote.key(KEY_A, true);
    remote.key(KEY_A, false);
    let unlocked = key_events_within(&mut wire, Duration::from_secs(5), 2);
    assert_eq!(
        unlocked,
        vec![
            (1000, "a".to_owned(), true, "a".to_owned()),
            (1000, "a".to_owned(), false, "a".to_owned())
        ],
        "the key reaches the shell as an event on the focused node, press then release"
    );

    // The session locks: the compositor tells the shell, and from then on a key reaches nobody the shell owns.
    let mut locker = support::Locker::connect(session.wayland());
    locker.lock();
    wait_for_lock_request(&mut wire);
    // The lock ends the EI session itself, so the real input path is closed at the EIS gate before the router sees a key.
    // The router's own `Unlocked` gate for the shell's node is covered at the unit level (`tests/tree_frame.rs`,
    // `tests/tree_live.rs` in gnoblin-wayland); this test pins that no key edge reaches the shell on the real path.
    remote.wait_for("the lock to end the EI session", |client| client.disconnected);
    remote.key(KEY_B, true);
    remote.key(KEY_B, false);
    let locked = key_events_within(&mut wire, Duration::from_millis(1500), 1);
    assert!(
        locked.is_empty(),
        "no key reaches the shell while the session is locked: {locked:?}"
    );
    session.stop().expect("the session stops cleanly");
}

// ---------------------------------------------------------------------------
// The head camera: what the XR extension, an observer, may write on it (design/xr-extension.md).

/// The serial after the shell made the head camera and the nodes beside it: what an observer's first write is built on.
const CREATED: u64 = 1;

/// A camera node named `name`, under `parent`.
fn create_camera(id: u64, parent: u64, name: &str) -> Vec<op::Properties> {
    let mut ops = create(id, parent);
    ops[0].class = "camera".to_owned();
    ops.push(set_text(id, "name", name));
    ops
}

/// The shell makes the room's head camera and a plain node named `head`, and an observer attaches beside it.
/// Returns the shell's wire and serial, the observer's wire, and the ids: the head camera, another camera, a node.
fn head_camera_session(session: &Session) -> (Wire, Wire, [u64; 3]) {
    let mut shell = Wire::connect(session.socket());
    let (held, _) = attach_and_snapshot(&mut shell, 1);
    let (head, other, plain) = (1000, 1001, 1002);
    let mut ops = create_camera(head, held.root, "head");
    ops.extend(create_camera(other, held.root, "other"));
    ops.extend(create(plain, held.root));
    ops.push(set_text(plain, "name", "head"));
    transact(&mut shell, 2, 0, 1, ops);
    let applied: ReplyPayload = reply_skipping_changes(&mut shell, "transact");
    assert!(applied.applied, "{applied:?}");
    let mut observer = Wire::connect(session.socket());
    attach_observer(&mut observer, 1);
    (shell, observer, [head, other, plain])
}

fn pose_ops(camera: u64) -> Vec<op::Properties> {
    vec![
        set_double(camera, "x", 1500.0),
        set_double(camera, "y", -1600.0),
        set_double(camera, "z", -250.0),
        set_list(camera, "rotation", vec![0.1, -0.2, 0.0]),
    ]
}

#[test]
fn an_observer_writes_the_head_cameras_pose_and_the_shell_hears_it() {
    let session = Session::start();
    let (mut shell, mut observer, [head, ..]) = head_camera_session(&session);
    transact(&mut observer, 2, CREATED, 2, pose_ops(head));
    let applied: ReplyPayload = reply_skipping_changes(&mut observer, "transact");
    assert!(applied.applied, "the head pose applies: {applied:?}");
    let heard = changes_until(&mut shell, |all| {
        all.iter().any(|c| c.node == head && c.key == "rotation")
    });
    for key in ["x", "y", "z"] {
        assert!(
            heard.iter().any(|c| c.node == head && c.key == key),
            "the shell hears {key}: {heard:?}"
        );
    }
    let x = heard.iter().find(|c| c.node == head && c.key == "x").expect("x");
    assert!((x.value_double - 1500.0).abs() < f64::EPSILON, "{x:?}");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn an_observer_may_not_write_any_other_node_or_property() {
    let session = Session::start();
    let (_shell, mut observer, [head, other, plain]) = head_camera_session(&session);
    let refused = |observer: &mut Wire, request_id: u64, ops: Vec<op::Properties>| {
        transact(observer, request_id, CREATED, 2, ops);
        let reply: ReplyPayload = reply_skipping_changes(observer, "transact");
        assert!(!reply.applied, "{reply:?}");
        reply
    };
    assert_eq!(
        refused(&mut observer, 2, vec![set_double(other, "x", 1.0)]).code,
        "not-holder",
        "a camera that is not named head"
    );
    assert_eq!(
        refused(&mut observer, 3, vec![set_double(plain, "x", 1.0)]).code,
        "not-holder",
        "a node named head that is not a camera"
    );
    assert_eq!(
        refused(&mut observer, 4, vec![set_double(head, "opacity", 0.5)]).code,
        "not-holder",
        "another property of the head camera"
    );
    let mut mixed = pose_ops(head);
    mixed.push(set_double(plain, "y", 1.0));
    let reply = refused(&mut observer, 5, mixed);
    assert_eq!((reply.code.as_str(), reply.failed_op), ("not-holder", 4));
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_pose_that_is_not_finite_is_refused_and_the_camera_keeps_its_last_pose() {
    let session = Session::start();
    let (mut shell, mut observer, [head, ..]) = head_camera_session(&session);
    transact(&mut observer, 2, CREATED, 2, pose_ops(head));
    let applied: ReplyPayload = reply_skipping_changes(&mut observer, "transact");
    assert!(applied.applied, "{applied:?}");
    for (request_id, bad) in [(3, f64::NAN), (4, f64::INFINITY), (5, f64::NEG_INFINITY)] {
        transact(
            &mut observer,
            request_id,
            applied.serial,
            3,
            vec![set_double(head, "x", 7.0), set_double(head, "y", bad)],
        );
        let reply: ReplyPayload = reply_skipping_changes(&mut observer, "transact");
        assert!(!reply.applied, "{bad}: {reply:?}");
        assert_eq!((reply.code.as_str(), reply.failed_op), ("not-finite", 1), "{bad}");
    }
    // The shell's mirror shows the first pose and never the refused x of 7.
    let heard = changes_until(&mut shell, |all| {
        all.iter().any(|c| c.node == head && c.key == "rotation")
    });
    assert!(
        heard
            .iter()
            .all(|c| !(c.node == head && c.key == "x" && (c.value_double - 7.0).abs() < f64::EPSILON)),
        "{heard:?}"
    );
    let mut later = Wire::connect(session.socket());
    let (_, nodes) = attach_observer(&mut later, 1);
    let x = prop_of(&nodes, head, "x").expect("x is not the default");
    assert!((x.value_double - 1500.0).abs() < f64::EPSILON, "{x:?}");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn the_layer_budget_is_the_observers_to_write_and_not_the_shells() {
    let session = Session::start();
    let (mut shell, mut observer, [head, other, _]) = head_camera_session(&session);
    transact(
        &mut shell,
        3,
        1,
        2,
        vec![set_number(head, "layer_budget", "uint", 0, 4, 0.0)],
    );
    let refused: ReplyPayload = reply_skipping_changes(&mut shell, "transact");
    assert_eq!(refused.code, "read-only", "the shell cannot write it: {refused:?}");

    transact(
        &mut observer,
        2,
        CREATED,
        2,
        vec![set_number(head, "layer_budget", "uint", 0, 16, 0.0)],
    );
    let applied: ReplyPayload = reply_skipping_changes(&mut observer, "transact");
    assert!(applied.applied, "{applied:?}");
    let heard = changes_until(&mut shell, |all| {
        all.iter().any(|c| c.node == head && c.key == "layer_budget")
    });
    let budget = heard.iter().find(|c| c.key == "layer_budget").expect("the budget");
    assert_eq!((budget.value_kind.as_str(), budget.value_uint), ("uint", 16));

    transact(
        &mut observer,
        3,
        applied.serial,
        3,
        vec![set_number(other, "layer_budget", "uint", 0, 16, 0.0)],
    );
    let refused: ReplyPayload = reply_skipping_changes(&mut observer, "transact");
    assert_eq!(refused.code, "not-holder", "only the camera named head: {refused:?}");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_burst_of_pose_writes_is_refused_rate_limited_and_the_rest_still_applies() {
    let session = Session::start();
    let (_shell, mut observer, [head, ..]) = head_camera_session(&session);
    // All sent before any answer is read: the compositor sees them within a millisecond or two of each other.
    let sent = 60_u64;
    for request_id in 0..sent {
        transact(
            &mut observer,
            10 + request_id,
            CREATED,
            2,
            vec![set_double(head, "x", request_id as f64)],
        );
    }
    let (mut applied, mut limited, mut serial) = (0_u64, 0_u64, CREATED);
    for _ in 0..sent {
        let reply: ReplyPayload = reply_skipping_changes(&mut observer, "transact");
        serial = serial.max(reply.serial);
        match reply.code.as_str() {
            "" => applied += 1,
            "rate-limited" => limited += 1,
            // Writes that got past the cap were all built against the same serial, and each changes the camera the next
            // one names: the rule for a transaction that is stale applies to them as to any other.
            "stale-base" => applied += 1,
            other => panic!("a refusal that is none of those: {other}: {reply:?}"),
        }
    }
    assert!(applied >= 1, "the first writes get through");
    assert!(applied <= 12, "the cap held: {applied} of {sent} got past it");
    assert!(limited >= 40, "the rest were refused rate-limited: {limited} of {sent}");
    assert_eq!(applied + limited, sent);
    // A write after the burst, once the allowance has refilled, applies again.
    std::thread::sleep(Duration::from_millis(100));
    transact(&mut observer, 1000, serial, 3, vec![set_double(head, "x", -1.0)]);
    let later: ReplyPayload = reply_skipping_changes(&mut observer, "transact");
    assert!(later.applied, "{later:?}");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_connection_that_is_not_the_owner_cannot_write_the_head_pose() {
    // D35: no capabilities, the owner gate is the connection predicate. A foreign uid is refused the attach, and a
    // transaction it sends without one is refused `not-privileged`, so no pose it writes reaches the tree.
    use std::os::unix::fs::MetadataExt as _;
    let own = std::fs::metadata("/proc/self").expect("the process directory").uid();
    let (session, _ends) = Session::start_privileged_configured(&[], |config| {
        config.ipc_owner_uid = Some(own.wrapping_add(1));
    });
    let mut wire = Wire::connect(session.socket());
    wire.send(&Envelope::request_message(
        NAMESPACE,
        "attach",
        1,
        &AttachArgs {
            version: 1,
            mode: "observer".to_owned(),
        },
    ));
    assert_eq!(wire.receive().expect("an answer").error_code(), Some("not-owner"));
    transact(&mut wire, 2, 0, 0, pose_ops(1000));
    let reply: ReplyPayload = reply_skipping_changes(&mut wire, "transact");
    assert!(!reply.applied);
    assert_eq!(reply.code, "not-privileged", "{reply:?}");
    transact(
        &mut wire,
        3,
        0,
        0,
        vec![set_number(1000, "layer_budget", "uint", 0, 99, 0.0)],
    );
    let reply: ReplyPayload = reply_skipping_changes(&mut wire, "transact");
    assert_eq!(reply.code, "not-privileged", "{reply:?}");
    session.stop().expect("the session stops cleanly");
}
