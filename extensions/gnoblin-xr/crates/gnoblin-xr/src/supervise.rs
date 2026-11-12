//! Keeps the extension alive across the OpenXR runtime's own failures.
//!
//! The runtime is not ours: Monado or WiVRn can be absent when the extension starts, refuse the session, lose it, or
//! be restarted under it. None of that may take the extension down, because the guardian gives a program that keeps
//! exiting only five restarts. So [`supervise`] runs one session after another against the same owner connection. A
//! session that cannot start, or that ends because the runtime failed or asked the program to exit, is followed by a
//! wait and a new attempt. The waits double from 250 ms to 4 s, as the guardian's do, and start again after a session
//! that stayed up for [`HEALTHY_AFTER`].
//!
//! Only the owner ends the loop. When the owner socket closes the extension has nothing left to show, so it returns;
//! when the owner socket fails it returns the error, and the guardian starts the program again.
//!
//! While it waits, [`wait`] keeps reading the owner socket, so the model stays current and a closed socket is seen at
//! once.

use std::thread;
use std::time::{Duration, Instant};

use xr_tree::{TreeClient, TreeError};

use crate::bridge::Outcome;

/// The wait before the first new attempt.
pub const INITIAL_WAIT: Duration = Duration::from_millis(250);
/// The longest wait between attempts.
pub const LONGEST_WAIT: Duration = Duration::from_secs(4);
/// How long a session must stay up for the waits to start again from the shortest.
pub const HEALTHY_AFTER: Duration = Duration::from_secs(30);
/// How often [`wait`] reads the owner socket while it waits.
const POLL_EVERY: Duration = Duration::from_millis(20);

/// Why an attempt failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The OpenXR runtime could not give a session, or failed one it gave. The next attempt may succeed.
    Runtime(String),
    /// The owner socket failed. No attempt can help.
    Owner(String),
}

/// The waits between attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    initial: Duration,
    longest: Duration,
    healthy_after: Duration,
    next: Duration,
}

impl Backoff {
    /// Waits that start at `initial`, double up to `longest`, and start again after a session of `healthy_after`.
    #[must_use]
    pub const fn new(initial: Duration, longest: Duration, healthy_after: Duration) -> Self {
        Self {
            initial,
            longest,
            healthy_after,
            next: initial,
        }
    }

    /// The wait to use now, which doubles the one after it up to the longest.
    fn take(&mut self) -> Duration {
        let wait = self.next;
        self.next = (self.next * 2).min(self.longest);
        wait
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new(INITIAL_WAIT, LONGEST_WAIT, HEALTHY_AFTER)
    }
}

/// Runs `attempt` until the owner closes the socket or fails it.
///
/// `attempt` makes one session over the owner connection and runs it to its end. `pause` waits for the time it is
/// given and reports whether the owner is still there, as [`wait`] does. `say` is told what to log: a reason is said
/// once, however many attempts fail the same way, and again after a session that stayed up.
///
/// # Errors
///
/// The owner socket failed, with its message.
pub fn supervise(
    tree: &mut TreeClient,
    backoff: &mut Backoff,
    attempt: &mut impl FnMut(&mut TreeClient) -> Result<Outcome, Failure>,
    pause: &mut impl FnMut(&mut TreeClient, Duration) -> Result<bool, TreeError>,
    say: &mut impl FnMut(&str),
) -> Result<(), String> {
    let mut said: Option<String> = None;
    loop {
        let started = Instant::now();
        let reason = match attempt(tree) {
            Ok(Outcome::OwnerClosed) => return Ok(()),
            Err(Failure::Owner(message)) => return Err(message),
            Ok(Outcome::SessionExited) => "the OpenXR session ended".to_owned(),
            Err(Failure::Runtime(message)) => message,
        };
        if started.elapsed() >= backoff.healthy_after {
            backoff.next = backoff.initial;
            said = None;
        }
        let delay = backoff.take();
        if said.as_deref() != Some(reason.as_str()) {
            say(&format!("{reason}; trying again in {} ms", delay.as_millis()));
            said = Some(reason);
        }
        match pause(tree, delay) {
            Ok(true) => {}
            Ok(false) => return Ok(()),
            Err(error) => return Err(format!("owner socket: {error}")),
        }
    }
}

/// Waits `total` while reading the owner socket. `Ok(false)` as soon as the owner has closed it.
///
/// # Errors
///
/// The owner socket failed.
pub fn wait(tree: &mut TreeClient, total: Duration) -> Result<bool, TreeError> {
    let end = Instant::now() + total;
    loop {
        match tree.poll() {
            Ok(_) => {}
            Err(TreeError::Closed) => return Ok(false),
            Err(error) => return Err(error),
        }
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(true);
        }
        thread::sleep(left.min(POLL_EVERY));
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;

    use gnoblin_proto::tree::tree::{AttachedPayload, SnapshotPayload};
    use gnoblin_proto::{Envelope, read_frame, write_frame};

    use super::*;

    /// A tree client attached to an owner that sends an empty snapshot and then holds the socket open until the
    /// returned stream is dropped.
    fn attached() -> (TreeClient, UnixStream) {
        let (client, mut server) = UnixStream::pair().expect("socket pair");
        let keep = server.try_clone().expect("a second handle");
        thread::spawn(move || {
            let attach = read_frame(&mut server).expect("read attach").expect("attach frame");
            let events = [
                Envelope::reply_message(
                    "tree",
                    "attach",
                    attach.request_id,
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
                        nodes: Vec::new(),
                    },
                ),
            ];
            for event in &events {
                if write_frame(&mut server, event).is_err() {
                    return;
                }
            }
            // Read until the client hangs up, so the socket stays open.
            while let Ok(Some(_)) = read_frame(&mut server) {}
        });
        (TreeClient::from_stream(client).expect("attach"), keep)
    }

    /// Runs `supervise` with the given results for the attempts and records the waits and the log lines.
    fn run(
        results: Vec<Result<Outcome, Failure>>,
        mut backoff: Backoff,
        owner_present_after_pause: bool,
    ) -> (Result<(), String>, Vec<Duration>, Vec<String>, usize) {
        let (mut tree, _owner) = attached();
        let mut results = results.into_iter();
        let mut attempts = 0;
        let mut waits = Vec::new();
        let mut lines = Vec::new();
        let outcome = supervise(
            &mut tree,
            &mut backoff,
            &mut |_| {
                attempts += 1;
                results.next().expect("the test scripted enough attempts")
            },
            &mut |_, wait| {
                waits.push(wait);
                Ok(owner_present_after_pause)
            },
            &mut |line| lines.push(line.to_owned()),
        );
        (outcome, waits, lines, attempts)
    }

    fn refused() -> Result<Outcome, Failure> {
        Err(Failure::Runtime("the runtime refused the session".to_owned()))
    }

    #[test]
    fn a_runtime_that_refuses_the_session_is_tried_again_with_growing_waits_and_logged_once() {
        let (outcome, waits, lines, attempts) = run(
            vec![refused(), refused(), refused(), refused(), Ok(Outcome::OwnerClosed)],
            Backoff::default(),
            true,
        );
        assert_eq!(outcome, Ok(()));
        assert_eq!(attempts, 5);
        let ms: Vec<u128> = waits.iter().map(Duration::as_millis).collect();
        assert_eq!(ms, vec![250, 500, 1000, 2000], "each wait doubles");
        assert_eq!(lines.len(), 1, "the same refusal is said once: {lines:?}");
        assert!(lines[0].contains("the runtime refused the session"), "{lines:?}");
    }

    #[test]
    fn the_waits_stop_doubling_at_the_longest() {
        let mut results: Vec<_> = (0..8).map(|_| refused()).collect();
        results.push(Ok(Outcome::OwnerClosed));
        let (_, waits, _, _) = run(results, Backoff::default(), true);
        let ms: Vec<u128> = waits.iter().map(Duration::as_millis).collect();
        assert_eq!(ms, vec![250, 500, 1000, 2000, 4000, 4000, 4000, 4000]);
    }

    #[test]
    fn a_session_the_runtime_ended_is_followed_by_a_new_one() {
        let (outcome, waits, lines, attempts) = run(
            vec![Ok(Outcome::SessionExited), Ok(Outcome::OwnerClosed)],
            Backoff::default(),
            true,
        );
        assert_eq!(outcome, Ok(()));
        assert_eq!((attempts, waits.len()), (2, 1));
        assert!(lines[0].contains("the OpenXR session ended"), "{lines:?}");
    }

    #[test]
    fn a_new_reason_is_said_and_an_old_one_is_not_said_again() {
        let lost = || Err(Failure::Runtime("the session was lost".to_owned()));
        let (_, _, lines, _) = run(
            vec![refused(), lost(), lost(), refused(), Ok(Outcome::OwnerClosed)],
            Backoff::default(),
            true,
        );
        assert_eq!(
            lines.len(),
            3,
            "refused, lost, then refused again after a different reason: {lines:?}"
        );
    }

    #[test]
    fn a_session_that_stays_up_starts_the_waits_and_the_log_again() {
        // Healthy after 50 ms: the attempt that sleeps twice that long ends the run of failures.
        let backoff = Backoff::new(
            Duration::from_millis(250),
            Duration::from_secs(4),
            Duration::from_millis(50),
        );
        let (mut tree, _owner) = attached();
        let mut backoff = backoff;
        let mut step = 0;
        let mut waits = Vec::new();
        let mut lines = Vec::new();
        let outcome = supervise(
            &mut tree,
            &mut backoff,
            &mut |_| {
                step += 1;
                match step {
                    1 | 2 => refused(),
                    3 => {
                        thread::sleep(Duration::from_millis(100));
                        refused()
                    }
                    _ => Ok(Outcome::OwnerClosed),
                }
            },
            &mut |_, wait| {
                waits.push(wait.as_millis());
                Ok(true)
            },
            &mut |line| lines.push(line.to_owned()),
        );
        assert_eq!(outcome, Ok(()));
        assert_eq!(
            waits,
            vec![250, 500, 250],
            "the healthy session sent the waits back to the shortest"
        );
        assert_eq!(lines.len(), 2, "the same refusal is said again after a healthy session");
    }

    #[test]
    fn an_owner_failure_stops_the_loop_without_a_new_attempt() {
        let (outcome, waits, _, attempts) = run(
            vec![Err(Failure::Owner("owner socket: broken".to_owned()))],
            Backoff::default(),
            true,
        );
        assert_eq!(outcome, Err("owner socket: broken".to_owned()));
        assert_eq!((attempts, waits.len()), (1, 0));
    }

    #[test]
    fn an_owner_that_closes_during_the_wait_ends_the_loop() {
        let (outcome, waits, _, attempts) = run(vec![refused()], Backoff::default(), false);
        assert_eq!(outcome, Ok(()));
        assert_eq!((attempts, waits.len()), (1, 1));
    }

    #[test]
    fn waiting_returns_false_the_moment_the_owner_hangs_up_and_true_when_the_time_is_up() {
        let (mut tree, owner) = attached();
        assert!(wait(&mut tree, Duration::from_millis(30)).expect("the owner is there"));
        owner.shutdown(std::net::Shutdown::Both).expect("hang up");
        let started = Instant::now();
        assert!(!wait(&mut tree, Duration::from_secs(5)).expect("a closed socket is not an error"));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "it did not wait out the time"
        );
    }
}
