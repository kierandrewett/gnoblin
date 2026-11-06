//! `window.kill` over real IPC: the named refusals, and the one positive kill.
//!
//! The in-process test client shares the test's pid, which `window.kill` refuses, so
//! the positive case registers a spawned `sleep` as the window's pid through the
//! session's `window_pid_override` seam.

mod support;

use std::os::unix::process::ExitStatusExt as _;
use std::process::{Child, Command as Process, Stdio};

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
    let session = Session::start_with_window_pid(None, Some(None));
    let mut client = Client::connect(session.wayland());
    let _window = client.map_window("org.example.kill", "unknown");
    client.roundtrip();
    let id = wait_for_listed_id(&session, "unknown");

    let refusal = session.ctl(&["window", "call", &id.to_string(), "kill"]);
    assert_eq!(refusal.code, 1, "a refused call exits 1: {}", refusal.out);
    assert!(refusal.err.contains("pid is unknown"), "{}", refusal.err);
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
    let session = Session::start_with_window_pid(None, Some(Some(victim.id())));
    let mut client = Client::connect(session.wayland());
    let _window = client.map_window("org.example.kill", "victim");
    client.roundtrip();
    let id = wait_for_listed_id(&session, "victim");

    session.ctl(&["window", "call", &id.to_string(), "kill"]).ok();

    let status = victim.wait().expect("the victim can be waited for");
    assert_eq!(status.signal(), Some(9), "the victim died by SIGKILL: {status:?}");
}
