//! A scripted authenticator for tests. It answers from a table and records which users were asked
//! about, so the lock's behaviour can be tested without PAM.

use std::fmt;
use std::sync::Mutex;

use crate::{AuthOutcome, Authenticator};

/// Answers password checks from a script.
///
/// A `(user, password)` pair listed with [`FakeAuthenticator::accept`] gives
/// [`AuthOutcome::Accepted`]. [`FakeAuthenticator::fail`] makes every check return
/// [`AuthOutcome::Failed`]. Anything else is [`AuthOutcome::Denied`]. Each call records the user
/// name, never the password.
pub struct FakeAuthenticator {
    accepted: Vec<(String, String)>,
    failure: Option<String>,
    calls: Mutex<Vec<String>>,
}

impl FakeAuthenticator {
    /// A fake that denies every password.
    pub fn new() -> Self {
        Self {
            accepted: Vec::new(),
            failure: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Accept `password` for `user`.
    #[must_use]
    pub fn accept(mut self, user: &str, password: &str) -> Self {
        self.accepted.push((user.to_owned(), password.to_owned()));
        self
    }

    /// Make every check return [`AuthOutcome::Failed`] with `message`.
    #[must_use]
    pub fn fail(mut self, message: &str) -> Self {
        self.failure = Some(message.to_owned());
        self
    }

    /// The user names checked so far, in call order. Passwords are never recorded.
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().map(|calls| calls.clone()).unwrap_or_default()
    }
}

impl Default for FakeAuthenticator {
    fn default() -> Self {
        Self::new()
    }
}

impl Authenticator for FakeAuthenticator {
    fn authenticate(&self, user: &str, password: &str) -> AuthOutcome {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(user.to_owned());
        }
        if let Some(message) = &self.failure {
            return AuthOutcome::Failed(message.clone());
        }
        let accepted = self.accepted.iter().any(|(u, p)| u == user && p == password);
        if accepted {
            AuthOutcome::Accepted
        } else {
            AuthOutcome::Denied
        }
    }
}

impl fmt::Debug for FakeAuthenticator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FakeAuthenticator")
            .field("accepted_users", &self.accepted.len())
            .field("failure", &self.failure)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_the_scripted_user_and_password() {
        let fake = FakeAuthenticator::new().accept("kieran", "hunter2");
        assert_eq!(fake.authenticate("kieran", "hunter2"), AuthOutcome::Accepted);
        assert_eq!(fake.authenticate("kieran", "wrong"), AuthOutcome::Denied);
        assert_eq!(fake.authenticate("root", "hunter2"), AuthOutcome::Denied);
    }

    #[test]
    fn denies_everything_when_nothing_is_scripted() {
        let fake = FakeAuthenticator::new();
        assert_eq!(fake.authenticate("kieran", ""), AuthOutcome::Denied);
    }

    #[test]
    fn several_accepted_pairs_can_be_scripted() {
        let fake = FakeAuthenticator::new().accept("a", "one").accept("b", "two");
        assert_eq!(fake.authenticate("a", "one"), AuthOutcome::Accepted);
        assert_eq!(fake.authenticate("b", "two"), AuthOutcome::Accepted);
        assert_eq!(fake.authenticate("a", "two"), AuthOutcome::Denied);
    }

    #[test]
    fn fail_turns_every_check_into_a_failure_with_the_message() {
        let fake = FakeAuthenticator::new()
            .accept("kieran", "hunter2")
            .fail("pam is not installed");
        assert_eq!(
            fake.authenticate("kieran", "hunter2"),
            AuthOutcome::Failed("pam is not installed".to_owned())
        );
    }

    #[test]
    fn records_each_user_in_call_order_without_passwords() {
        let fake = FakeAuthenticator::new().accept("kieran", "hunter2");
        let _ = fake.authenticate("kieran", "hunter2");
        let _ = fake.authenticate("root", "secret-value");
        assert_eq!(fake.calls(), vec!["kieran".to_owned(), "root".to_owned()]);
        let recorded = format!("{:?}", fake.calls());
        assert!(!recorded.contains("hunter2"));
        assert!(!recorded.contains("secret-value"));
    }

    #[test]
    fn debug_output_never_contains_the_scripted_password() {
        let fake = FakeAuthenticator::new().accept("kieran", "hunter2").fail("boom");
        let shown = format!("{fake:?}");
        assert!(!shown.contains("hunter2"));
        assert!(!shown.contains("kieran"));
    }
}
