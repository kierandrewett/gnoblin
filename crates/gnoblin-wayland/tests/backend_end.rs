//! How a backend ends a session: the person closing the last window and the device going away.
//!
//! The two are different endings, and the loop must keep them apart. A session whose last window was closed has
//! been logged out of, so the loop returns success and reports the logout; a session whose device is gone has no
//! output and can never have one, so the loop returns a failure instead of serving nothing for ever. The headless
//! backend states both through its handle, the same events a nested session's parent produces.

mod stock;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use gnoblin_backend::headless::{Headless, HeadlessHandle};
use gnoblin_wayland::{Config, LoopError, Server};

/// How long a session may run before the watchdog stops it. A loop that ignores the backend's ending would run until
/// then, and the test fails on the flag the watchdog sets rather than hanging.
const WATCHDOG: Duration = Duration::from_secs(20);

/// What a session's loop returned, whether it was the watchdog that stopped it, and whether it reported a logout.
struct Ended {
    outcome: Result<(), LoopError>,
    watchdog: bool,
    logged_out: bool,
}

/// Runs a headless session on this thread, and `act` on a worker once it is serving.
fn run_with(act: impl FnOnce(&HeadlessHandle) + Send + 'static) -> Ended {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let config = Config {
        socket_path: Some(directory.path().join("wayland-0.sock")),
        ..crate::stock::config()
    };
    let backend = Headless::new();
    let handle = backend.handle();
    let server = Server::new(Box::new(backend), config).expect("the session starts");
    let control = server.control();
    let watchdog = Arc::new(AtomicBool::new(false));
    let worker = thread::spawn({
        let watchdog = Arc::clone(&watchdog);
        let control = control.clone();
        move || {
            thread::sleep(Duration::from_millis(100));
            act(&handle);
            thread::sleep(WATCHDOG);
            // Only reached when the loop did not end by itself.
            watchdog.store(true, Ordering::SeqCst);
            control.stop();
        }
    });
    let outcome = server.run();
    drop(worker);
    Ended {
        outcome,
        watchdog: watchdog.load(Ordering::SeqCst),
        logged_out: control.logged_out(),
    }
}

#[test]
fn a_session_end_stops_the_loop_as_a_logout() {
    let ended = run_with(HeadlessHandle::end_session);
    assert!(!ended.watchdog, "the loop ended by itself");
    assert!(ended.outcome.is_ok(), "a logout is not a failure: {:?}", ended.outcome);
    assert!(ended.logged_out, "the session reports that it was logged out of");
}

#[test]
fn a_lost_device_stops_the_loop_with_a_failure() {
    let ended = run_with(HeadlessHandle::lose_device);
    assert!(
        !ended.watchdog,
        "the loop ended by itself instead of serving nothing for ever"
    );
    assert!(
        matches!(ended.outcome, Err(LoopError::BackendLost)),
        "a device that is gone is a failure: {:?}",
        ended.outcome
    );
    assert!(!ended.logged_out, "it was not a logout");
}
