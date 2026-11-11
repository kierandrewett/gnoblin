//! The lock's password check: an `authenticate` call from the shell is answered here, never by the
//! shell. The compositor runs the check on a worker thread, so the lock stays responsive while PAM
//! runs, and it unlocks only on an accepted answer. Everything else keeps the session locked.

use std::sync::Arc;
use std::thread;
use std::time::Instant;

use gnoblin_lock_auth::{AuthOutcome, Authenticator, RateLimiter};
use gnoblin_secret::SecretText;
use smithay::reexports::calloop::channel::{self, Channel, Sender};

use crate::tree_frame::TreeAnswer;

/// The worker's answer to one check, delivered back to the event loop.
pub struct AuthDone {
    pub token: u64,
    pub outcome: AuthOutcome,
}

/// What to tell the shell for a settled check, and whether the session may unlock now.
#[derive(Debug, PartialEq, Eq)]
pub struct LockSettlement {
    pub answer: TreeAnswer,
    pub unlock: bool,
}

/// Owns the checks in flight. At most one runs at a time; the limiter counts denials across them.
pub struct LockAuth {
    authenticator: Arc<dyn Authenticator>,
    user: Option<String>,
    limiter: RateLimiter,
    in_flight: Option<u64>,
    started: Instant,
    done: Sender<AuthDone>,
}

impl LockAuth {
    /// A checker for `user`'s password. The channel carries each finished check back to the loop.
    /// With no `user` every attempt fails closed.
    pub fn new(authenticator: Arc<dyn Authenticator>, user: Option<String>) -> (Self, Channel<AuthDone>) {
        let (done, results) = channel::channel();
        let auth = Self {
            authenticator,
            user,
            limiter: RateLimiter::new(),
            in_flight: None,
            started: Instant::now(),
            done,
        };
        (auth, results)
    }

    /// The monotonic clock the limiter reads, in milliseconds since this checker was made.
    pub fn now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Start a check of `password` when the session is `locked`. Returns the answer at once when
    /// the attempt is refused (not locked, a check already running, rate-limited, or no user);
    /// otherwise the check runs on a worker and its answer comes through [`Self::finish`].
    pub fn begin(&mut self, token: u64, password: SecretText, locked: bool, now_ms: u64) -> Option<TreeAnswer> {
        if !locked {
            return Some(refused(token, "not-locked"));
        }
        if self.in_flight.is_some() {
            return Some(refused(token, "busy"));
        }
        if !self.limiter.allows(now_ms) {
            return Some(TreeAnswer {
                token,
                result: self.limiter.retry_after_ms(now_ms).to_string(),
                error: "rate-limited".to_owned(),
            });
        }
        let Some(user) = self.user.clone() else {
            return Some(refused(token, "failed: no user to check the password for"));
        };
        self.in_flight = Some(token);
        let authenticator = Arc::clone(&self.authenticator);
        let done = self.done.clone();
        thread::spawn(move || {
            let outcome = authenticator.authenticate(&user, password.expose());
            drop(password);
            // The loop may be gone at shutdown; then nobody is left to answer.
            let _ = done.send(AuthDone { token, outcome });
        });
        None
    }

    /// Settle a finished check. `locked` is whether the session is still locked now. Returns
    /// `None` for a check that is not the one in flight.
    pub fn finish(&mut self, done: AuthDone, locked: bool, now_ms: u64) -> Option<LockSettlement> {
        if self.in_flight != Some(done.token) {
            return None;
        }
        self.in_flight = None;
        if !locked {
            return Some(LockSettlement {
                answer: refused(done.token, "not-locked"),
                unlock: false,
            });
        }
        let settled = match done.outcome {
            AuthOutcome::Accepted => {
                self.limiter.record(&AuthOutcome::Accepted, now_ms);
                LockSettlement {
                    answer: TreeAnswer {
                        token: done.token,
                        result: "ok".to_owned(),
                        error: String::new(),
                    },
                    unlock: true,
                }
            }
            AuthOutcome::Denied => {
                self.limiter.record(&AuthOutcome::Denied, now_ms);
                LockSettlement {
                    answer: refused(done.token, "denied"),
                    unlock: false,
                }
            }
            AuthOutcome::Failed(message) => LockSettlement {
                answer: refused(done.token, &format!("failed: {message}")),
                unlock: false,
            },
        };
        Some(settled)
    }
}

/// An answer that refuses: an empty result and the reason as the error.
fn refused(token: u64, error: &str) -> TreeAnswer {
    TreeAnswer {
        token,
        result: String::new(),
        error: error.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gnoblin_lock_auth::{DELAY_MS, DENIALS_BEFORE_DELAY, FakeAuthenticator};
    use smithay::reexports::calloop::EventLoop;

    use super::*;

    const USER: &str = "kieran";
    const PASSWORD: &str = "correct horse";

    fn accepting() -> Arc<dyn Authenticator> {
        Arc::new(FakeAuthenticator::new().accept(USER, PASSWORD))
    }

    /// The loop that carries a checker's answers, run the way the server runs it.
    struct Answers {
        event_loop: EventLoop<'static, Vec<AuthDone>>,
        received: Vec<AuthDone>,
    }

    impl Answers {
        fn new(results: Channel<AuthDone>) -> Self {
            let event_loop = EventLoop::try_new().expect("an event loop");
            event_loop
                .handle()
                .insert_source(results, |event, _, received: &mut Vec<AuthDone>| {
                    if let channel::Event::Msg(done) = event {
                        received.push(done);
                    }
                })
                .expect("the channel source");
            Self {
                event_loop,
                received: Vec::new(),
            }
        }

        /// Run the loop until the next check has finished, and return it.
        fn next(&mut self) -> AuthDone {
            for _ in 0..500 {
                if !self.received.is_empty() {
                    return self.received.remove(0);
                }
                self.event_loop
                    .dispatch(Some(Duration::from_millis(10)), &mut self.received)
                    .expect("dispatch");
            }
            panic!("the worker never answered");
        }
    }

    #[test]
    fn a_check_while_not_locked_is_refused_without_a_worker() {
        let (mut auth, _results) = LockAuth::new(accepting(), Some(USER.to_owned()));
        let answer = auth.begin(7, SecretText::new(PASSWORD), false, 0).expect("refused");
        assert_eq!(answer, refused(7, "not-locked"));
    }

    #[test]
    fn the_right_password_settles_ok_and_unlocks() {
        let (mut auth, results) = LockAuth::new(accepting(), Some(USER.to_owned()));
        let mut answers = Answers::new(results);
        assert!(auth.begin(1, SecretText::new(PASSWORD), true, 0).is_none());
        let done = answers.next();
        assert_eq!(done.token, 1);
        let settled = auth.finish(done, true, 0).expect("settles");
        assert_eq!(
            settled,
            LockSettlement {
                answer: TreeAnswer {
                    token: 1,
                    result: "ok".to_owned(),
                    error: String::new(),
                },
                unlock: true,
            }
        );
    }

    #[test]
    fn the_wrong_password_is_denied_and_keeps_the_lock() {
        let (mut auth, results) = LockAuth::new(accepting(), Some(USER.to_owned()));
        let mut answers = Answers::new(results);
        assert!(auth.begin(2, SecretText::new("wrong"), true, 0).is_none());
        let settled = auth.finish(answers.next(), true, 0).expect("settles");
        assert_eq!(settled.answer, refused(2, "denied"));
        assert!(!settled.unlock);
    }

    #[test]
    fn a_second_check_while_one_runs_is_busy() {
        let (mut auth, _results) = LockAuth::new(accepting(), Some(USER.to_owned()));
        assert!(auth.begin(3, SecretText::new(PASSWORD), true, 0).is_none());
        let busy = auth.begin(4, SecretText::new(PASSWORD), true, 0).expect("refused");
        assert_eq!(busy, refused(4, "busy"));
    }

    #[test]
    fn a_failed_check_names_the_failure_and_keeps_the_lock() {
        let failing: Arc<dyn Authenticator> = Arc::new(FakeAuthenticator::new().fail("libpam is missing"));
        let (mut auth, results) = LockAuth::new(failing, Some(USER.to_owned()));
        let mut answers = Answers::new(results);
        assert!(auth.begin(5, SecretText::new(PASSWORD), true, 0).is_none());
        let settled = auth.finish(answers.next(), true, 0).expect("settles");
        assert_eq!(settled.answer.error, "failed: libpam is missing");
        assert!(!settled.unlock);
    }

    #[test]
    fn no_user_fails_closed_without_a_worker() {
        let (mut auth, _results) = LockAuth::new(accepting(), None);
        let answer = auth.begin(6, SecretText::new(PASSWORD), true, 0).expect("refused");
        assert!(answer.error.starts_with("failed:"), "{answer:?}");
        assert!(answer.result.is_empty());
    }

    #[test]
    fn repeated_denials_rate_limit_the_next_attempt() {
        let (mut auth, results) = LockAuth::new(accepting(), Some(USER.to_owned()));
        let mut answers = Answers::new(results);
        let mut now = 0;
        for token in 0..u64::from(DENIALS_BEFORE_DELAY) {
            assert!(auth.begin(token, SecretText::new("wrong"), true, now).is_none());
            let settled = auth.finish(answers.next(), true, now).expect("settles");
            assert_eq!(settled.answer.error, "denied");
            now += 1;
        }
        // The last denial was at `now - 1`; the delay runs from there.
        let last_denial = now - 1;
        let limited = auth
            .begin(99, SecretText::new(PASSWORD), true, now)
            .expect("rate-limited");
        assert_eq!(limited.error, "rate-limited");
        assert_eq!(limited.result, (last_denial + DELAY_MS - now).to_string());
    }

    #[test]
    fn a_finish_for_a_check_not_in_flight_is_ignored() {
        let (mut auth, _results) = LockAuth::new(accepting(), Some(USER.to_owned()));
        let stray = AuthDone {
            token: 42,
            outcome: AuthOutcome::Accepted,
        };
        assert!(auth.finish(stray, true, 0).is_none());
    }

    #[test]
    fn a_check_that_settles_after_the_lock_is_gone_does_not_unlock() {
        let (mut auth, results) = LockAuth::new(accepting(), Some(USER.to_owned()));
        let mut answers = Answers::new(results);
        assert!(auth.begin(8, SecretText::new(PASSWORD), true, 0).is_none());
        let settled = auth.finish(answers.next(), false, 0).expect("settles");
        assert_eq!(settled.answer.error, "not-locked");
        assert!(!settled.unlock);
    }
}
