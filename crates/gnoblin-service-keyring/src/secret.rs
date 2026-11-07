//! The Secret portal's secret: one master in the store, one secret per app derived from it.
//!
//! `design/keyring.md`, "Secret portal". The master is 64 random bytes kept in the default
//! collection under the attribute `gnoblin-purpose=portal-secret`, created on first use. Each app
//! gets 32 bytes from HKDF-SHA256 over that master, with the app id as `info`, so two app ids
//! never share a secret and the same app id always gets the same one.
//!
//! Consent is asked once per app per session. The answer is held in [`Grants`], which lives in
//! memory and is never written to the store or to disk. A refusal grants nothing, so the next
//! request asks again. When no consent provider can be reached, the request is refused with no
//! secret: this module never falls back to answering on the user's behalf.
//!
//! # Failure modes first
//!
//! 1. **Two apps share a secret.** `distinct_apps_derive_distinct_secrets` pins it.
//! 2. **The same app gets a different secret on the next request.** `derivation_is_deterministic`.
//! 3. **A refused prompt still returns a secret.** `a_refused_prompt_returns_no_secret`.
//! 4. **A refusal is remembered.** `a_refusal_is_not_remembered`.
//! 5. **A grant is asked again within the session.** `an_allowed_app_is_not_asked_again`.
//! 6. **No consent provider still returns a secret.** `no_consent_provider_fails_closed`.
//! 7. **An empty app id derives a shared secret.** `an_empty_app_id_is_refused_before_asking`.
//! 8. **The master is replaced on a second run.** `the_master_is_created_once_and_read_back`.

use std::collections::BTreeSet;

use gnoblin_keyring_store::{Keyring, KeyringError};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::objects::{DEFAULT_ALIAS, State, ensure_default_collection, with_open_keyring};

/// The attribute that marks the master secret in the store.
pub const MASTER_ATTRIBUTE: (&str, &str) = ("gnoblin-purpose", "portal-secret");
/// The label the master item carries.
const MASTER_LABEL: &str = "Gnoblin portal secret";
/// The master's length in bytes.
const MASTER_LEN: usize = 64;
/// The per-app secret's length in bytes.
pub const APP_SECRET_LEN: usize = 32;
/// The HKDF salt. It names the derivation, so a future change of scheme takes a new salt.
const SALT: &[u8] = b"gnoblin portal secret v1";
/// Prefixed to the app id in `info`, so the input is unambiguous even for odd app ids.
const INFO_PREFIX: &[u8] = b"app:";

/// One app's secret. It is wiped when dropped.
pub type AppSecret = Zeroizing<[u8; APP_SECRET_LEN]>;

/// Why the master secret could not be read or created.
#[derive(Debug)]
pub enum MasterError {
    /// The store is locked, so there is nothing to read.
    Locked,
    /// The store has no default collection to hold the master.
    NoDefault,
    /// The store refused the read or the write.
    Store(KeyringError),
    /// The system random source failed.
    Random,
}

impl From<KeyringError> for MasterError {
    fn from(error: KeyringError) -> Self {
        MasterError::Store(error)
    }
}

/// The master secret held in the default collection, creating it on first use. The store must
/// already have a default collection; `master` ensures one before calling this.
pub fn master_in(keyring: &mut Keyring) -> Result<Zeroizing<Vec<u8>>, MasterError> {
    let (name, value) = MASTER_ATTRIBUTE;
    if let Some(id) = keyring.search_items(&[(name, value)])?.first().copied()
        && let Some(item) = keyring.item(id)?
    {
        return Ok(item.secret);
    }
    let collection = keyring.alias(DEFAULT_ALIAS)?.ok_or(MasterError::NoDefault)?;
    let mut bytes = Zeroizing::new(vec![0u8; MASTER_LEN]);
    getrandom::fill(&mut bytes).map_err(|_| MasterError::Random)?;
    keyring.put_item(collection, MASTER_LABEL, &bytes, &[MASTER_ATTRIBUTE])?;
    Ok(bytes)
}

/// The master secret of the open store behind `state`, creating it on first use.
pub fn master(state: &State) -> Result<Zeroizing<Vec<u8>>, MasterError> {
    ensure_default_collection(state)?;
    with_open_keyring(state, master_in).ok_or(MasterError::Locked)?
}

/// The secret for `app_id`: HKDF-SHA256 of `master`, with the app id as `info`.
#[must_use]
pub fn derive(master: &[u8], app_id: &str) -> AppSecret {
    let mut out = Zeroizing::new([0u8; APP_SECRET_LEN]);
    let hkdf = Hkdf::<Sha256>::new(Some(SALT), master);
    let mut info = Vec::with_capacity(INFO_PREFIX.len() + app_id.len());
    info.extend_from_slice(INFO_PREFIX);
    info.extend_from_slice(app_id.as_bytes());
    // 32 bytes is far below HKDF-SHA256's output limit, so expansion cannot fail.
    hkdf.expand(&info, &mut out[..])
        .expect("32 bytes is within HKDF-SHA256's output limit");
    out
}

/// What the user answered when asked whether an app may read its secret.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Consent {
    /// The app may read its secret.
    Allowed,
    /// The app may not, this time.
    Refused,
}

/// No consent provider could be asked. The answer is refusal, never a default.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NoConsent;

/// Asks the user whether `app_id` may read its secret. The services role implements this over
/// the `portal.ui` slot; a missing provider is [`NoConsent`].
pub trait ConsentPrompt {
    /// Ask once for `app_id`.
    fn ask(&mut self, app_id: &str) -> Result<Consent, NoConsent>;
}

/// The apps the user has allowed in this session. It is memory only: a restart forgets it, and
/// nothing about it is written to the store.
#[derive(Debug, Default)]
pub struct Grants {
    allowed: BTreeSet<String>,
}

impl Grants {
    /// Whether `app_id` was allowed earlier in this session.
    #[must_use]
    pub fn is_granted(&self, app_id: &str) -> bool {
        self.allowed.contains(app_id)
    }
}

/// Why a request got no secret.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// The request named no app, so there is no secret to give.
    NoApp,
    /// The user refused this app.
    Denied,
    /// No consent provider could be asked, so the request failed closed.
    NoPrompt,
}

impl Refusal {
    /// The portal response code for this refusal: 1 is "cancelled by the user", 2 is "other".
    #[must_use]
    pub const fn response(self) -> u32 {
        match self {
            Refusal::Denied => 1,
            Refusal::NoApp | Refusal::NoPrompt => 2,
        }
    }
}

/// The secret for `app_id`, asking for consent first unless this session already allowed it.
///
/// A refusal records nothing, so the next request asks again. The secret is derived only after an
/// allow, so a refused request never holds one.
pub fn retrieve(
    master: &[u8],
    grants: &mut Grants,
    prompt: &mut dyn ConsentPrompt,
    app_id: &str,
) -> Result<AppSecret, Refusal> {
    if app_id.is_empty() {
        return Err(Refusal::NoApp);
    }
    if !grants.is_granted(app_id) {
        match prompt.ask(app_id) {
            Ok(Consent::Allowed) => {
                grants.allowed.insert(app_id.to_owned());
            }
            Ok(Consent::Refused) => return Err(Refusal::Denied),
            Err(NoConsent) => return Err(Refusal::NoPrompt),
        }
    }
    Ok(derive(master, app_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A prompt that answers from a script and counts the questions it was asked.
    struct Scripted {
        answers: Vec<Result<Consent, NoConsent>>,
        asked: Vec<String>,
    }

    impl Scripted {
        fn answering(answers: Vec<Result<Consent, NoConsent>>) -> Scripted {
            Scripted {
                answers,
                asked: Vec::new(),
            }
        }
    }

    impl ConsentPrompt for Scripted {
        fn ask(&mut self, app_id: &str) -> Result<Consent, NoConsent> {
            self.asked.push(app_id.to_owned());
            self.answers.remove(0)
        }
    }

    const MASTER: &[u8] = b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn derivation_is_deterministic() {
        assert_eq!(*derive(MASTER, "org.example.App"), *derive(MASTER, "org.example.App"));
    }

    #[test]
    fn distinct_apps_derive_distinct_secrets() {
        let names = [
            "org.example.App",
            "org.example.Other",
            "org.example",
            "a",
            "a/b",
            "org.example.Appx",
        ];
        for (i, first) in names.iter().enumerate() {
            for second in &names[i + 1..] {
                assert_ne!(
                    *derive(MASTER, first),
                    *derive(MASTER, second),
                    "{first} and {second} share a secret"
                );
            }
        }
    }

    #[test]
    fn a_different_master_derives_a_different_secret() {
        let other = [0x5a_u8; 64];
        assert_ne!(*derive(MASTER, "org.example.App"), *derive(&other, "org.example.App"));
    }

    #[test]
    fn an_allowed_app_is_not_asked_again() {
        let mut grants = Grants::default();
        let mut prompt = Scripted::answering(vec![Ok(Consent::Allowed)]);
        let first = retrieve(MASTER, &mut grants, &mut prompt, "org.example.App").expect("allowed");
        let second = retrieve(MASTER, &mut grants, &mut prompt, "org.example.App").expect("granted");
        assert_eq!(*first, *second);
        assert_eq!(prompt.asked, vec!["org.example.App".to_owned()]);
        assert!(grants.is_granted("org.example.App"));
    }

    #[test]
    fn a_refused_prompt_returns_no_secret() {
        let mut grants = Grants::default();
        let mut prompt = Scripted::answering(vec![Ok(Consent::Refused)]);
        let refusal = retrieve(MASTER, &mut grants, &mut prompt, "org.example.App")
            .expect_err("a refused request gets no secret");
        assert_eq!(refusal, Refusal::Denied);
        assert_eq!(refusal.response(), 1);
        assert!(!grants.is_granted("org.example.App"));
    }

    #[test]
    fn a_refusal_is_not_remembered() {
        let mut grants = Grants::default();
        let mut prompt = Scripted::answering(vec![Ok(Consent::Refused), Ok(Consent::Allowed)]);
        assert_eq!(
            retrieve(MASTER, &mut grants, &mut prompt, "org.example.App"),
            Err(Refusal::Denied)
        );
        assert!(retrieve(MASTER, &mut grants, &mut prompt, "org.example.App").is_ok());
        assert_eq!(prompt.asked.len(), 2, "the second request asks again");
    }

    #[test]
    fn no_consent_provider_fails_closed() {
        let mut grants = Grants::default();
        let mut prompt = Scripted::answering(vec![Err(NoConsent)]);
        let refusal =
            retrieve(MASTER, &mut grants, &mut prompt, "org.example.App").expect_err("no provider means no secret");
        assert_eq!(refusal, Refusal::NoPrompt);
        assert_eq!(refusal.response(), 2);
        assert!(!grants.is_granted("org.example.App"));
    }

    #[test]
    fn an_empty_app_id_is_refused_before_asking() {
        let mut grants = Grants::default();
        let mut prompt = Scripted::answering(vec![]);
        assert_eq!(retrieve(MASTER, &mut grants, &mut prompt, ""), Err(Refusal::NoApp));
        assert!(prompt.asked.is_empty());
    }

    fn scratch(label: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        std::env::temp_dir().join(format!("gnoblin-secret-{label}-{}-{nanos}.db", std::process::id()))
    }

    #[test]
    fn the_master_is_created_once_and_read_back() {
        let path = scratch("master");
        let pass = gnoblin_keyring_store::Passphrase::new("correct horse".to_owned()).expect("a passphrase");
        let mut keyring = Keyring::create(&path, &pass).expect("create the store");
        let collection = keyring.create_collection("Login").expect("collection");
        keyring.set_alias(DEFAULT_ALIAS, collection).expect("alias");
        let first = master_in(&mut keyring).expect("created");
        assert_eq!(first.len(), MASTER_LEN);
        let second = master_in(&mut keyring).expect("read back");
        assert_eq!(*first, *second, "a second run reads the master, it does not replace it");
        let items = keyring.search_items(&[MASTER_ATTRIBUTE]).expect("search");
        assert_eq!(items.len(), 1, "exactly one master item");
        drop(keyring);
        let _ = std::fs::remove_file(&path);
    }
}
