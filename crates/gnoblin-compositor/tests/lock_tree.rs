//! The session lock on the tree: the compositor's `lock` node, read and taken over the real socket.
//!
//! The lock's rules are tested where they live (`gnoblin-wayland/tests/session_lock.rs` with real Wayland
//! clients, `gnoblin-lock`'s state machine). These tests are the tree's face of them, for the connections that
//! are not the shell: the screensaver service and the compat adapters are *observers*, and `gnoblinctl lock`
//! is one. The failure modes, each with its test:
//!
//! 1. A connection that is not the session owner's can neither observe the lock nor take it.
//! 2. An observer may take the lock and may never unlock it: `authenticate` is the shell's, and an
//!    observer's call to it is refused before any check runs.
//! 3. Taking the lock twice is the same lock: no flap through `not-locked`, no second cover.
//! 4. The state is observable and replays: an observer that attaches late reads what the others read.

mod support;

use std::time::{Duration, Instant};

use gnoblin_proto::tree::op;
use gnoblin_proto::tree::tree::{
    AttachArgs, AttachedPayload, ChangesPayload, ReplyPayload, SnapshotPayload, TransactArgs,
};
use gnoblin_proto::{Envelope, Kind};
use support::{Session, Wire};

const NAMESPACE: &str = "tree";

/// An observer's connection, the lock node's id and the state the node last had.
struct Observed {
    wire: Wire,
    lock: u64,
    serial: u64,
    state: String,
}

fn attach_request(wire: &mut Wire, request_id: u64, mode: &str) {
    wire.send(&Envelope::request_message(
        NAMESPACE,
        "attach",
        request_id,
        &AttachArgs {
            version: 1,
            mode: mode.to_owned(),
        },
    ));
}

/// Attaches `wire` as an observer and reads the snapshot's lock node.
fn observe(mut wire: Wire) -> Observed {
    attach_request(&mut wire, 1, "observer");
    let envelope = wire.receive().expect("an attach answer");
    assert!(
        envelope.error_code().is_none(),
        "the owner's observer attaches: {envelope:?}"
    );
    let attached: AttachedPayload = envelope.payload_as().expect("the payload decodes");
    let mut lock = None;
    let mut state = "not-locked".to_owned();
    loop {
        let envelope = wire.receive().expect("a snapshot chunk");
        assert_eq!(envelope.member, "snapshot");
        let chunk: SnapshotPayload = envelope.payload_as().expect("the payload decodes");
        for node in chunk.nodes.iter().filter(|node| node.class == "lock") {
            lock = Some(node.id);
            if let Some(prop) = node.props.iter().find(|prop| prop.key == "state") {
                state.clone_from(&prop.value_text);
            }
        }
        if chunk.last {
            break;
        }
    }
    Observed {
        wire,
        lock: lock.expect("the compositor's tree has a lock node"),
        serial: attached.serial,
        state,
    }
}

fn call(node: u64, method: &str, text: &str) -> op::Properties {
    op::Properties {
        kind: "call".to_owned(),
        node,
        method: method.to_owned(),
        text: text.to_owned(),
        ..Default::default()
    }
}

/// Sends one transaction and reads to its reply. State changes met on the way are returned in order.
fn transact(observed: &mut Observed, request_id: u64, ops: Vec<op::Properties>) -> (ReplyPayload, Vec<String>) {
    observed.wire.send(&Envelope::request_message(
        NAMESPACE,
        "transact",
        request_id,
        &TransactArgs {
            base_serial: observed.serial,
            serial: observed.serial + 1,
            ops,
        },
    ));
    let mut states = Vec::new();
    loop {
        let envelope = observed.wire.receive().expect("a frame");
        match envelope.kind {
            Some(Kind::Event(_)) => note_states(observed, &envelope, &mut states),
            Some(Kind::Reply(_)) => {
                let reply: ReplyPayload = envelope.payload_as().expect("the payload decodes");
                observed.serial = observed.serial.max(reply.serial);
                return (reply, states);
            }
            _ => panic!("an unexpected frame `{}` in the transact loop", envelope.member),
        }
    }
}

fn note_states(observed: &mut Observed, envelope: &Envelope, states: &mut Vec<String>) {
    if envelope.member != "changes" {
        return;
    }
    let batch: ChangesPayload = envelope.payload_as().expect("the payload decodes");
    observed.serial = observed.serial.max(batch.serial);
    for change in &batch.changes {
        if change.kind == "set" && change.node == observed.lock && change.key == "state" {
            states.push(change.value_text.clone());
            observed.state.clone_from(&change.value_text);
        }
    }
}

/// Reads change batches until the lock node reports `target`, or fails after `limit`.
fn wait_for(observed: &mut Observed, target: &str, limit: Duration) -> Vec<String> {
    let deadline = Instant::now() + limit;
    let mut states = Vec::new();
    observed.wire.set_read_timeout(Duration::from_millis(100));
    while observed.state != target {
        assert!(
            Instant::now() < deadline,
            "the lock never reached `{target}`; it is `{}` after {states:?}",
            observed.state
        );
        if let Some(envelope) = observed.wire.receive_within() {
            note_states(observed, &envelope, &mut states);
        }
    }
    states
}

/// What the lock node says over the next `window`, for a test that proves nothing changes.
fn states_within(observed: &mut Observed, window: Duration) -> Vec<String> {
    let deadline = Instant::now() + window;
    let mut states = Vec::new();
    observed.wire.set_read_timeout(Duration::from_millis(50));
    while Instant::now() < deadline {
        if let Some(envelope) = observed.wire.receive_within() {
            note_states(observed, &envelope, &mut states);
        }
    }
    states
}

fn own_uid() -> u32 {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::metadata("/proc/self").expect("the process directory").uid()
}

#[test]
fn an_observer_takes_the_lock_and_a_late_observer_reads_it() {
    // The screensaver's `Lock`, over the socket: the call has no answer, the node's state is it, and the
    // state is a snapshot value, so a connection that attaches after the lock reads it too.
    let session = Session::start();
    let mut first = observe(Wire::connect(session.socket()));
    assert_eq!(first.state, "not-locked", "a new session is not locked");

    let lock = first.lock;
    let (reply, _) = transact(&mut first, 10, vec![call(lock, "lock", "")]);
    assert!(reply.applied, "an observer's `lock` call applies: {reply:?}");
    wait_for(&mut first, "locked", Duration::from_secs(10));

    let late = observe(Wire::connect(session.socket()));
    assert_eq!(
        late.state, "locked",
        "the lock replays to an observer that attaches late"
    );
    assert_eq!(late.lock, first.lock, "one lock node per session");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn taking_the_lock_twice_is_one_lock() {
    // The failure mode: a second request while locked restarts the cover, or flaps the state through
    // `not-locked` and lets a capture through for a frame.
    let session = Session::start();
    let mut observed = observe(Wire::connect(session.socket()));
    let lock = observed.lock;
    let (reply, _) = transact(&mut observed, 10, vec![call(lock, "lock", "")]);
    assert!(reply.applied, "{reply:?}");
    wait_for(&mut observed, "locked", Duration::from_secs(10));

    let (again, during) = transact(&mut observed, 11, vec![call(lock, "lock", "")]);
    assert!(again.applied, "a second `lock` is accepted: {again:?}");
    let after = states_within(&mut observed, Duration::from_millis(500));
    let seen: Vec<&String> = during.iter().chain(&after).collect();
    assert!(
        !seen.iter().any(|state| state.as_str() == "not-locked"),
        "the repeat never passes through `not-locked`: {seen:?}"
    );
    assert_eq!(observed.state, "locked");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn an_observer_cannot_unlock_through_authenticate() {
    // The failure mode: the screensaver service or an adapter (observers) answers the lock's password
    // prompt, so any owner-uid process that can attach unlocks the session. `authenticate` is the shell's;
    // an observer's call is refused at the first op, and the session stays locked.
    let session = Session::start();
    let mut observed = observe(Wire::connect(session.socket()));
    let lock = observed.lock;
    let (reply, _) = transact(&mut observed, 10, vec![call(lock, "lock", "")]);
    assert!(reply.applied, "{reply:?}");
    wait_for(&mut observed, "locked", Duration::from_secs(10));

    let (refused, _) = transact(
        &mut observed,
        11,
        vec![call(lock, "authenticate", "any password at all")],
    );
    assert!(!refused.applied, "an observer's authenticate is refused: {refused:?}");
    assert_eq!(refused.code, "not-holder");
    assert_eq!(refused.failed_op, 0);
    let after = states_within(&mut observed, Duration::from_millis(500));
    assert!(after.is_empty(), "the lock did not move: {after:?}");
    assert_eq!(observed.state, "locked");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn an_observer_may_make_no_other_call() {
    // The one call an observer may make is `lock`. A `close` aimed at the lock node is a different call and
    // is refused the same way, so the allowance is the method and not "any call on the lock node".
    let session = Session::start();
    let mut observed = observe(Wire::connect(session.socket()));
    let lock = observed.lock;
    let (refused, _) = transact(&mut observed, 10, vec![call(lock, "close", "")]);
    assert!(!refused.applied, "{refused:?}");
    assert_eq!(refused.code, "not-holder");
    assert_eq!(observed.state, "not-locked", "the refused call took no lock");
    session.stop().expect("the session stops cleanly");
}

#[test]
fn a_connection_that_is_not_the_owner_can_neither_observe_nor_lock() {
    // The failure mode: any program on the machine locks the session on a whim (a denial of service) or
    // watches whether it is locked. The owner gate (`ipc/privileged.rs`) refuses the attach, and a
    // transaction sent without one is refused `not-privileged`.
    let foreign = own_uid().wrapping_add(1);
    let (session, _ends) = Session::start_privileged_configured(&[], |config| {
        config.ipc_owner_uid = Some(foreign);
    });
    let mut wire = Wire::connect(session.socket());
    attach_request(&mut wire, 1, "observer");
    let envelope = wire.receive().expect("an answer");
    assert_eq!(envelope.error_code(), Some("not-owner"), "{envelope:?}");

    wire.send(&Envelope::request_message(
        NAMESPACE,
        "transact",
        2,
        &TransactArgs {
            base_serial: 0,
            serial: 1,
            ops: vec![call(2, "lock", "")],
        },
    ));
    let envelope = wire.receive().expect("an answer");
    let reply: ReplyPayload = envelope.payload_as().expect("the payload decodes");
    assert!(
        !reply.applied,
        "a connection that never attached cannot transact: {reply:?}"
    );
    assert_eq!(reply.code, "not-privileged");
    session.stop().expect("the session stops cleanly");
}
