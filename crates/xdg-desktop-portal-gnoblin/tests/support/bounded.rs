//! A bound on how long a test may take.
//!
//! zbus gives a method call no deadline. A call over a private bus whose reply is lost waits
//! for ever, and a test that waits for it holds its CI job until the job's own limit cancels
//! it, with nothing in the log to say which call it was: the Arch job of test matrix run
//! 45963491201 held for 100 minutes in `settings.rs` (bead gnoblin-uplg). `within` is the
//! helper `gnoblin-service-testkit` has for the service tests' bus steps, here for the
//! portal's.

use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

/// Run `step` on a thread of its own and fail the test, naming `what`, if it has not returned
/// within `limit`.
///
/// The step has a thread of its own so that the limit holds whatever the step is stuck on: a
/// call whose reply never comes, a read that never ends. A thread that missed the limit is
/// left behind and ends with the test process.
///
/// # Panics
///
/// When `step` has not returned within `limit`. A panic of `step` itself is raised again as it
/// was, so a failed assertion keeps its own message.
pub fn within<T: Send + 'static>(what: &str, limit: Duration, step: impl FnOnce() -> T + Send + 'static) -> T {
    let (sender, answer) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let _ = sender.send(step());
    });
    match answer.recv_timeout(limit) {
        Ok(value) => value,
        Err(RecvTimeoutError::Timeout) => panic!("{what} did not return within {limit:?}"),
        Err(RecvTimeoutError::Disconnected) => match thread.join() {
            Err(payload) => std::panic::resume_unwind(payload),
            Ok(()) => unreachable!("a step that returned has sent its value first"),
        },
    }
}
