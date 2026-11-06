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
