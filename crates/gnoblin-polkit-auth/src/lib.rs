//! polkit authentication that no script can decide.
//!
//! polkitd asks the session's agent to authenticate somebody (`BeginAuthentication`). The agent does not
//! check a password itself, and neither does anything that draws a prompt: it starts polkit's own setuid
//! helper, `polkit-agent-helper-1`, which runs the PAM stack, and which, only when PAM says yes, tells polkitd
//! itself with `AuthenticationAgentResponse2` before it prints `SUCCESS` (design/js-research/polkit.md). This
//! crate is the compositor's half of that conversation:
//!
//! - [`AuthRequest`] is what the services role's agent hands the compositor for one `BeginAuthentication`: the
//!   action, the message, the identities and the cookie. It travels in the offer's subject, and an observer
//!   sees [`AuthRequest::observed`], which leaves the cookie out.
//! - [`HelperSession`] runs the helper for one identity and one cookie, reads its PAM prompts back as
//!   [`HelperEvent`]s and feeds it the answers as [`SecretText`](gnoblin_secret::SecretText), which is wiped when
//!   it drops. Only [`HelperEvent::Success`], which is the helper's own `SUCCESS` line, ever means the user
//!   authenticated.
//! - [`locate_helper`] finds the helper, with an environment override for tests.
//!
//! Nothing here knows a script, a slot or a dialog. A password is never formatted, logged or kept: the only
//! place it is read is the write to the helper's standard input.

#![forbid(unsafe_code)]

mod helper;
mod request;
mod user;

pub use gnoblin_secret::SecretText;
pub use helper::{HELPER_CANDIDATES, HELPER_ENV, HelperError, HelperEvent, HelperSession, locate_helper, parse_line};
pub use request::{AuthRequest, Identity, MAX_PAYLOAD, Observed, PayloadError};
pub use user::{current_uid, name_of, parse_passwd};

gnoblin_log::logger!(log = "polkit-auth");
