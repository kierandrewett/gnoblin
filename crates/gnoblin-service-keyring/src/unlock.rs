//! The unlock path. The store key is the login password, as `design/keyring.md` "Key source"
//! decides. The password reaches this module from the PAM hand-off, or from one request answered
//! by the user when the login had no password. The SQLCipher key derivation (PBKDF2 with an
//! explicit `kdf_iter`) runs in the store crate, so this module only chooses between create, open
//! and re-key.
//!
//! The keyring never draws UI. A password prompt is an offer of a `prompt.secret` request to the
//! compositor, and the answer comes back as a request outcome. Nothing here logs or keeps the
//! password: a `Passphrase` wipes its text on drop, and the byte buffers the caller passes in are
//! the caller's to wipe.

use std::path::PathBuf;

use gnoblin_keyring_store::{Keyring, KeyringError, Passphrase};
use zeroize::Zeroizing;

use crate::log;
use crate::objects::{self, State};

/// The title and message of the password request. They name the keyring and never the password.
pub const PROMPT_TITLE: &str = "Unlock the keyring";
pub const PROMPT_MESSAGE: &str = "Enter your login password to unlock the keyring.";

/// Why the store was not unlocked or re-keyed.
#[derive(Debug, PartialEq, Eq)]
pub enum UnlockError {
    /// The password does not open the store. Nothing was changed.
    WrongKey,
    /// The request itself is unacceptable: an empty or non-UTF-8 password, or no store to re-key.
    Refused(String),
    /// The store could not be created, opened or re-keyed.
    Failed(String),
}

/// Names one password request the keyring offered. [`PasswordOffer::offer`] returns it, and the
/// answer comes back keyed by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromptId(u64);

impl PromptId {
    pub const fn new(raw: u64) -> PromptId {
        PromptId(raw)
    }
}

/// How a password request ended, as the keyring sees it. The answer bytes are wiped on drop.
pub enum PromptOutcome {
    /// The prompt answered with the password.
    Answered(Zeroizing<Vec<u8>>),
    /// The prompt ended without an answer.
    Ended,
    /// The prompt was declined, and said why.
    Refused(PromptRefusal),
    /// The deadline passed before an answer arrived.
    TimedOut,
}

impl PromptOutcome {
    /// An answer, taking ownership of its bytes so they are wiped when the outcome drops.
    pub fn answered(bytes: Vec<u8>) -> PromptOutcome {
        PromptOutcome::Answered(Zeroizing::new(bytes))
    }
}

/// Why a prompt was declined. The reason is not secret, so it may be logged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptRefusal {
    /// The provider cannot answer while its store is locked.
    Locked,
    /// The provider was asked and refused.
    Denied,
    /// The provider cannot answer at all right now.
    Unavailable,
}

/// Offers the password request to whoever answers prompts. The answer returns through
/// [`UnlockKeySource::answer_password`], keyed by the id this returns.
pub trait PasswordOffer: Send {
    /// Offer one request with `title` and `message`. The error says why nothing was offered.
    fn offer(&mut self, title: &str, message: &str) -> Result<PromptId, String>;
}

/// The password request this source offered and is still waiting on, with every bus object that
/// joined it while it was pending.
struct PendingPassword {
    request: PromptId,
    /// The bus objects waiting on the `Completed` signal, one per caller that joined the request.
    paths: Vec<String>,
}

/// A prompt that has finished, and what its bus objects must now report. `dismissed` is true when
/// the store is still locked, so the caller is told it was cancelled.
#[derive(Debug, PartialEq, Eq)]
pub struct PromptEnd {
    /// Every bus object that joined the request. Each one is told the same outcome.
    pub paths: Vec<String>,
    pub dismissed: bool,
}

/// Opens or creates the keyring file for the shared `state`, and runs `on_unlock` once the store
/// is open so the bus objects can be exported.
pub struct UnlockKeySource {
    path: PathBuf,
    state: State,
    pending: Option<PendingPassword>,
    on_unlock: Box<dyn FnMut() + Send>,
    offers: Box<dyn PasswordOffer>,
}

impl UnlockKeySource {
    pub fn new(
        path: PathBuf,
        state: State,
        on_unlock: Box<dyn FnMut() + Send>,
        offers: Box<dyn PasswordOffer>,
    ) -> UnlockKeySource {
        UnlockKeySource {
            path,
            state,
            pending: None,
            on_unlock,
            offers,
        }
    }

    /// Unlock with the login password. Creates the file on first run, and opens it afterwards.
    /// A store that is already open is left as it is.
    pub fn unlock(&mut self, password: &[u8]) -> Result<(), UnlockError> {
        if objects::is_unlocked(&self.state) {
            return Ok(());
        }
        let passphrase = passphrase_from(password)?;
        let keyring = match Keyring::open(&self.path, &passphrase) {
            Ok(keyring) => keyring,
            Err(KeyringError::Missing(_)) => {
                log::info!("no keyring file yet, creating one");
                Keyring::create(&self.path, &passphrase).map_err(failure)?
            }
            Err(error) => return Err(failure(error)),
        };
        objects::install_keyring(&self.state, Some(keyring));
        log::info!("the keyring is unlocked");
        self.after_install()
    }

    /// Re-key the store from `old` to `new`, after the login password changed. `old` must open
    /// the store. The state's connection is replaced under the same lock, so no other call sees
    /// a half re-keyed file.
    pub fn change_password(&mut self, old: &[u8], new: &[u8]) -> Result<(), UnlockError> {
        let old = passphrase_from(old)?;
        let new = passphrase_from(new)?;
        let keyring = match Keyring::open(&self.path, &old) {
            Ok(keyring) => keyring,
            Err(KeyringError::Missing(_)) => {
                return Err(UnlockError::Refused("there is no keyring to re-key".to_owned()));
            }
            Err(error) => return Err(failure(error)),
        };
        // The old key is valid until the re-key commits, so a failure leaves the store usable.
        objects::rekey_and_install(&self.state, keyring, &new).map_err(failure)?;
        log::info!("the keyring was re-keyed to the new password");
        self.after_install()
    }

    /// Ask for the login password through a request, for a login that had none (fingerprint or
    /// autologin). While a request is pending, a further caller joins it, so the user sees one
    /// prompt rather than one per caller. `path` is the bus object the caller waits on; it is handed
    /// back by [`UnlockKeySource::answer_password`]. Once a request has ended, the next one asks
    /// again: a typo or a dismissed prompt does not lock the user out for the session.
    pub fn offer_password(&mut self, path: &str) -> Result<(), String> {
        if objects::is_unlocked(&self.state) {
            return Err("the keyring is already open".to_owned());
        }
        if let Some(pending) = self.pending.as_mut() {
            pending.paths.push(path.to_owned());
            return Ok(());
        }
        let request = self.offers.offer(PROMPT_TITLE, PROMPT_MESSAGE)?;
        self.pending = Some(PendingPassword {
            request,
            paths: vec![path.to_owned()],
        });
        log::info!("asked for the keyring password");
        Ok(())
    }

    /// The answer to the password request. An answer for any other request is refused and changes
    /// nothing. An answer that opens the store finishes the prompt undismissed; any other ending
    /// finishes it dismissed, and the store stays locked.
    pub fn answer_password(&mut self, request: PromptId, outcome: PromptOutcome) -> Option<PromptEnd> {
        let pending = self.pending.take_if(|pending| pending.request == request)?;
        match outcome {
            PromptOutcome::Answered(password) => {
                if let Err(error) = self.unlock(&password) {
                    log::info!(reason = ?error, "the password from the prompt did not unlock the keyring");
                }
            }
            PromptOutcome::Ended => log::info!("the password prompt ended without an answer"),
            PromptOutcome::Refused(reason) => log::info!(reason = ?reason, "the password prompt was declined"),
            PromptOutcome::TimedOut => log::info!("the password prompt timed out"),
        }
        Some(PromptEnd {
            paths: pending.paths,
            dismissed: !objects::is_unlocked(&self.state),
        })
    }

    /// The prompt object at `path` was dismissed by its caller. It leaves the shared prompt, which
    /// stays open for the callers still waiting on it. When none is left, a later answer for it is
    /// refused as unknown.
    pub fn dismiss_password(&mut self, path: &str) {
        if let Some(pending) = self.pending.as_mut() {
            pending.paths.retain(|waiting| waiting != path);
            if pending.paths.is_empty() {
                self.pending = None;
            }
        }
    }

    /// Create the default collection if needed, then tell the bus side the store is open. The
    /// store stays open even when the collection step fails, so the error is returned after the
    /// objects are exported.
    fn after_install(&mut self) -> Result<(), UnlockError> {
        let collection = objects::ensure_default_collection(&self.state).map_err(failure);
        (self.on_unlock)();
        collection
    }
}

/// Turn the bytes from the hand-off into a passphrase. Non-UTF-8 bytes cannot be a password the
/// user typed, so they are refused rather than lossily converted.
fn passphrase_from(bytes: &[u8]) -> Result<Passphrase, UnlockError> {
    let text = std::str::from_utf8(bytes).map_err(|_| UnlockError::Refused("the password is not UTF-8".to_owned()))?;
    Passphrase::new(text.to_owned()).map_err(failure)
}

fn failure(error: KeyringError) -> UnlockError {
    match error {
        KeyringError::WrongKey => UnlockError::WrongKey,
        KeyringError::EmptyPassphrase => UnlockError::Refused("the password is empty".to_owned()),
        other => UnlockError::Failed(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A scratch directory unique to this process and test.
    fn scratch(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("gnoblin-unlock-test-{}-{label}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch directory");
        dir
    }

    /// A key source over a fresh locked state, with a counter of how often the store opened. Its
    /// password offer has no provider, so an offer fails at once.
    fn source(dir: &std::path::Path) -> (UnlockKeySource, State, Arc<AtomicUsize>) {
        let state = objects::new_state(None);
        let unlocks = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&unlocks);
        let source = UnlockKeySource::new(
            dir.join("login.keyring"),
            Arc::clone(&state),
            Box::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
            Box::new(FakeOffer::none()),
        );
        (source, state, unlocks)
    }

    /// A key source whose password offer answers `result`, with a counter of the offers made.
    fn prompt_source(
        dir: &std::path::Path,
        result: Result<PromptId, String>,
    ) -> (UnlockKeySource, State, Arc<AtomicUsize>) {
        let state = objects::new_state(None);
        let offers = Arc::new(AtomicUsize::new(0));
        let offer = FakeOffer {
            result,
            offers: Arc::clone(&offers),
        };
        let source = UnlockKeySource::new(
            dir.join("login.keyring"),
            Arc::clone(&state),
            Box::new(|| {}),
            Box::new(offer),
        );
        (source, state, offers)
    }

    /// A password offer that counts its calls and returns a fixed result.
    struct FakeOffer {
        result: Result<PromptId, String>,
        offers: Arc<AtomicUsize>,
    }

    impl FakeOffer {
        fn none() -> FakeOffer {
            FakeOffer {
                result: Err("no provider".to_owned()),
                offers: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    impl PasswordOffer for FakeOffer {
        fn offer(&mut self, _title: &str, _message: &str) -> Result<PromptId, String> {
            self.offers.fetch_add(1, Ordering::SeqCst);
            self.result.clone()
        }
    }

    /// Drop the open store, as a restart of the service would.
    fn lock_store(state: &State) {
        objects::install_keyring(state, None);
    }

    #[test]
    fn first_unlock_creates_the_file_and_later_unlocks_open_it() {
        let dir = scratch("create-open");
        let (mut source, state, unlocks) = source(&dir);
        source.unlock(b"correct horse").expect("first run creates the keyring");
        assert!(objects::is_unlocked(&state));
        assert!(dir.join("login.keyring").exists());
        lock_store(&state);
        source.unlock(b"correct horse").expect("the same password opens it");
        assert_eq!(unlocks.load(Ordering::SeqCst), 2);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn wrong_password_is_refused_and_the_store_stays_locked() {
        let dir = scratch("wrong");
        let (mut source, state, unlocks) = source(&dir);
        source.unlock(b"right").expect("create");
        lock_store(&state);
        assert_eq!(source.unlock(b"wrong"), Err(UnlockError::WrongKey));
        assert!(!objects::is_unlocked(&state));
        assert_eq!(
            unlocks.load(Ordering::SeqCst),
            1,
            "on_unlock ran only for the good open"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn empty_and_non_utf8_passwords_are_refused_without_touching_the_file() {
        let dir = scratch("refused");
        let (mut source, state, _) = source(&dir);
        assert!(matches!(source.unlock(b""), Err(UnlockError::Refused(_))));
        assert!(matches!(source.unlock(&[0xff, 0xfe]), Err(UnlockError::Refused(_))));
        assert!(!dir.join("login.keyring").exists());
        assert!(!objects::is_unlocked(&state));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_wrong_password_from_the_prompt_stays_locked_and_is_dismissed() {
        let dir = scratch("prompt-wrong");
        let (mut source, state, offers) = prompt_source(&dir, Ok(PromptId::new(7)));
        source.unlock(b"right").expect("create");
        lock_store(&state);
        source.offer_password("/prompt/1").expect("the request is offered");
        let end = source.answer_password(PromptId::new(7), PromptOutcome::answered(b"wrong".to_vec()));
        assert_eq!(
            end,
            Some(PromptEnd {
                paths: vec!["/prompt/1".to_owned()],
                dismissed: true,
            })
        );
        assert!(!objects::is_unlocked(&state));
        assert_eq!(offers.load(Ordering::SeqCst), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    /// A password that is typed wrong, through every path that takes a password, must never reach
    /// a log event. The assertions do not print the captured text, because it would hold the
    /// password on failure.
    #[test]
    fn the_password_never_reaches_the_log() {
        const SECRET: &str = "pw-never-logged-9f3c";
        let dir = scratch("no-log");
        let (mut source, state, _) = prompt_source(&dir, Ok(PromptId::new(7)));
        source.unlock(SECRET.as_bytes()).expect("create");
        lock_store(&state);
        source.offer_password("/prompt/1").expect("the request is offered");
        let wrong = format!("{SECRET}-wrong");
        let events = gnoblin_log::event::capture::logged(|| {
            source.answer_password(PromptId::new(7), PromptOutcome::answered(wrong.as_bytes().to_vec()));
            let _ = source.unlock(wrong.as_bytes());
            let _ = source.change_password(wrong.as_bytes(), SECRET.as_bytes());
        });
        let text: String = events
            .iter()
            .map(|event| format!("{} {}\n", event.message, event.fields_line()))
            .collect();
        assert!(text.contains("did not unlock"), "the refused prompt answer is logged");
        assert!(!text.contains(SECRET), "the password reached a log event");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn with_no_provider_the_offer_fails_at_once_and_nothing_is_pending() {
        let dir = scratch("prompt-no-provider");
        let (mut source, state, offers) = prompt_source(&dir, Err("no provider for prompt.secret".to_owned()));
        source.unlock(b"right").expect("create");
        lock_store(&state);
        assert_eq!(
            source.offer_password("/prompt/1"),
            Err("no provider for prompt.secret".to_owned())
        );
        assert_eq!(
            source.answer_password(PromptId::new(7), PromptOutcome::answered(b"right".to_vec())),
            None
        );
        assert!(!objects::is_unlocked(&state));
        assert_eq!(offers.load(Ordering::SeqCst), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_prompt_that_ends_without_an_answer_is_dismissed_and_stays_locked() {
        let replies = [
            PromptOutcome::Ended,
            PromptOutcome::Refused(PromptRefusal::Denied),
            PromptOutcome::TimedOut,
        ];
        for (index, reply) in replies.into_iter().enumerate() {
            let dir = scratch(&format!("prompt-ended-{index}"));
            let (mut source, state, _) = prompt_source(&dir, Ok(PromptId::new(7)));
            source.unlock(b"right").expect("create");
            lock_store(&state);
            source.offer_password("/prompt/1").expect("the request is offered");
            let end = source.answer_password(PromptId::new(7), reply);
            assert_eq!(
                end,
                Some(PromptEnd {
                    paths: vec!["/prompt/1".to_owned()],
                    dismissed: true,
                }),
                "reply {index} dismisses the prompt"
            );
            assert!(!objects::is_unlocked(&state));
            std::fs::remove_dir_all(dir).ok();
        }
    }

    #[test]
    fn the_right_password_from_the_prompt_unlocks_and_is_not_dismissed() {
        let dir = scratch("prompt-right");
        let (mut source, state, _) = prompt_source(&dir, Ok(PromptId::new(7)));
        source.unlock(b"right").expect("create");
        lock_store(&state);
        source.offer_password("/prompt/1").expect("the request is offered");
        let end = source.answer_password(PromptId::new(7), PromptOutcome::answered(b"right".to_vec()));
        assert_eq!(
            end,
            Some(PromptEnd {
                paths: vec!["/prompt/1".to_owned()],
                dismissed: false,
            })
        );
        assert!(objects::is_unlocked(&state));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn an_answer_for_another_request_is_refused_and_the_prompt_stays_pending() {
        let dir = scratch("prompt-other-id");
        let (mut source, state, _) = prompt_source(&dir, Ok(PromptId::new(7)));
        source.unlock(b"right").expect("create");
        lock_store(&state);
        source.offer_password("/prompt/1").expect("the request is offered");
        assert_eq!(
            source.answer_password(PromptId::new(8), PromptOutcome::answered(b"right".to_vec())),
            None
        );
        assert!(!objects::is_unlocked(&state), "a stranger's answer unlocks nothing");
        let end = source.answer_password(PromptId::new(7), PromptOutcome::answered(b"right".to_vec()));
        assert_eq!(
            end,
            Some(PromptEnd {
                paths: vec!["/prompt/1".to_owned()],
                dismissed: false,
            })
        );
        assert!(objects::is_unlocked(&state));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_dismissed_prompt_refuses_a_late_answer() {
        let dir = scratch("prompt-dismissed");
        let (mut source, state, _) = prompt_source(&dir, Ok(PromptId::new(7)));
        source.unlock(b"right").expect("create");
        lock_store(&state);
        source.offer_password("/prompt/1").expect("the request is offered");
        source.dismiss_password("/prompt/1");
        assert_eq!(
            source.answer_password(PromptId::new(7), PromptOutcome::answered(b"right".to_vec())),
            None,
            "the caller gave up, so the late answer is ignored"
        );
        assert!(!objects::is_unlocked(&state));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_request_made_while_one_is_pending_joins_it() {
        let dir = scratch("prompt-join");
        let (mut source, state, offers) = prompt_source(&dir, Ok(PromptId::new(7)));
        source.unlock(b"right").expect("create");
        lock_store(&state);
        source.offer_password("/prompt/1").expect("the first offer");
        source.offer_password("/prompt/2").expect("the second caller joins");
        assert_eq!(
            offers.load(Ordering::SeqCst),
            1,
            "one request is asked, however many callers wait"
        );
        let end = source.answer_password(PromptId::new(7), PromptOutcome::Ended);
        assert_eq!(
            end,
            Some(PromptEnd {
                paths: vec!["/prompt/1".to_owned(), "/prompt/2".to_owned()],
                dismissed: true,
            })
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn dismissing_one_joined_caller_leaves_the_others_waiting() {
        let dir = scratch("prompt-leave");
        let (mut source, state, _) = prompt_source(&dir, Ok(PromptId::new(7)));
        source.unlock(b"right").expect("create");
        lock_store(&state);
        source.offer_password("/prompt/1").expect("the first offer");
        source.offer_password("/prompt/2").expect("the second caller joins");
        source.dismiss_password("/prompt/1");
        let end = source.answer_password(PromptId::new(7), PromptOutcome::Ended);
        assert_eq!(
            end,
            Some(PromptEnd {
                paths: vec!["/prompt/2".to_owned()],
                dismissed: true,
            })
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_wrong_password_is_asked_for_again_by_the_next_request() {
        let dir = scratch("prompt-retry-wrong");
        let (mut source, state, offers) = prompt_source(&dir, Ok(PromptId::new(7)));
        source.unlock(b"right").expect("create");
        lock_store(&state);
        source.offer_password("/prompt/1").expect("the first offer");
        source.answer_password(PromptId::new(7), PromptOutcome::answered(b"wrong".to_vec()));
        source.offer_password("/prompt/2").expect("the next request asks again");
        assert_eq!(offers.load(Ordering::SeqCst), 2);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_dismissed_prompt_is_asked_for_again_by_the_next_request() {
        let dir = scratch("prompt-retry-dismissed");
        let (mut source, state, offers) = prompt_source(&dir, Ok(PromptId::new(7)));
        source.unlock(b"right").expect("create");
        lock_store(&state);
        source.offer_password("/prompt/1").expect("the first offer");
        source.dismiss_password("/prompt/1");
        source.offer_password("/prompt/2").expect("the next request asks again");
        assert_eq!(offers.load(Ordering::SeqCst), 2);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn an_open_store_is_never_offered_a_password() {
        let dir = scratch("prompt-open");
        let (mut source, _, offers) = prompt_source(&dir, Ok(PromptId::new(7)));
        source.unlock(b"right").expect("create");
        assert!(source.offer_password("/prompt/1").is_err());
        assert_eq!(offers.load(Ordering::SeqCst), 0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_password_offer_that_fails_leaves_the_prompt_free_to_be_offered_again() {
        let dir = scratch("prompt-retry");
        let (mut source, state, offers) = prompt_source(&dir, Err("no provider".to_owned()));
        source.unlock(b"right").expect("create");
        lock_store(&state);
        assert!(source.offer_password("/prompt/1").is_err());
        assert!(source.offer_password("/prompt/2").is_err());
        assert_eq!(
            offers.load(Ordering::SeqCst),
            2,
            "a failed offer does not count as the one ask"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn password_change_re_keys_the_file() {
        let dir = scratch("change");
        let (mut source, state, _) = source(&dir);
        source.unlock(b"old pw").expect("create");
        source.change_password(b"old pw", b"new pw").expect("re-key");
        assert!(objects::is_unlocked(&state), "the open store now uses the new key");
        lock_store(&state);
        assert_eq!(source.unlock(b"old pw"), Err(UnlockError::WrongKey));
        source
            .unlock(b"new pw")
            .expect("the new password opens the re-keyed file");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn password_change_with_a_wrong_old_password_changes_nothing() {
        let dir = scratch("change-wrong");
        let (mut source, state, _) = source(&dir);
        source.unlock(b"pw").expect("create");
        lock_store(&state);
        assert_eq!(source.change_password(b"not it", b"other"), Err(UnlockError::WrongKey));
        source.unlock(b"pw").expect("the old key still opens the file");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn password_change_without_a_store_is_refused() {
        let dir = scratch("change-missing");
        let (mut source, _, _) = source(&dir);
        assert!(matches!(
            source.change_password(b"a", b"b"),
            Err(UnlockError::Refused(_))
        ));
        std::fs::remove_dir_all(dir).ok();
    }
}
