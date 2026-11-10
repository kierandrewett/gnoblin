//! Password checks for the session lock.
//!
//! The lock asks an [`Authenticator`] whether a user name and password are good. The answer is an
//! [`AuthOutcome`] with three cases, and only one of them lets the session open:
//!
//! - [`AuthOutcome::Accepted`]: PAM vouched for the password and the account.
//! - [`AuthOutcome::Denied`]: PAM ran and refused the password, the user, or the account.
//! - [`AuthOutcome::Failed`]: PAM could not be run at all (no libpam, no service, a broken stack).
//!   The lock must stay locked: this is not a wrong password, and it is not a grant either.
//!
//! [`PamAuthenticator`] is the real check. [`FakeAuthenticator`] scripts outcomes for tests, and
//! [`RateLimiter`] slows repeated wrong passwords down. The password never reaches a log line, an
//! error message or a `Debug` string: no type in this crate formats the password it was given.

mod fake;
mod ffi;
mod pam;
mod ratelimit;

pub use fake::FakeAuthenticator;
pub use pam::{DEFAULT_SERVICE, PamAuthenticator};
pub use ratelimit::{DELAY_MS, DENIALS_BEFORE_DELAY, RateLimiter};

/// The answer to one password check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthOutcome {
    /// The password and the account are good: the lock may open.
    Accepted,
    /// The password, the user name or the account was refused.
    Denied,
    /// The check could not be run. The message names what went wrong, never the password.
    Failed(String),
}

/// Decides whether a user name and password open the session.
///
/// Implementations may block while PAM runs, so the caller keeps the lock responsive by asking from
/// a worker. The trait takes `&self`, so one authenticator serves every attempt.
pub trait Authenticator: Send + Sync {
    /// Check `password` for `user`. Returns [`AuthOutcome::Accepted`] only when the check passed.
    fn authenticate(&self, user: &str, password: &str) -> AuthOutcome;
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "correct-horse-battery";

    #[test]
    fn outcome_debug_never_shows_the_password() {
        let auth = FakeAuthenticator::new().accept("kieran", SECRET);
        for password in [SECRET, "wrong"] {
            let outcome = auth.authenticate("kieran", password);
            assert!(!format!("{outcome:?}").contains(SECRET));
        }
        let failed = FakeAuthenticator::new()
            .fail("libpam is missing")
            .authenticate("kieran", SECRET);
        assert!(!format!("{failed:?}").contains(SECRET));
    }

    #[test]
    fn authenticator_debug_never_shows_the_password() {
        let fake = FakeAuthenticator::new().accept("kieran", SECRET);
        assert!(!format!("{fake:?}").contains(SECRET));
        let pam = PamAuthenticator::default();
        assert!(!format!("{pam:?}").contains(SECRET));
    }

    #[test]
    fn the_trait_objects_can_cross_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<PamAuthenticator>();
        assert_send_sync::<FakeAuthenticator>();
        let shared: Box<dyn Authenticator> = Box::new(FakeAuthenticator::new());
        let outcome = std::thread::spawn(move || shared.authenticate("kieran", "pw"))
            .join()
            .expect("worker thread finishes");
        assert_eq!(outcome, AuthOutcome::Denied);
    }
}
