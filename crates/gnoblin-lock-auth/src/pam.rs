//! The real password check: PAM, run as the `gnoblin-lock` service by default.
//!
//! The service's stack is shipped as `pam.d/gnoblin-lock` under the install prefix's `lib` and
//! reads the system `login` stack, so the lock accepts what a console login accepts.

use std::ffi::CString;

use crate::{AuthOutcome, Authenticator, ffi};

/// The PAM service name the lock uses unless told otherwise.
pub const DEFAULT_SERVICE: &str = "gnoblin-lock";

const PAM_SUCCESS: i32 = 0;
const PAM_AUTH_ERR: i32 = 7;
const PAM_USER_UNKNOWN: i32 = 10;
const PAM_MAXTRIES: i32 = 11;

/// Checks passwords through a PAM service.
///
/// The check runs `pam_authenticate` and then `pam_acct_mgmt`, so a locked or expired account is
/// refused even with the right password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PamAuthenticator {
    service: String,
}

impl PamAuthenticator {
    /// An authenticator that checks against the PAM service named `service`.
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }
}

impl Default for PamAuthenticator {
    fn default() -> Self {
        Self::new(DEFAULT_SERVICE)
    }
}

impl Authenticator for PamAuthenticator {
    fn authenticate(&self, user: &str, password: &str) -> AuthOutcome {
        let Ok(service) = CString::new(self.service.as_str()) else {
            return AuthOutcome::Failed("the PAM service name contains a NUL byte".to_owned());
        };
        // A NUL cannot name a user or be a password, and C strings cannot carry one: refuse
        // without asking PAM.
        let Ok(user_c) = CString::new(user) else {
            return AuthOutcome::Denied;
        };
        if password.contains('\0') {
            return AuthOutcome::Denied;
        }
        outcome_from_code(ffi::authenticate(&service, &user_c, password.as_bytes()))
    }
}

/// Map a PAM return code (or a failure to reach PAM) to an outcome.
fn outcome_from_code(result: Result<i32, String>) -> AuthOutcome {
    match result {
        Ok(PAM_SUCCESS) => AuthOutcome::Accepted,
        Ok(PAM_AUTH_ERR | PAM_USER_UNKNOWN | PAM_MAXTRIES) => AuthOutcome::Denied,
        Ok(code) => AuthOutcome::Failed(format!("PAM returned error {code}")),
        Err(message) => AuthOutcome::Failed(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_service_is_the_shipped_gnoblin_lock_stack() {
        assert_eq!(PamAuthenticator::default().service, "gnoblin-lock");
        assert_eq!(DEFAULT_SERVICE, "gnoblin-lock");
    }

    #[test]
    fn success_is_accepted() {
        assert_eq!(outcome_from_code(Ok(0)), AuthOutcome::Accepted);
    }

    #[test]
    fn wrong_password_unknown_user_and_too_many_tries_are_denied() {
        for code in [7, 10, 11] {
            assert_eq!(outcome_from_code(Ok(code)), AuthOutcome::Denied, "code {code}");
        }
    }

    #[test]
    fn other_codes_fail_closed_with_the_code_in_the_message() {
        assert_eq!(
            outcome_from_code(Ok(4)),
            AuthOutcome::Failed("PAM returned error 4".to_owned())
        );
    }

    #[test]
    fn a_pam_that_cannot_be_reached_fails_closed() {
        assert_eq!(
            outcome_from_code(Err("cannot load libpam.so.0".to_owned())),
            AuthOutcome::Failed("cannot load libpam.so.0".to_owned())
        );
    }

    #[test]
    fn a_nul_in_the_user_name_is_denied_without_asking_pam() {
        let auth = PamAuthenticator::default();
        assert_eq!(auth.authenticate("kieran\0root", "pw"), AuthOutcome::Denied);
    }

    #[test]
    fn a_nul_in_the_password_is_denied_without_asking_pam() {
        let auth = PamAuthenticator::default();
        assert_eq!(auth.authenticate("kieran", "pass\0word"), AuthOutcome::Denied);
    }

    #[test]
    fn a_nul_in_the_service_name_fails_closed() {
        let auth = PamAuthenticator::new("gnoblin\0lock");
        assert!(matches!(auth.authenticate("kieran", "pw"), AuthOutcome::Failed(_)));
    }

    #[test]
    fn a_failure_message_never_contains_the_password() {
        let auth = PamAuthenticator::new("gnoblin\0lock");
        let outcome = auth.authenticate("kieran", "s3cret-passphrase");
        assert!(!format!("{outcome:?}").contains("s3cret-passphrase"));
    }

    /// Needs a real PAM stack and an account to test against: run with
    /// `GNOBLIN_PAM_TEST=1 cargo test -p gnoblin-lock-auth -- --ignored`. The user comes from
    /// `GNOBLIN_PAM_TEST_USER` (default `USER`) and the service from `GNOBLIN_PAM_TEST_SERVICE`
    /// (default `gnoblin-lock`). A wrong password must be denied.
    #[test]
    #[ignore = "needs libpam and GNOBLIN_PAM_TEST=1"]
    fn real_pam_denies_a_wrong_password() {
        if std::env::var("GNOBLIN_PAM_TEST").as_deref() != Ok("1") {
            return;
        }
        let user = std::env::var("GNOBLIN_PAM_TEST_USER")
            .or_else(|_| std::env::var("USER"))
            .expect("GNOBLIN_PAM_TEST_USER or USER names the test account");
        let service = std::env::var("GNOBLIN_PAM_TEST_SERVICE").unwrap_or_else(|_| DEFAULT_SERVICE.to_owned());
        let auth = PamAuthenticator::new(service);
        let outcome = auth.authenticate(&user, "gnoblin-wrong-password-for-test");
        assert_eq!(outcome, AuthOutcome::Denied);
    }
}
