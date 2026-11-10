//! Window requests reach the shell as `event` frames on the window's node (design/node-core.md,
//! section 5). The shell is the policy authority while it holds the tree: a request arrives as
//! an event and the window manager does not act on it. With no shell the window manager acts as
//! it always has.

mod support;

use std::time::{Duration, Instant};

use gnoblin_proto::tree::tree::{AttachArgs, AttachedPayload, EventPayload, SnapshotPayload};
use gnoblin_proto::{Envelope, Kind};
use support::{Client, Session, Wire};

const NAMESPACE: &str = "tree";

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

/// Re-attaches on the same connection until the snapshot holds a window titled `title`, and
/// returns the window's node id. The composer creates the window node on the frame after the
/// client maps the window, so the first attach may not see it yet. The shell holds the tree
/// afterwards, so the window's requests become events for this connection.
fn attach_holding_window(wire: &mut Wire, title: &str) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut request_id = 1;
    loop {
        wire.send(&Envelope::request_message(
            NAMESPACE,
            "attach",
            request_id,
            &AttachArgs { version: 1 },
        ));
        let envelope = wire.receive().expect("the attach reply");
        let attached: AttachedPayload = envelope.payload_as().expect("the payload decodes");
        let mut found = None;
        loop {
            let chunk: SnapshotPayload = event(wire, "snapshot");
            assert_eq!(chunk.serial, attached.serial, "the snapshot is at the attach serial");
            if let Some(node) = chunk
                .nodes
                .iter()
                .find(|node| node.class == "window" && node.title == title)
            {
                found = Some(node.id);
            }
            if chunk.last {
                break;
            }
        }
        if let Some(node) = found {
            return node;
        }
        assert!(Instant::now() < deadline, "no window titled `{title}` in the tree");
        request_id += 1;
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Reads frames until an `event` for `request` arrives, or fails after five seconds.
fn event_for_request(wire: &mut Wire, request: &str) -> EventPayload {
    wire.set_read_timeout(Duration::from_millis(200));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "no `{request}` event arrived");
        let Some(envelope) = wire.receive_within() else {
            continue;
        };
        if envelope.member != "event" {
            continue;
        }
        let payload: EventPayload = envelope.payload_as().expect("the payload decodes");
        if payload.request == request {
            return payload;
        }
    }
}

#[test]
fn a_maximise_request_while_a_shell_holds_the_tree_is_an_event_for_the_window_node() {
    let session = Session::start();
    let mut client = Client::connect(session.wayland());
    let window = client.map_window("org.example.window", "Requested");
    let mut wire = Wire::connect(session.socket());
    let node = attach_holding_window(&mut wire, "Requested");

    client.request_maximize(&window);
    let payload = event_for_request(&mut wire, "maximize");

    assert_eq!(payload.kind, "request", "a request is delivered as a request event");
    assert_eq!(payload.node, node, "the event is for the window's node");
    assert!(payload.seq >= 1, "the event carries a sequence number");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_maximise_request_with_no_shell_is_acted_on_by_the_window_manager_as_before() {
    let session = Session::start();
    let mut client = Client::connect(session.wayland());
    let window = client.map_window("org.example.window", "Unshelled");

    client.request_maximize(&window);

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if client.configured_size(&window).is_some_and(|(width, _)| width > 200) {
            break;
        }
        assert!(Instant::now() < deadline, "the window was never maximised");
        client.roundtrip();
        std::thread::sleep(Duration::from_millis(10));
    }
    session.stop().expect("the session stops cleanly");
}

#[test]
fn event_sequence_numbers_increase_across_requests() {
    let session = Session::start();
    let mut client = Client::connect(session.wayland());
    let window = client.map_window("org.example.window", "Sequenced");
    let mut wire = Wire::connect(session.socket());
    attach_holding_window(&mut wire, "Sequenced");

    client.request_maximize(&window);
    let first = event_for_request(&mut wire, "maximize");
    client.request_move(&window);
    let second = event_for_request(&mut wire, "move");

    assert!(
        second.seq > first.seq,
        "sequence numbers increase: {} then {}",
        first.seq,
        second.seq
    );
    session.stop().expect("the session stops cleanly");
}
