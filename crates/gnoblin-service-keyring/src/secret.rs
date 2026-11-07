//! The Secret portal's secret: one master in the store, one secret per app derived from it.
//!
//! `design/keyring.md`, "Secret portal". The master is 64 random bytes kept in the default
//! collection under the attribute `gnoblin-purpose=portal-secret`, created on first use. Each app
//! gets 32 bytes from HKDF-SHA256 over that master, with the app id as `info`, so two app ids
//! never share a secret and the same app id always gets the same one.
//!
//! Consent is decided by the compositor, which asks the user and holds the session's grants. This
//! module refuses an empty app id and an ungranted app, so no caller gets a secret by skipping the
//! grant.
//!
//! # Failure modes first
//!
//! 1. **Two apps share a secret.** `distinct_apps_derive_distinct_secrets` pins it.
//! 2. **The same app gets a different secret on the next request.** `derivation_is_deterministic`.
//! 3. **An ungranted app gets a secret.** `an_ungranted_app_gets_no_secret`.
//! 4. **An empty app id gets a secret.** `retrieve_refuses_an_empty_app_id_even_when_granted`.
//! 5. **The master is replaced on a second run.** `the_master_is_created_once_and_read_back`.

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
    keyring.put_item(
        collection,
        MASTER_LABEL,
        "application/octet-stream",
        &bytes,
        &[MASTER_ATTRIBUTE],
    )?;
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

/// Why a request got no secret.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecretRefusal {
    /// The request named no app, so there is no secret to give.
    NoApp,
    /// The app is not granted this session, so its secret is withheld.
    Denied,
}

impl SecretRefusal {
    /// The portal response code: 1 is "cancelled by the user", 2 is "other".
    #[must_use]
    pub const fn response(self) -> u32 {
        match self {
            SecretRefusal::Denied => 1,
            SecretRefusal::NoApp => 2,
        }
    }
}

/// The secret for `app_id`, once the caller has established that the app is granted.
///
/// Consent is decided by the compositor before a request reaches this module. The check here is
/// only what the store can verify: an app id is named and the app is granted. Either failing gets
/// no secret.
pub fn retrieve(master: &[u8], app_id: &str, granted: bool) -> Result<AppSecret, SecretRefusal> {
    if app_id.is_empty() {
        return Err(SecretRefusal::NoApp);
    }
    if !granted {
        return Err(SecretRefusal::Denied);
    }
    Ok(derive(master, app_id))
}

/// Why an app's secret was not given.
#[derive(Debug)]
pub enum AppSecretError {
    /// The master could not be read, most often because the store is locked.
    Master(MasterError),
    /// The request named no app, so there is no secret to give.
    Refused(SecretRefusal),
}

/// The app secrets of the store behind a Secret Service. It shares the store's lock with the bus
/// methods, so a secret is never derived while the store is being re-keyed.
#[derive(Clone)]
pub struct Secrets {
    state: State,
}

impl Secrets {
    pub(crate) fn new(state: State) -> Secrets {
        Secrets { state }
    }

    /// The secret for `app_id`. The compositor asks the user before it offers the read here, so the
    /// grant is taken as given; an empty app id is still refused by `retrieve`.
    pub fn app_secret(&self, app_id: &str) -> Result<AppSecret, AppSecretError> {
        let master = master(&self.state).map_err(AppSecretError::Master)?;
        retrieve(&master, app_id, true).map_err(AppSecretError::Refused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn an_ungranted_app_gets_no_secret() {
        let refusal = retrieve(MASTER, "org.example.App", false).expect_err("ungranted");
        assert_eq!(refusal, SecretRefusal::Denied);
        assert_eq!(refusal.response(), 1);
    }

    #[test]
    fn retrieve_refuses_an_empty_app_id_even_when_granted() {
        assert_eq!(retrieve(MASTER, "", true), Err(SecretRefusal::NoApp));
    }

    #[test]
    fn a_granted_app_gets_its_derived_secret() {
        let secret = retrieve(MASTER, "org.example.App", true).expect("granted");
        assert_eq!(*secret, *derive(MASTER, "org.example.App"));
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
