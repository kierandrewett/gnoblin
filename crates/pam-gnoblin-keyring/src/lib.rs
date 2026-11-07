//! `pam_gnoblin_keyring`: the PAM module that hands the login password to the Gnoblin keyring
//! service. See `design/keyring.md` "Key source" and "Failure modes".
//!
//! The module never fails a login and never grants one. Every auth-type entry point returns
//! `PAM_IGNORE`, so a `sufficient` entry cannot be the only check. A hand-off that fails leaves the
//! service to prompt once, as it does when there is no password at all.

mod ffi;
mod handoff;

// Re-exported so the cdylib exports the PAM entry points by name.
pub use ffi::{
    pam_sm_acct_mgmt, pam_sm_authenticate, pam_sm_chauthtok, pam_sm_close_session, pam_sm_open_session, pam_sm_setcred,
};
