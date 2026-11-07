//! The unlock path. The store key is the login password, as `design/keyring.md` "Key source"
//! decides. The password reaches this module from the PAM hand-off, or from one prompt when the
//! login had no password. The SQLCipher key derivation (PBKDF2 with an explicit `kdf_iter`) runs
//! in the store crate, so this module only chooses between create, open and re-key.
//!
//! Nothing here logs or keeps the password. A `Passphrase` wipes its text on drop, and the byte
//! buffers the caller passes in are the caller's to wipe.

use std::path::PathBuf;

use gnoblin_keyring_store::{Keyring, KeyringError, Passphrase};
use zeroize::Zeroizing;

use crate::log;
use crate::objects::{self, State};

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

/// Why a prompted unlock ended.
#[derive(Debug, PartialEq, Eq)]
pub enum PromptOutcome {
    /// The store is open.
    Unlocked,
    /// The user dismissed the prompt. The store stays locked.
    Cancelled,
    /// The password the user typed does not open the store.
    WrongKey,
    /// The store could not be opened for another reason.
    Failed(String),
    /// The prompt was already shown in this login session. It is never shown twice.
    AlreadyAsked,
}

/// Asks the user for the store password. `None` means the user cancelled.
pub trait PasswordPrompt {
    fn ask(&mut self) -> Option<Zeroizing<Vec<u8>>>;
}

/// Opens or creates the keyring file for the shared `state`, and runs `on_unlock` once the store
/// is open so the bus objects can be exported.
pub struct UnlockKeySource {
    path: PathBuf,
    state: State,
    /// Set once the no-password prompt has been shown, so a login session is never asked twice.
    asked: bool,
    on_unlock: Box<dyn FnMut() + Send>,
}

impl UnlockKeySource {
    pub fn new(path: PathBuf, state: State, on_unlock: Box<dyn FnMut() + Send>) -> UnlockKeySource {
        UnlockKeySource {
            path,
            state,
            asked: false,
            on_unlock,
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

    /// Unlock by asking the user, for a login that had no password (fingerprint or autologin).
    /// The prompt is shown at most once per login session, whatever the answer.
    pub fn unlock_by_prompt(&mut self, prompt: &mut dyn PasswordPrompt) -> PromptOutcome {
        if objects::is_unlocked(&self.state) {
            return PromptOutcome::Unlocked;
        }
        if self.asked {
            return PromptOutcome::AlreadyAsked;
        }
        self.asked = true;
        let Some(password) = prompt.ask() else {
            log::info!("the unlock prompt was cancelled");
            return PromptOutcome::Cancelled;
        };
        match self.unlock(&password) {
            Ok(()) => PromptOutcome::Unlocked,
            Err(UnlockError::WrongKey) => PromptOutcome::WrongKey,
            Err(UnlockError::Refused(reason) | UnlockError::Failed(reason)) => PromptOutcome::Failed(reason),
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

    /// A key source over a fresh locked state, with a counter of how often the store opened.
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
        );
        (source, state, unlocks)
    }

    /// Drop the open store, as a restart of the service would.
    fn lock_store(state: &State) {
        objects::install_keyring(state, None);
    }

    struct Scripted {
        answer: Option<Vec<u8>>,
        asks: usize,
    }

    impl PasswordPrompt for Scripted {
        fn ask(&mut self) -> Option<Zeroizing<Vec<u8>>> {
            self.asks += 1;
            self.answer.clone().map(Zeroizing::new)
        }
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
    fn no_password_prompts_once_per_session() {
        let dir = scratch("prompt-once");
        let (mut source, state, _) = source(&dir);
        source.unlock(b"pw").expect("create");
        lock_store(&state);
        let mut prompt = Scripted {
            answer: Some(b"wrong".to_vec()),
            asks: 0,
        };
        assert_eq!(source.unlock_by_prompt(&mut prompt), PromptOutcome::WrongKey);
        assert_eq!(source.unlock_by_prompt(&mut prompt), PromptOutcome::AlreadyAsked);
        assert_eq!(prompt.asks, 1, "a wrong answer is not asked again in the same session");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_prompt_with_the_right_password_unlocks_and_is_not_shown_again() {
        let dir = scratch("prompt-right");
        let (mut source, state, _) = source(&dir);
        source.unlock(b"pw").expect("create");
        lock_store(&state);
        let mut prompt = Scripted {
            answer: Some(b"pw".to_vec()),
            asks: 0,
        };
        assert_eq!(source.unlock_by_prompt(&mut prompt), PromptOutcome::Unlocked);
        lock_store(&state);
        assert_eq!(source.unlock_by_prompt(&mut prompt), PromptOutcome::AlreadyAsked);
        assert_eq!(prompt.asks, 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn cancelling_the_prompt_leaves_the_store_locked_and_is_not_repeated() {
        let dir = scratch("prompt-cancel");
        let (mut source, state, _) = source(&dir);
        source.unlock(b"pw").expect("create");
        lock_store(&state);
        let mut prompt = Scripted { answer: None, asks: 0 };
        assert_eq!(source.unlock_by_prompt(&mut prompt), PromptOutcome::Cancelled);
        assert_eq!(source.unlock_by_prompt(&mut prompt), PromptOutcome::AlreadyAsked);
        assert_eq!(prompt.asks, 1);
        assert!(!objects::is_unlocked(&state));
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
