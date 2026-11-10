//! The attach, snapshot and change flow against a fake owner socket.

use std::f64::consts::FRAC_PI_2;
use std::os::unix::net::UnixStream;
use std::thread;

use gnoblin_proto::tree::change;
use gnoblin_proto::tree::snapshot_node;
use gnoblin_proto::tree::tree::{AttachedPayload, ChangesPayload, SnapshotPayload};
use gnoblin_proto::{Envelope, read_frame, write_frame};
use xr_scene::{CameraProjection, Class};
use xr_tree::TreeClient;

/// The request id the client uses for its attach request.
const ATTACH_REQUEST_ID: u64 = 1;

fn set(node: u64, key: &str) -> change::Properties {
    change::Properties {
        kind: "set".into(),
        node,
        key: key.into(),
        ..Default::default()
    }
}

fn double(node: u64, key: &str, value: f64) -> change::Properties {
    change::Properties {
        value_kind: "double".into(),
        value_double: value,
        ..set(node, key)
    }
}

fn boolean(node: u64, key: &str, value: bool) -> change::Properties {
    change::Properties {
        value_kind: "bool".into(),
        value_bool: value,
        ..set(node, key)
    }
}

fn text(node: u64, key: &str, value: &str) -> change::Properties {
    change::Properties {
        value_kind: "text".into(),
        value_text: value.into(),
        ..set(node, key)
    }
}

fn list(node: u64, key: &str, values: &[f64]) -> change::Properties {
    change::Properties {
        value_kind: "list_double".into(),
        value_list: values.to_vec(),
        ..set(node, key)
    }
}

fn snapshot_node(
    id: u64,
    parent: u64,
    index: i64,
    class: &str,
    props: Vec<change::Properties>,
) -> snapshot_node::Properties {
    snapshot_node::Properties {
        id,
        parent,
        index,
        class: class.into(),
        props,
        ..Default::default()
    }
}

/// Starts a fake owner that answers the attach request with `events`, then holds the
/// socket open until the client drops it. Returns the client end.
fn serve(events: Vec<Envelope>) -> UnixStream {
    let (client, mut server) = UnixStream::pair().expect("socket pair");
    thread::spawn(move || {
        let _attach = read_frame(&mut server).expect("read attach").expect("attach frame");
        for event in &events {
            if write_frame(&mut server, event).is_err() {
                return;
            }
        }
        let _ = read_frame(&mut server);
    });
    client
}

/// The attach reply, then the snapshot in one chunk.
fn attached_and_snapshot(nodes: Vec<snapshot_node::Properties>) -> Vec<Envelope> {
    vec![
        Envelope::reply_message(
            "tree",
            "attach",
            ATTACH_REQUEST_ID,
            &AttachedPayload {
                epoch: 1,
                root: 1,
                serial: 1,
            },
        ),
        Envelope::event_message(
            "tree",
            "snapshot",
            &SnapshotPayload {
                serial: 1,
                chunk: 0,
                last: true,
                nodes,
            },
        ),
    ]
}

#[test]
fn snapshot_builds_the_model() {
    let room = snapshot_node::Properties {
        props: vec![
            double(1, "x", 1.5),
            list(1, "rotation", &[0.0, 0.0, FRAC_PI_2]),
            list(1, "scale", &[2.0]),
        ],
        ..snapshot_node(1, 0, 0, "node", vec![])
    };
    let view = snapshot_node(
        2,
        1,
        0,
        "view",
        vec![double(2, "width", 1920.0), double(2, "height", 1080.0)],
    );
    let camera = snapshot_node::Properties {
        props: vec![text(3, "name", "head"), text(3, "projection", "perspective")],
        ..snapshot_node(3, 1, 1, "camera", vec![])
    };
    let events = attached_and_snapshot(vec![room, view, camera]);

    let client = TreeClient::from_stream(serve(events)).expect("attach");
    let model = client.model();

    assert_eq!(model.roots, vec![1]);
    assert_eq!(model.nodes[&1].children, vec![2, 3]);
    assert_eq!(model.nodes[&1].position[0], 1.5);
    assert!(
        (model.nodes[&1].rotation[2] - 90.0).abs() < 1e-9,
        "radians become degrees"
    );
    assert_eq!(model.nodes[&1].scale, 2.0);
    assert!(matches!(model.nodes[&2].class, Class::View { width, height, .. } if width == 1920.0 && height == 1080.0));
    assert_eq!(model.nodes[&3].name.as_deref(), Some("head"));
    assert!(matches!(
        model.nodes[&3].class,
        Class::Camera {
            projection: CameraProjection::Orthographic
        }
    ));
    assert_eq!(model.nodes[&3].metres, Some(0.001), "a camera's metres default");
}

#[test]
fn change_updates_the_model() {
    let view = snapshot_node(2, 0, 0, "view", vec![double(2, "width", 1920.0)]);
    let mut events = attached_and_snapshot(vec![view]);
    events.push(Envelope::event_message(
        "tree",
        "changes",
        &ChangesPayload {
            serial: 2,
            changes: vec![double(2, "width", 800.0), boolean(2, "visible", false)],
        },
    ));

    let mut client = TreeClient::from_stream(serve(events)).expect("attach");
    let mut changed = false;
    for _ in 0..100 {
        changed |= client.poll().expect("poll");
        if changed {
            break;
        }
    }

    assert!(changed, "the changes batch applies");
    let node = &client.model().nodes[&2];
    assert!(matches!(node.class, Class::View { width, .. } if width == 800.0));
    assert!(!node.visible);
}

#[test]
fn attach_refusal_is_an_error() {
    let refusal = Envelope::error(
        "tree",
        "attach",
        ATTACH_REQUEST_ID,
        "denied",
        "the owner refused the tree",
    );
    let result = TreeClient::from_stream(serve(vec![refusal]));
    assert!(result.is_err(), "a refused attach must not yield a client");
}
