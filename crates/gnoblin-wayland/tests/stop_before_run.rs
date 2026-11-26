//! A stop asked for before the loop runs, or while it is being entered, ends the run.
//!
//! calloop's `EventLoop::run` clears the loop's stop flag as it starts. A stop made between the session being built and
//! the loop being entered, which is what a session dropped as soon as it serves does, was forgotten, and the loop then ran
//! for ever while the thread that asked for the stop waited to join it. The server keeps the request itself, so every one
//! of these runs ends.

use gnoblin_wayland_testkit::stock;

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use gnoblin_backend::headless::Headless;
use gnoblin_wayland::{Config, ControlHandle, Server};

/// How long a run may take to end once it was asked to. A loop that lost the request would never end, and the test fails
/// on this limit rather than hanging.
const WATCHDOG: Duration = Duration::from_secs(60);

/// How many sessions the race test builds and stops straight away.
const ROUNDS: usize = 200;

/// Who asks the loop to stop, and when.
#[derive(Clone, Copy)]
enum Ask {
    /// The session's own thread, after the server is built and before it runs: no race, the loop has not started.
    BeforeRun,
    /// Another thread, as soon as it has the session's control, while the session's thread is entering the loop.
    FromAnotherThread,
}

/// Builds a session on a thread of its own, has it stopped as `ask` says, and returns whether its run ended in success.
///
/// The server is created and run on one thread, as its loop requires.
fn run_ends(ask: Ask) -> bool {
    let (handed, control_rx) = mpsc::channel::<ControlHandle>();
    let (ended, outcome_rx) = mpsc::channel::<bool>();
    thread::spawn(move || {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let config = Config {
            socket_path: Some(directory.path().join("wayland-0.sock")),
            ..stock::config()
        };
        let server = Server::new(Box::new(Headless::new()), config).expect("the session starts");
        let control = server.control();
        if let Ask::BeforeRun = ask {
            control.stop();
        }
        handed.send(control).expect("the test is waiting for the control");
        let _ = ended.send(server.run().is_ok());
    });
    let control = control_rx.recv().expect("the session was built");
    if let Ask::FromAnotherThread = ask {
        control.stop();
    }
    if let Ok(ok) = outcome_rx.recv_timeout(WATCHDOG) {
        return ok;
    }
    // The loop lost the request. A second one, made now that the loop is running, ends it so the thread does not outlive
    // the test.
    control.stop();
    panic!("the loop did not end after it was asked to stop");
}

#[test]
fn a_stop_asked_for_before_the_loop_runs_ends_it() {
    assert!(
        run_ends(Ask::BeforeRun),
        "a run that was stopped before it began is not a failure"
    );
}

#[test]
fn a_session_stopped_as_soon_as_it_is_built_always_ends() {
    for round in 0..ROUNDS {
        assert!(
            run_ends(Ask::FromAnotherThread),
            "round {round}: a run that was stopped is not a failure"
        );
    }
}
