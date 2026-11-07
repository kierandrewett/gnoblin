//! The Secret Service objects: `Service`, `Session`, `Collection`, `Item` and `Prompt`.
//!
//! Every object reads and writes the store through one [`State`]. No method holds that lock
//! across an await, so a slow bus call cannot block another one on the same store.
//!
//! The store is `None` while the keyring is locked. Then the objects stay registered, and
//! every method that needs the store answers `org.freedesktop.Secret.Error.IsLocked`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gnoblin_keyring_store::{CollectionId, ItemId, Keyring, KeyringError};
use zbus::ObjectServer;
use zbus::object_server::SignalEmitter;
use zbus::{OwnedObjectPath, OwnedVariant, Variant};

use crate::dh::{self, DhSessionKey};

/// The service's object root. Every path below it is one object of this crate.
pub const SERVICE_PATH: &str = "/org/freedesktop/secrets";
/// Collections live here, and an item is a child of its collection.
const COLLECTION_PREFIX: &str = "/org/freedesktop/secrets/collection/";
const SESSION_PREFIX: &str = "/org/freedesktop/secrets/session/";
const PROMPT_PREFIX: &str = "/org/freedesktop/secrets/prompt/";
/// The alias the default collection answers to. Clients read it with `ReadAlias`.
pub const DEFAULT_ALIAS: &str = "default";
/// The label of the collection created on first run.
const DEFAULT_LABEL: &str = "Login";

/// The path a method returns when there is no object to prompt for.
const NO_OBJECT: &str = "/";

/// A secret as the spec sends it: `(session, parameters, value, content type)`. For the DH
/// session `parameters` is the IV and `value` the AES-128-CBC ciphertext.
pub type WireSecret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

/// The state every object shares. `keyring` is `None` while the store is locked.
pub struct SecretServiceState {
    keyring: Option<Keyring>,
    sessions: HashMap<u64, DhSessionKey>,
    next_id: u64,
}

/// The shared state handle every object holds.
pub type State = Arc<Mutex<SecretServiceState>>;

/// Why a bus call failed. Each variant is a name the Secret Service spec defines.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop")]
pub enum SecretError {
    #[zbus(error)]
    ZBus(zbus::Error),
    #[zbus(name = "DBus.Error.Failed")]
    Failed(String),
    #[zbus(name = "DBus.Error.InvalidArgs")]
    InvalidArgs(String),
    #[zbus(name = "DBus.Error.NotSupported")]
    NotSupported(String),
    #[zbus(name = "Secret.Error.IsLocked")]
    IsLocked(String),
    #[zbus(name = "Secret.Error.NoSession")]
    NoSession(String),
    #[zbus(name = "Secret.Error.NoSuchObject")]
    NoSuchObject(String),
}

impl From<KeyringError> for SecretError {
    fn from(error: KeyringError) -> Self {
        SecretError::Failed(error.to_string())
    }
}

fn failed<E: std::fmt::Display>(error: E) -> SecretError {
    SecretError::Failed(error.to_string())
}

fn refused(what: &str) -> SecretError {
    SecretError::NotSupported(format!("{what} is not offered over the bus"))
}

/// Make the shared state. A `None` store is a locked keyring.
pub fn new_state(keyring: Option<Keyring>) -> State {
    Arc::new(Mutex::new(SecretServiceState {
        keyring,
        sessions: HashMap::new(),
        next_id: 1,
    }))
}

fn lock(state: &State) -> MutexGuard<'_, SecretServiceState> {
    // A poisoned lock means a method panicked while holding the store. The store itself is
    // transactional, so the state is still usable and refusing every later call would be worse.
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

impl SecretServiceState {
    fn fresh_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn open(&self) -> Result<&Keyring, SecretError> {
        self.keyring
            .as_ref()
            .ok_or_else(|| SecretError::IsLocked("the keyring is locked".to_owned()))
    }
}

fn collection_path(id: CollectionId) -> String {
    format!("{COLLECTION_PREFIX}{}", id.raw())
}

fn item_path(collection: CollectionId, id: ItemId) -> String {
    format!("{COLLECTION_PREFIX}{}/{}", collection.raw(), id.raw())
}

/// The item a path names, if it is an item path of this service.
fn item_id(path: &str) -> Option<ItemId> {
    let rest = path.strip_prefix(COLLECTION_PREFIX)?;
    let (_, item) = rest.split_once('/')?;
    item.parse::<i64>().ok().map(ItemId::from_raw)
}

/// The number at the end of a session path, which is the key the session table uses.
fn session_id(path: &str) -> Option<u64> {
    path.strip_prefix(SESSION_PREFIX)?.parse().ok()
}

fn owned_path(path: &str) -> Result<OwnedObjectPath, SecretError> {
    OwnedObjectPath::try_from(path.to_owned()).map_err(failed)
}

fn seconds(ms: i64) -> u64 {
    u64::try_from(ms / 1000).unwrap_or(0)
}

/// Create the default collection on first run, and point the `default` alias at it. A store
/// that already has the alias is left alone.
pub fn ensure_default_collection(state: &State) -> Result<(), KeyringError> {
    let shared = lock(state);
    let Some(keyring) = shared.keyring.as_ref() else {
        return Ok(());
    };
    if keyring.alias(DEFAULT_ALIAS)?.is_some() {
        return Ok(());
    }
    let collection = keyring.create_collection(DEFAULT_LABEL)?;
    keyring.set_alias(DEFAULT_ALIAS, collection)?;
    Ok(())
}

/// Register the collection and item objects the store holds. A path already registered is left
/// as it is, so this is safe to run again after a change.
pub async fn export_tree(server: &ObjectServer, state: &State) -> Result<(), SecretError> {
    let tree = {
        let shared = lock(state);
        let Some(keyring) = shared.keyring.as_ref() else {
            return Ok(());
        };
        let mut tree = Vec::new();
        for (collection, _) in keyring.collections()? {
            for item in keyring.items_in(collection)? {
                tree.push((collection, Some(item)));
            }
            tree.push((collection, None));
        }
        tree
    };
    for (collection, item) in tree {
        match item {
            None => {
                let path = collection_path(collection);
                server
                    .at(
                        path.as_str(),
                        CollectionObject {
                            state: state.clone(),
                            id: collection,
                        },
                    )
                    .await
                    .map_err(failed)?;
            }
            Some(item) => {
                let path = item_path(collection, item);
                server
                    .at(
                        path.as_str(),
                        ItemObject {
                            state: state.clone(),
                            id: item,
                        },
                    )
                    .await
                    .map_err(failed)?;
            }
        }
    }
    Ok(())
}

/// The `org.freedesktop.Secret.Service` object at [`SERVICE_PATH`].
pub struct ServiceObject {
    pub state: State,
}

#[zbus::interface(name = "org.freedesktop.Secret.Service")]
impl ServiceObject {
    /// Only `dh-ietf1024-sha256-aes128-cbc-pkcs7` is offered. `plain` is refused, because a
    /// secret sent in the clear over the session bus is readable by every local process.
    async fn open_session(
        &self,
        algorithm: &str,
        input: Variant<'_>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<(OwnedVariant, OwnedObjectPath), SecretError> {
        if algorithm != dh::ALGORITHM {
            return Err(SecretError::NotSupported(format!("algorithm {algorithm}")));
        }
        let peer =
            Vec::<u8>::try_from(input).map_err(|_| SecretError::InvalidArgs("the DH input is not bytes".to_owned()))?;
        let (public, key) = dh::accept(&peer).map_err(failed)?;
        let (id, path) = {
            let mut shared = lock(&self.state);
            let id = shared.fresh_id();
            shared.sessions.insert(id, key);
            (id, format!("{SESSION_PREFIX}{id}"))
        };
        server
            .at(
                path.as_str(),
                SecretSessionObject {
                    state: self.state.clone(),
                    id,
                    path: path.clone(),
                },
            )
            .await
            .map_err(failed)?;
        let output = OwnedVariant::try_from(Variant::from(public)).map_err(failed)?;
        Ok((output, owned_path(&path)?))
    }

    /// Items that match, split into those the caller may read and those it may not. While the
    /// keyring is locked both lists are empty: the attributes are inside the cipher, so there is
    /// nothing to match against. See `search_locked`.
    async fn search_items(
        &self,
        attributes: HashMap<String, String>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<(Vec<OwnedObjectPath>, Vec<OwnedObjectPath>), SecretError> {
        let found = {
            let shared = lock(&self.state);
            match shared.keyring.as_ref() {
                None => return Ok(search_locked()),
                Some(keyring) => matching(keyring, &attributes)?,
            }
        };
        let mut unlocked = Vec::new();
        for (collection, item) in found {
            let path = item_path(collection, item);
            server
                .at(
                    path.as_str(),
                    ItemObject {
                        state: self.state.clone(),
                        id: item,
                    },
                )
                .await
                .map_err(failed)?;
            unlocked.push(owned_path(&path)?);
        }
        Ok((unlocked, Vec::new()))
    }

    /// The store is opened by the unlock path, which is a later phase. Until then the answer is a
    /// prompt that is dismissed at once, so the caller learns the keyring is still locked.
    async fn unlock(
        &self,
        objects: Vec<OwnedObjectPath>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<(Vec<OwnedObjectPath>, OwnedObjectPath), SecretError> {
        if lock(&self.state).keyring.is_some() {
            return Ok((objects, owned_path(NO_OBJECT)?));
        }
        let id = lock(&self.state).fresh_id();
        let path = format!("{PROMPT_PREFIX}{id}");
        server
            .at(path.as_str(), PromptObject { path: path.clone() })
            .await
            .map_err(failed)?;
        Ok((Vec::new(), owned_path(&path)?))
    }

    /// Locking drops the store handle. The file stays on disk, and nothing reads it again until
    /// the unlock path reopens it.
    async fn lock(
        &self,
        objects: Vec<OwnedObjectPath>,
    ) -> Result<(Vec<OwnedObjectPath>, OwnedObjectPath), SecretError> {
        {
            let mut shared = lock(&self.state);
            shared.keyring = None;
            shared.sessions.clear();
        }
        Ok((objects, owned_path(NO_OBJECT)?))
    }

    /// The secrets of `items`, each encrypted for `session`. Items that are not in the store are
    /// left out, as the spec says.
    async fn get_secrets(
        &self,
        items: Vec<OwnedObjectPath>,
        session: OwnedObjectPath,
    ) -> Result<HashMap<OwnedObjectPath, WireSecret>, SecretError> {
        let mut secrets = HashMap::new();
        for path in items {
            // The item is read here, not through `ItemObject`, so that one locked check covers
            // the whole call.
            let encrypted = encrypt_item(&self.state, &session, path.as_str())?;
            if let Some(secret) = encrypted {
                secrets.insert(path, secret);
            }
        }
        Ok(secrets)
    }

    /// The collection the alias names, or `/` when there is none or the keyring is locked.
    async fn read_alias(&self, name: &str) -> Result<OwnedObjectPath, SecretError> {
        let shared = lock(&self.state);
        let Some(keyring) = shared.keyring.as_ref() else {
            return owned_path(NO_OBJECT);
        };
        match keyring.alias(name)? {
            Some(collection) => owned_path(&collection_path(collection)),
            None => owned_path(NO_OBJECT),
        }
    }

    async fn set_alias(&self, _name: &str, _collection: OwnedObjectPath) -> Result<(), SecretError> {
        Err(refused("SetAlias"))
    }

    async fn create_collection(
        &self,
        _properties: HashMap<String, OwnedVariant>,
        _alias: &str,
    ) -> Result<(OwnedObjectPath, OwnedObjectPath), SecretError> {
        Err(refused("CreateCollection"))
    }

    /// The collections of the open store, or none while it is locked.
    #[zbus(property)]
    async fn collections(&self) -> Result<Vec<OwnedObjectPath>, SecretError> {
        let shared = lock(&self.state);
        let Some(keyring) = shared.keyring.as_ref() else {
            return Ok(Vec::new());
        };
        keyring
            .collections()?
            .into_iter()
            .map(|(id, _)| owned_path(&collection_path(id)))
            .collect()
    }
}

/// Search a locked keyring. It answers nothing: the attributes sit inside the SQLCipher file, and
/// a searchable index outside the cipher is the choice that is still open.
///
/// The design asks whether hashed attributes should be kept outside the cipher so that search
/// works while locked, as GNOME does. That is open in `design/keyring.md` under "Needs Kieran"
/// item 1. Until it is decided, a locked search returns nothing. Choosing the other option would
/// need a new column or table, with a migration from the phase 1 schema, which this function is
/// the only place to change.
fn search_locked() -> (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) {
    (Vec::new(), Vec::new())
}

fn matching(
    keyring: &Keyring,
    attributes: &HashMap<String, String>,
) -> Result<Vec<(CollectionId, ItemId)>, SecretError> {
    let pairs: Vec<(&str, &str)> = attributes
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    let mut found = Vec::new();
    for id in keyring.search_items(&pairs)? {
        if let Some(item) = keyring.item(id)? {
            found.push((item.collection, id));
        }
    }
    Ok(found)
}

/// Encrypt one item's secret for a session. `None` means the path names no item.
fn encrypt_item(state: &State, session: &OwnedObjectPath, path: &str) -> Result<Option<WireSecret>, SecretError> {
    let shared = lock(state);
    let keyring = shared.open()?;
    let session_key = session_id(session.as_str())
        .and_then(|id| shared.sessions.get(&id))
        .ok_or_else(|| SecretError::NoSession(session.to_string()))?;
    let Some(id) = item_id(path) else {
        return Ok(None);
    };
    let Some(item) = keyring.item(id)? else {
        return Ok(None);
    };
    let (iv, value) = dh::encrypt(session_key, &item.secret).map_err(failed)?;
    Ok(Some((session.clone(), iv, value, "text/plain".to_owned())))
}

/// One collection of the store.
pub struct CollectionObject {
    pub state: State,
    pub id: CollectionId,
}

#[zbus::interface(name = "org.freedesktop.Secret.Collection")]
impl CollectionObject {
    async fn delete(&self) -> Result<OwnedObjectPath, SecretError> {
        Err(refused("Delete"))
    }

    async fn search_items(
        &self,
        attributes: HashMap<String, String>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<Vec<OwnedObjectPath>, SecretError> {
        let found: Vec<ItemId> = {
            let shared = lock(&self.state);
            let keyring = shared.open()?;
            let pairs: Vec<(&str, &str)> = attributes
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str()))
                .collect();
            let mine = keyring.items_in(self.id)?;
            keyring
                .search_items(&pairs)?
                .into_iter()
                .filter(|id| mine.contains(id))
                .collect()
        };
        let mut paths = Vec::new();
        for id in found {
            let path = item_path(self.id, id);
            server
                .at(
                    path.as_str(),
                    ItemObject {
                        state: self.state.clone(),
                        id,
                    },
                )
                .await
                .map_err(failed)?;
            paths.push(owned_path(&path)?);
        }
        Ok(paths)
    }

    async fn create_item(
        &self,
        _properties: HashMap<String, OwnedVariant>,
        _secret: WireSecret,
        _replace: bool,
    ) -> Result<(OwnedObjectPath, OwnedObjectPath), SecretError> {
        Err(refused("CreateItem"))
    }

    #[zbus(property)]
    async fn items(&self) -> Result<Vec<OwnedObjectPath>, SecretError> {
        let shared = lock(&self.state);
        let keyring = shared.open()?;
        keyring
            .items_in(self.id)?
            .into_iter()
            .map(|id| owned_path(&item_path(self.id, id)))
            .collect()
    }

    #[zbus(property)]
    async fn label(&self) -> Result<String, SecretError> {
        let shared = lock(&self.state);
        let keyring = shared.open()?;
        keyring
            .collections()?
            .into_iter()
            .find(|(id, _)| *id == self.id)
            .map(|(_, label)| label)
            .ok_or_else(|| SecretError::NoSuchObject(collection_path(self.id)))
    }

    #[zbus(property)]
    fn locked(&self) -> bool {
        lock(&self.state).keyring.is_none()
    }
}

/// One item of a collection. The attributes and label are readable, the secret through
/// `GetSecret` only.
pub struct ItemObject {
    pub state: State,
    pub id: ItemId,
}

#[zbus::interface(name = "org.freedesktop.Secret.Item")]
impl ItemObject {
    async fn delete(&self) -> Result<OwnedObjectPath, SecretError> {
        Err(refused("Delete"))
    }

    async fn get_secret(&self, session: OwnedObjectPath) -> Result<WireSecret, SecretError> {
        let path = item_path_of(self.state.clone(), self.id)?;
        encrypt_item(&self.state, &session, &path)?.ok_or_else(|| SecretError::NoSuchObject(path))
    }

    async fn set_secret(&self, _secret: WireSecret) -> Result<(), SecretError> {
        Err(refused("SetSecret"))
    }

    #[zbus(property)]
    async fn locked(&self) -> Result<bool, SecretError> {
        Ok(lock(&self.state).keyring.is_none())
    }

    #[zbus(property)]
    async fn attributes(&self) -> Result<HashMap<String, String>, SecretError> {
        let shared = lock(&self.state);
        let keyring = shared.open()?;
        let item = keyring
            .item(self.id)?
            .ok_or_else(|| SecretError::NoSuchObject(format!("item {}", self.id.raw())))?;
        Ok(item.attributes.into_iter().collect())
    }

    #[zbus(property)]
    async fn label(&self) -> Result<String, SecretError> {
        let shared = lock(&self.state);
        let keyring = shared.open()?;
        let item = keyring
            .item(self.id)?
            .ok_or_else(|| SecretError::NoSuchObject(format!("item {}", self.id.raw())))?;
        Ok(item.label)
    }

    #[zbus(property)]
    async fn created(&self) -> Result<u64, SecretError> {
        let shared = lock(&self.state);
        let item = shared
            .open()?
            .item(self.id)?
            .ok_or_else(|| SecretError::NoSuchObject(format!("item {}", self.id.raw())))?;
        Ok(seconds(item.created_ms))
    }

    #[zbus(property)]
    async fn modified(&self) -> Result<u64, SecretError> {
        let shared = lock(&self.state);
        let item = shared
            .open()?
            .item(self.id)?
            .ok_or_else(|| SecretError::NoSuchObject(format!("item {}", self.id.raw())))?;
        Ok(seconds(item.modified_ms))
    }
}

/// The object path of the item at `id`, read from its row so the collection is never guessed.
fn item_path_of(state: State, id: ItemId) -> Result<String, SecretError> {
    let shared = lock(&state);
    let item = shared
        .open()?
        .item(id)?
        .ok_or_else(|| SecretError::NoSuchObject(format!("item {}", id.raw())))?;
    Ok(item_path(item.collection, id))
}

/// One session of the DH exchange. It holds the key until `Close`.
pub struct SecretSessionObject {
    pub state: State,
    pub id: u64,
    pub path: String,
}

#[zbus::interface(name = "org.freedesktop.Secret.Session")]
impl SecretSessionObject {
    /// Drops the key and removes the session object.
    async fn close(&self, #[zbus(object_server)] server: &ObjectServer) -> Result<(), SecretError> {
        lock(&self.state).sessions.remove(&self.id);
        server
            .remove::<SecretSessionObject, _>(self.path.as_str())
            .await
            .map_err(failed)?;
        Ok(())
    }
}

/// A prompt the service raised. No prompt UI exists yet, so `Prompt` and `Dismiss` both finish
/// at once as dismissed, with no result. Each finished prompt removes its own object, so repeated
/// `Unlock` calls do not leak objects.
pub struct PromptObject {
    pub path: String,
}

#[zbus::interface(name = "org.freedesktop.Secret.Prompt")]
impl PromptObject {
    async fn prompt(
        &self,
        _window_id: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<(), SecretError> {
        finish_prompt(&self.path, &emitter, server).await
    }

    async fn dismiss(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<(), SecretError> {
        finish_prompt(&self.path, &emitter, server).await
    }

    #[zbus(signal)]
    async fn completed(emitter: &SignalEmitter<'_>, dismissed: bool, result: Variant<'_>) -> zbus::Result<()>;
}

async fn finish_prompt(path: &str, emitter: &SignalEmitter<'_>, server: &ObjectServer) -> Result<(), SecretError> {
    PromptObject::completed(emitter, true, Variant::from("")).await?;
    server.remove::<PromptObject, _>(path).await.map_err(failed)?;
    Ok(())
}
