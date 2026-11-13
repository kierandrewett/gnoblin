//! `window.kill` over real IPC: the named refusals, and the positive kill.
//!
//! The in-process test client shares the test's pid, which `window.kill` refuses, so the
//! cases that reach the signal register a spawned `sleep` as the window's process through
//! the session's `window_process_override` seam. The seam stands in for the process the
//! connection recorded, so a case can name a process whose pid has been reused since.

mod support;

use std::os::unix::process::ExitStatusExt as _;
use std::process::{Child, Command as Process, Stdio};

use gnoblin_wayland::{ProcessIdentity, WindowProcess};
use support::{Client, Session, wait_for_listed_id};

fn sleeper() -> Child {
    Process::new("sleep")
        .arg("60")
        .stdin(Stdio::null())
        .spawn()
        .expect("sleep is on PATH")
}

#[test]
fn kill_refuses_a_window_whose_pid_is_unknown() {
    let session = Session::start_with_window_process(None, Some(WindowProcess::Unknown));
    let mut client = Client::connect(session.wayland());
    let _window = client.map_window("org.example.kill", "unknown");
    client.roundtrip();
    let id = wait_for_listed_id(&session, "unknown");

    let refusal = session.ctl(&["window", "call", &id.to_string(), "kill"]);
    assert_eq!(refusal.code, 1, "a refused call exits 1: {}", refusal.out);
    assert!(refusal.err.contains("pid is unknown"), "{}", refusal.err);
}

#[test]
fn kill_refuses_a_window_whose_client_has_disconnected() {
    let session = Session::start_with_window_process(None, Some(WindowProcess::Disconnected));
    let mut client = Client::connect(session.wayland());
    let _window = client.map_window("org.example.kill", "disconnected");
    client.roundtrip();
    let id = wait_for_listed_id(&session, "disconnected");

    let refusal = session.ctl(&["window", "call", &id.to_string(), "kill"]);
    assert_eq!(refusal.code, 1, "a refused call exits 1: {}", refusal.out);
    assert!(refusal.err.contains("process-changed"), "{}", refusal.err);
    assert!(refusal.err.contains("disconnected"), "{}", refusal.err);
}

#[test]
fn kill_refuses_the_compositors_own_process() {
    // The in-process client's pid is this test process, which is the compositor's.
    let session = Session::start();
    let mut client = Client::connect(session.wayland());
    let _window = client.map_window("org.example.kill", "own");
    client.roundtrip();
    let id = wait_for_listed_id(&session, "own");

    let refusal = session.ctl(&["window", "call", &id.to_string(), "kill"]);
    assert_eq!(refusal.code, 1, "a refused call exits 1: {}", refusal.out);
    assert!(refusal.err.contains("own process"), "{}", refusal.err);
}

#[test]
fn kill_refuses_a_window_that_is_not_there() {
    let session = Session::start();
    let refusal = session.ctl(&["window", "call", "9999999", "kill"]);
    assert_eq!(refusal.code, 1, "a refused call exits 1: {}", refusal.out);
    assert!(refusal.err.contains("object-unknown"), "{}", refusal.err);
}

#[test]
fn kill_sends_sigkill_to_the_windows_process() {
    let mut victim = sleeper();
    let identity = ProcessIdentity::of(victim.id()).expect("the victim has a stat line");
    let session = Session::start_with_window_process(None, Some(WindowProcess::Connected(identity)));
    let mut client = Client::connect(session.wayland());
    let _window = client.map_window("org.example.kill", "victim");
    client.roundtrip();
    let id = wait_for_listed_id(&session, "victim");

    session.ctl(&["window", "call", &id.to_string(), "kill"]).ok();

    let status = victim.wait().expect("the victim can be waited for");
    assert_eq!(status.signal(), Some(9), "the victim died by SIGKILL: {status:?}");
}

#[test]
fn kill_refuses_a_pid_reused_since_the_client_connected_and_spares_the_process() {
    // The window recorded an earlier process with this pid: the same number, a different
    // start time. The process now holding the pid must not be signalled.
    let mut other = sleeper();
    let current = ProcessIdentity::of(other.id()).expect("the other process has a stat line");
    let recorded = ProcessIdentity {
        start_time: current.start_time + 1,
        ..current
    };
    let session = Session::start_with_window_process(None, Some(WindowProcess::Connected(recorded)));
    let mut client = Client::connect(session.wayland());
    let _window = client.map_window("org.example.kill", "reused");
    client.roundtrip();
    let id = wait_for_listed_id(&session, "reused");

    let refusal = session.ctl(&["window", "call", &id.to_string(), "kill"]);
    assert_eq!(refusal.code, 1, "a refused call exits 1: {}", refusal.out);
    assert!(refusal.err.contains("process-changed"), "{}", refusal.err);
    let still_running = other.try_wait().expect("the other process can be polled").is_none();
    assert!(still_running, "the other process must survive the refusal");

    other.kill().expect("the other process can be killed");
    other.wait().expect("the other process can be waited for");
}

/// The shell's `Window:kill` is a call on the window's node, so it needs the tree, which only the session owner's
/// connection may hold. A connection that is not the owner's is refused before the call is looked at, whichever node
/// it names, and the process behind the window lives. The node's id is not known to a connection that cannot read the
/// tree, so the call is made on every id a small session allocates.
#[test]
fn a_connection_that_is_not_the_owner_cannot_kill_a_window_through_the_tree() {
    use gnoblin_proto::tree::op;
    use gnoblin_proto::tree::tree::{ReplyPayload, TransactArgs};
    use gnoblin_proto::{Envelope, Kind};
    use std::os::unix::fs::MetadataExt as _;

    let victim = sleeper();
    let identity = ProcessIdentity::of(victim.id()).expect("the victim has a stat line");
    let foreign = std::fs::metadata("/proc/self")
        .expect("the process directory")
        .uid()
        .wrapping_add(1);
    let (session, _ends) = Session::start_privileged_configured(&[], |config| {
        config.ipc_owner_uid = Some(foreign);
        config.wayland.window_process_override = Some(WindowProcess::Connected(identity));
    });
    let mut client = Client::connect(session.wayland());
    let _window = client.map_window("org.example.kill", "tree-victim");
    client.roundtrip();

    let mut wire = support::Wire::connect(session.socket());
    for node in 1..=64_u64 {
        wire.send(&Envelope::request_message(
            "tree",
            "transact",
            node,
            &TransactArgs {
                base_serial: 0,
                serial: node,
                ops: vec![op::Properties {
                    kind: "call".to_owned(),
                    node,
                    method: "kill".to_owned(),
                    ..Default::default()
                }],
            },
        ));
        let envelope = loop {
            let envelope = wire.receive().expect("an answer");
            if matches!(envelope.kind, Some(Kind::Reply(_))) {
                break envelope;
            }
        };
        let reply: ReplyPayload = envelope.payload_as().expect("the payload decodes");
        assert!(
            !reply.applied,
            "a call from a connection that never attached is refused: {reply:?}"
        );
        assert_eq!(reply.code, "not-privileged");
    }
    client.roundtrip();
    let mut victim = victim;
    assert!(
        victim.try_wait().expect("the victim is waitable").is_none(),
        "the process behind the window is still running"
    );
    victim.kill().expect("the victim can be killed");
    victim.wait().expect("the victim can be waited for");
}
