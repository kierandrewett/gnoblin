//! The compositor's polkit authentication: the dialog's card, and the helper it runs for whoever the user is.
//!
//! polkitd asks the services role's agent to authenticate somebody, and the agent offers the request to the
//! compositor on `auth.polkit` ([`super::requests`]). Nothing that draws, and nothing a script wrote, decides
//! whether the password was right: [`PolkitDialog`] collects it in [`PolkitUi`], runs polkit's own
//! `polkit-agent-helper-1` for the identity the user chose with the request's cookie, shows the PAM prompts the
//! helper sends back, and reports [`Outcome::Authenticated`] only when the helper itself says `SUCCESS`. The helper
//! has by then told polkitd, which is the authority (design/js-research/polkit.md).
//!
//! [`PolkitDialog`] never touches a socket or the screen. [`super::secret::SecretService`] gives it keys and the
//! clock and draws its card; it returns what the dialog ended in.
//!
//! # Failure modes first
//!
//! 1. **The helper is missing or cannot start.** The card says why and offers Close; the request ends unauthenticated
//!    when the user closes it, or at its deadline: `a_missing_helper_is_said_and_fails_closed`.
//! 2. **The password is wrong.** The helper's own message is shown and a new helper starts for the same identity,
//!    up to [`MAX_FAILURES`] times, after which the card offers only Close:
//!    `a_wrong_password_shows_the_helpers_message_and_asks_again`, `too_many_failures_end_the_attempt`.
//! 3. **The user cancels, or the dialog is dropped** (the request timed out, polkitd cancelled it, the session
//!    locked). The helper is killed and reaped with its session: `cancel_kills_the_helper`.
//! 4. **The password leaks.** It is a [`gnoblin_secret::SecretText`] from the first key to the helper's input; no
//!    line here logs more than a request-level event: `nothing_the_dialog_formats_holds_the_password`.

use std::path::PathBuf;

use gnoblin_polkit_auth::{AuthRequest, HelperError, HelperEvent, HelperSession, Identity, current_uid, locate_helper};
use gnoblin_portal::{PolkitEvent, PolkitUi};
use gnoblin_portal_core::model::ScreenKey;

/// How many attempts may fail before the dialog stops asking. PAM's own lockout (faillock) usually acts first; this
/// keeps a stack that never locks out from being guessed at through an open dialog.
pub(crate) const MAX_FAILURES: u32 = 5;

/// What the dialog says after a failure the helper gave no message for.
const TRY_AGAIN: &str = "Sorry, that didn't work. Please try again.";

/// Where the helper is: a path a test or a distribution fixed, or wherever [`locate_helper`] finds it.
#[derive(Clone, Debug, Default)]
pub(crate) struct HelperPath(Option<PathBuf>);

impl HelperPath {
    /// A helper at `path`, whether or not it exists: a missing one is the dialog's error to show.
    pub(crate) fn fixed(path: PathBuf) -> Self {
        Self(Some(path))
    }

    fn locate(&self) -> Result<PathBuf, HelperError> {
        match &self.0 {
            Some(path) => Ok(path.clone()),
            None => locate_helper(),
        }
    }
}

/// How the dialog ended.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The helper said `SUCCESS` for this identity, written as polkit writes it: `unix-user:alice`.
    Authenticated { identity: String },
    /// The user cancelled.
    Declined,
}

/// What one [`PolkitDialog::advance`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Step {
    /// The card changed, so it must be drawn again.
    pub changed: bool,
    /// How the dialog ended, once it did.
    pub outcome: Option<Outcome>,
}

/// One polkit request on screen: its card and the helper behind it.
pub(crate) struct PolkitDialog {
    ui: PolkitUi,
    cookie: String,
    /// The login name behind each identity the card offers, in the card's order.
    users: Vec<String>,
    helper: HelperPath,
    session: Option<HelperSession>,
    failures: u32,
    /// Whether the helper sent an error message in the current attempt, so a failure that follows is not told twice.
    said: bool,
}

impl PolkitDialog {
    /// The dialog for `request`, with the helper for the identity the user is most likely to be: themselves, when
    /// polkit offered them.
    pub(crate) fn new(request: AuthRequest, helper: HelperPath) -> Self {
        let own = current_uid();
        let mut users: Vec<(bool, String)> = request
            .identities
            .iter()
            .filter_map(|identity| match identity {
                Identity::UnixUser { uid } => gnoblin_polkit_auth::name_of(*uid).map(|name| (Some(*uid) != own, name)),
                // A group or netgroup is expanded to its users by polkitd; a kind that is left has no helper.
                Identity::Other { .. } => None,
            })
            .collect();
        // Stable: the user's own account first, the others as polkit listed them.
        users.sort_by_key(|(other, _)| *other);
        let users: Vec<String> = users.into_iter().map(|(_, name)| name).collect();
        let ui = PolkitUi::new(request.message, request.requester, request.action_id, users.clone(), 0);
        let mut dialog = Self {
            ui,
            cookie: request.cookie,
            users,
            helper,
            session: None,
            failures: 0,
            said: false,
        };
        if dialog.users.is_empty() {
            dialog
                .ui
                .fail("None of the accounts polkit offered can authenticate on this machine.");
        } else {
            dialog.start(0);
        }
        dialog
    }

    /// The card, for the shell that draws it.
    pub(crate) fn ui_mut(&mut self) -> &mut PolkitUi {
        &mut self.ui
    }

    /// Whether PAM is waiting for an answer, for the service's tests.
    #[cfg(test)]
    pub(crate) fn is_asking(&self) -> bool {
        self.ui.is_asking()
    }

    /// Feeds one key to the card, as the modal's own keys do. Keys the card does not use are ignored.
    pub(crate) fn key(&mut self, key: ScreenKey) {
        match key {
            ScreenKey::Escape => self.ui.cancel(),
            ScreenKey::Enter => self.ui.submit(),
            ScreenKey::Backspace => self.ui.backspace(),
            ScreenKey::Space => self.ui.type_char(' '),
            ScreenKey::Text(character) => self.ui.type_char(character),
            ScreenKey::Tab | ScreenKey::Up | ScreenKey::Down => {}
        }
    }

    /// Acts on what the user did and on what the helper said since the last call, without waiting.
    pub(crate) fn advance(&mut self) -> Step {
        let mut step = Step::default();
        for event in self.ui.take_events() {
            step.changed = true;
            match event {
                PolkitEvent::Cancel => {
                    self.session = None;
                    step.outcome = Some(Outcome::Declined);
                    return step;
                }
                PolkitEvent::Reply(answer) => {
                    // The answer is wiped when it drops at the end of this arm, whether or not the helper took it.
                    let sent = self.session.as_mut().map(|session| session.respond(&answer));
                    if let Some(Err(error)) = sent {
                        self.session = None;
                        self.ui.fail(error.to_string());
                    }
                }
                PolkitEvent::Identity(index) => {
                    self.failures = 0;
                    self.start(index);
                }
            }
        }
        let events = self.session.as_mut().map(HelperSession::poll).unwrap_or_default();
        for event in events {
            step.changed = true;
            match event {
                HelperEvent::Prompt { text, echo } => self.ui.ask(text, echo),
                HelperEvent::Info(text) => self.ui.info(text),
                HelperEvent::Error(text) => {
                    self.said = true;
                    self.ui.error(text);
                }
                HelperEvent::Success => {
                    self.session = None;
                    let user = self.users.get(self.ui.selected()).cloned().unwrap_or_default();
                    step.outcome = Some(Outcome::Authenticated {
                        identity: format!("unix-user:{user}"),
                    });
                    return step;
                }
                HelperEvent::Failure => {
                    self.session = None;
                    self.failures += 1;
                    if self.failures >= MAX_FAILURES {
                        self.ui.fail("Too many failed attempts.");
                    } else {
                        if !self.said {
                            self.ui.error(TRY_AGAIN);
                        }
                        self.start(self.ui.selected());
                    }
                }
            }
        }
        step
    }

    /// Starts the helper for the identity at `index`, replacing the one that ran. A helper that cannot start is the
    /// card's error to show.
    fn start(&mut self, index: usize) {
        self.session = None;
        self.said = false;
        let Some(user) = self.users.get(index) else {
            self.ui.fail("That account cannot authenticate.");
            return;
        };
        let started = self
            .helper
            .locate()
            .and_then(|path| HelperSession::spawn(&path, user, &self.cookie));
        match started {
            Ok(session) => {
                self.session = Some(session);
                self.ui.wait();
            }
            Err(error) => self.ui.fail(error.to_string()),
        }
    }
}

impl std::fmt::Debug for PolkitDialog {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PolkitDialog")
            .field("ui", &self.ui)
            .field("failures", &self.failures)
            .finish_non_exhaustive()
    }
}

/// Fake helpers for the dialog's tests and the service's: scripts that speak the real helper's protocol
/// (design/js-research/polkit.md). They run no PAM and talk to no polkitd.
#[cfg(test)]
pub(crate) mod fake {
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::PathBuf;

    use gnoblin_polkit_auth::{AuthRequest, Identity, current_uid, name_of};

    use super::HelperPath;

    /// A directory that is removed when the test ends, holding one fake helper.
    pub(crate) struct Fake(pub(crate) PathBuf);

    impl Fake {
        /// A helper whose script is `body`, with `@DIR@` replaced by the directory it lives in.
        pub(crate) fn new(name: &str, body: &str) -> Self {
            let directory = std::env::temp_dir().join(format!("gnoblin-polkit-dialog-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&directory);
            std::fs::create_dir_all(&directory).expect("a scratch directory");
            let script = directory.join("polkit-agent-helper-1");
            let body = body.replace("@DIR@", &directory.display().to_string());
            std::fs::write(&script, format!("#!/bin/sh\n{body}\n")).expect("the script is written");
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("it is executable");
            Self(directory)
        }

        pub(crate) fn path(&self) -> HelperPath {
            HelperPath::fixed(self.0.join("polkit-agent-helper-1"))
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The uid and account name this process runs as, which `/etc/passwd` knows.
    pub(crate) fn me() -> (u32, String) {
        let uid = current_uid().expect("this process has a uid");
        (uid, name_of(uid).expect("and an account"))
    }

    /// The agent's payload for one authentication of `uid`, with the cookie `7-cookie`.
    pub(crate) fn request(uid: u32) -> AuthRequest {
        AuthRequest::new(
            "7-cookie".to_owned(),
            "org.freedesktop.policykit.exec".to_owned(),
            "Authentication is needed".to_owned(),
            String::new(),
            std::collections::BTreeMap::new(),
            vec![Identity::UnixUser { uid }],
        )
    }

    /// A helper that asks for a password and accepts `hunter2` for `user` with the cookie `7-cookie`.
    pub(crate) fn password_helper(user: &str) -> Fake {
        Fake::new(
            "password",
            &format!(
                r#"read cookie
printf 'PAM_PROMPT_ECHO_OFF Password: \n'
read password
if [ "$1" = {user} ] && [ "$cookie" = 7-cookie ] && [ "$password" = hunter2 ]; then
    printf 'SUCCESS\n'
    exit 0
fi
printf 'PAM_ERROR_MSG Authentication failure\n'
printf 'FAILURE\n'
exit 1"#
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use super::fake::{Fake, me, password_helper, request};
    use super::*;

    /// Advances until `done` holds or the dialog ends, a few seconds at most.
    fn run_until(dialog: &mut PolkitDialog, mut done: impl FnMut(&mut PolkitDialog) -> bool) -> Option<Outcome> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let step = dialog.advance();
            if step.outcome.is_some() {
                return step.outcome;
            }
            if done(dialog) {
                return None;
            }
            assert!(Instant::now() < deadline, "the dialog did not get there: {dialog:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A dialog whose helper started, tolerating the kernel's "text file busy" for a script just written: a thread
    /// that forks in the test process between the write and the close holds a copy of the descriptor for a moment.
    fn dialog(uid: u32, helper: &Fake) -> PolkitDialog {
        for _ in 0..50 {
            let dialog = PolkitDialog::new(request(uid), helper.path());
            if !dialog.ui.has_failed() {
                return dialog;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the helper never started");
    }

    fn type_text(dialog: &mut PolkitDialog, text: &str) {
        for character in text.chars() {
            dialog.key(ScreenKey::Text(character));
        }
        dialog.key(ScreenKey::Enter);
    }

    #[test]
    fn only_the_helpers_success_authenticates_and_names_the_identity() {
        let (uid, name) = me();
        let fake = password_helper(&name);
        let mut dialog = dialog(uid, &fake);
        run_until(&mut dialog, |dialog| dialog.ui.is_asking());
        type_text(&mut dialog, "hunter2");
        assert_eq!(
            run_until(&mut dialog, |_| false),
            Some(Outcome::Authenticated {
                identity: format!("unix-user:{name}")
            })
        );
    }

    #[test]
    fn a_wrong_password_shows_the_helpers_message_and_asks_again() {
        let (uid, name) = me();
        let fake = password_helper(&name);
        let mut dialog = dialog(uid, &fake);
        run_until(&mut dialog, |dialog| dialog.ui.is_asking());
        type_text(&mut dialog, "nope");
        // The failure starts a new helper, which asks again, and the right password then succeeds.
        run_until(&mut dialog, |dialog| dialog.failures == 1 && dialog.ui.is_asking());
        type_text(&mut dialog, "hunter2");
        assert!(matches!(
            run_until(&mut dialog, |_| false),
            Some(Outcome::Authenticated { .. })
        ));
    }

    #[test]
    fn too_many_failures_end_the_attempt() {
        let (uid, _) = me();
        let fake = Fake::new("always-fails", "read cookie\nprintf 'FAILURE\\n'\nexit 1");
        let mut dialog = dialog(uid, &fake);
        run_until(&mut dialog, |dialog| dialog.ui.has_failed());
        assert_eq!(dialog.failures, MAX_FAILURES);
        assert!(!dialog.ui.is_asking());
    }

    #[test]
    fn a_missing_helper_is_said_and_fails_closed() {
        let (uid, _) = me();
        let mut dialog = PolkitDialog::new(
            request(uid),
            HelperPath::fixed(PathBuf::from("/nonexistent/polkit-agent-helper-1")),
        );
        assert!(dialog.ui.has_failed(), "the card says it cannot authenticate");
        assert_eq!(dialog.advance().outcome, None, "nothing authenticated");
        dialog.key(ScreenKey::Escape);
        assert_eq!(
            dialog.advance().outcome,
            Some(Outcome::Declined),
            "closing it ends the request unauthenticated"
        );
    }

    #[test]
    fn a_request_with_no_usable_account_fails_closed() {
        let request = AuthRequest::new(
            "c".to_owned(),
            "action".to_owned(),
            "message".to_owned(),
            String::new(),
            std::collections::BTreeMap::new(),
            vec![Identity::Other {
                kind: "unix-netgroup".to_owned(),
            }],
        );
        let dialog = PolkitDialog::new(request, HelperPath::fixed(PathBuf::from("/bin/true")));
        assert!(dialog.ui.has_failed());
    }

    #[test]
    fn cancel_kills_the_helper() {
        let (uid, _) = me();
        let fake = Fake::new(
            "hangs",
            "read cookie\necho $$ > @DIR@/pid\nprintf 'PAM_PROMPT_ECHO_OFF Password: \\n'\nexec sleep 60",
        );
        let mut dialog = dialog(uid, &fake);
        run_until(&mut dialog, |dialog| dialog.ui.is_asking());
        let pid: u32 = std::fs::read_to_string(fake.0.join("pid"))
            .expect("the helper wrote its pid")
            .trim()
            .parse()
            .expect("a pid");
        assert!(Path::new(&format!("/proc/{pid}")).exists());
        dialog.key(ScreenKey::Escape);
        assert_eq!(dialog.advance().outcome, Some(Outcome::Declined));
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "the helper is killed and reaped when the user cancels"
        );
    }

    #[test]
    fn nothing_the_dialog_formats_holds_the_password() {
        let (uid, name) = me();
        let fake = password_helper(&name);
        let mut dialog = dialog(uid, &fake);
        run_until(&mut dialog, |dialog| dialog.ui.is_asking());
        for character in "hunter2".chars() {
            dialog.key(ScreenKey::Text(character));
        }
        let shown = format!("{dialog:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
        dialog.key(ScreenKey::Enter);
        let step = run_until(&mut dialog, |_| false);
        let shown = format!("{dialog:?} {step:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
    }
}
