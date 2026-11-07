//! The Secret Service objects: `Service`, `Session`, `Collection`, `Item` and `Prompt`.
//!
//! Every object reads and writes the store through one [`State`]. No method holds that lock
//! across an await, so a slow bus call cannot block another one on the same store.
//!
//! The store is `None` while the keyring is locked. Then the objects stay registered, and
//! every method that needs the store answers `org.freedesktop.Secret.Error.IsLocked`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use gnoblin_keyring_store::{CollectionId, ItemId, Keyring, KeyringError};
use zbus::ObjectServer;
use zbus::object_server::SignalEmitter;
use zbus::{OwnedObjectPath, OwnedVariant, Variant};
use zeroize::Zeroizing;

use crate::dh::{self, DhSessionKey};
use crate::log;
use crate::unlock::UnlockKeySource;

/// The service's object root. Every path below it is one object of this crate.
pub const SERVICE_PATH: &str = "/org/freedesktop/secrets";
/// Collections live here, and an item is a child of its collection.
const COLLECTION_PREFIX: &str = "/org/freedesktop/secrets/collection/";
const SESSION_PREFIX: &str = "/org/freedesktop/secrets/session/";
const PROMPT_PREFIX: &str = "/org/freedesktop/secrets/prompt/";
/// The alias the default collection answers to. Clients read it with `ReadAlias`.
pub const DEFAULT_ALIAS: &str = "default";
/// The fixed path of the default collection. A client holds it across a lock and an unlock, and
/// the collection behind it follows the `default` alias.
pub const DEFAULT_COLLECTION_PATH: &str = "/org/freedesktop/secrets/collection/default";
/// The label of the collection created on first run.
const DEFAULT_LABEL: &str = "Login";
/// The property names `CreateItem` reads its label and attributes from.
const ITEM_LABEL: &str = "org.freedesktop.Secret.Item.Label";
const ITEM_ATTRIBUTES: &str = "org.freedesktop.Secret.Item.Attributes";
/// The property name `CreateCollection` reads its label from.
const COLLECTION_LABEL: &str = "org.freedesktop.Secret.Collection.Label";

/// The path a method returns when there is no object to prompt for.
const NO_OBJECT: &str = "/";

/// A secret as the spec sends it: `(session, parameters, value, content type)`. For the DH
/// session `parameters` is the IV and `value` the AES-128-CBC ciphertext.
pub type WireSecret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

/// A write a client made while the store was locked. It waits under the prompt that will unlock
/// the store, and is applied once, when that prompt completes undismissed. The secret is
/// decrypted when the call arrives, because the client may close its session before the user
/// answers; only the plain bytes are kept, in a buffer that is zeroed when it is dropped.
pub(crate) enum PendingWrite {
    CreateItem {
        target: CollectionTarget,
        label: String,
        content_type: String,
        secret: Zeroizing<Vec<u8>>,
        attributes: Vec<(String, String)>,
        replace: bool,
    },
}

/// One item a deferred write produced, so the async side can register its object and signal.
#[derive(Debug)]
struct AppliedItem {
    collection: CollectionId,
    client: String,
    item: ItemId,
    created: bool,
}

/// The state every object shares. `keyring` is `None` while the store is locked.
pub struct SecretServiceState {
    keyring: Option<Keyring>,
    sessions: HashMap<u64, DhSessionKey>,
    next_id: u64,
    /// Writes waiting for an unlock, keyed by the path of the prompt each one is waiting on.
    pending: HashMap<String, Vec<PendingWrite>>,
    /// The unlock source, so a collection can raise an unlock prompt. Weak, because the source
    /// already holds this state.
    source: Option<Weak<Mutex<UnlockKeySource>>>,
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

/// Make the shared state. A `None` store is a locked keyring.
pub fn new_state(keyring: Option<Keyring>) -> State {
    Arc::new(Mutex::new(SecretServiceState {
        keyring,
        sessions: HashMap::new(),
        next_id: 1,
        pending: HashMap::new(),
        source: None,
    }))
}

pub(crate) fn lock(state: &State) -> MutexGuard<'_, SecretServiceState> {
    // A poisoned lock means a method panicked while holding the store. The store itself is
    // transactional, so the state is still usable and refusing every later call would be worse.
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Whether the store is open. The unlock path checks this before it prompts.
pub(crate) fn is_unlocked(state: &State) -> bool {
    lock(state).keyring.is_some()
}

/// Install an open store, or drop it with `None`. Dropping the handle leaves the file on disk.
pub(crate) fn install_keyring(state: &State, keyring: Option<Keyring>) {
    lock(state).keyring = keyring;
}

/// Run `f` against the open store, or return `None` while the store is locked. The portal
/// secret reads and writes its master item through this, under the same lock the bus methods
/// take, so a locked store is never touched.
pub(crate) fn with_open_keyring<T>(state: &State, f: impl FnOnce(&mut Keyring) -> T) -> Option<T> {
    lock(state).keyring.as_mut().map(f)
}

/// Re-key `keyring` to `new` and install it, all under one lock. No other call sees the file
/// between the re-key and the install, and a failed re-key leaves the old handle in place.
pub(crate) fn rekey_and_install(
    state: &State,
    mut keyring: Keyring,
    new: &gnoblin_keyring_store::Passphrase,
) -> Result<(), KeyringError> {
    let mut shared = lock(state);
    keyring.rekey(new)?;
    shared.keyring = Some(keyring);
    Ok(())
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

    fn open_mut(&mut self) -> Result<&mut Keyring, SecretError> {
        self.keyring
            .as_mut()
            .ok_or_else(|| SecretError::IsLocked("the keyring is locked".to_owned()))
    }
}

fn collection_path(id: CollectionId) -> String {
    format!("{COLLECTION_PREFIX}{}", id.raw())
}

fn item_path(collection: CollectionId, id: ItemId) -> String {
    format!("{COLLECTION_PREFIX}{}/{}", collection.raw(), id.raw())
}

/// The collection a path names, if it is a collection path of this service.
fn collection_id(path: &str) -> Option<CollectionId> {
    let rest = path.strip_prefix(COLLECTION_PREFIX)?;
    rest.parse::<i64>().ok().map(CollectionId::from_raw)
}

/// The client-facing path of `id`. The default collection answers at [`DEFAULT_COLLECTION_PATH`]
/// whatever its id is, so a client holds one path across a lock and an unlock. Every other
/// collection keeps its numeric path.
fn client_path(keyring: &Keyring, id: CollectionId) -> Result<String, SecretError> {
    if keyring.alias(DEFAULT_ALIAS)? == Some(id) {
        return Ok(DEFAULT_COLLECTION_PATH.to_owned());
    }
    Ok(collection_path(id))
}

/// The collection the `default` alias names. With no alias there is no default collection.
fn default_collection(keyring: &Keyring) -> Result<CollectionId, SecretError> {
    keyring
        .alias(DEFAULT_ALIAS)?
        .ok_or_else(|| SecretError::NoSuchObject(DEFAULT_COLLECTION_PATH.to_owned()))
}

/// The collection an alias names. `/` names none and removes the alias. The default path names
/// whatever the `default` alias names now. Any other path must be a collection path, so an item
/// or session path is an argument error, not a silent removal.
fn alias_target(keyring: &Keyring, path: &str) -> Result<Option<CollectionId>, SecretError> {
    if path == NO_OBJECT {
        return Ok(None);
    }
    if path == DEFAULT_COLLECTION_PATH {
        return default_collection(keyring).map(Some);
    }
    collection_id(path)
        .map(Some)
        .ok_or_else(|| SecretError::InvalidArgs(format!("{path} is not a collection path")))
}

/// The collections a client sees. While locked the store cannot be read, but the default
/// collection is still listed at its fixed path, so a client can name it before the unlock.
fn collection_paths(keyring: Option<&Keyring>) -> Result<Vec<OwnedObjectPath>, SecretError> {
    let Some(keyring) = keyring else {
        return Ok(vec![owned_path(DEFAULT_COLLECTION_PATH)?]);
    };
    let mut paths = Vec::new();
    for (id, _) in keyring.collections()? {
        paths.push(owned_path(&client_path(keyring, id)?)?);
    }
    Ok(paths)
}

/// The collection `name` names, or `/` when there is none. While locked only the `default` alias
/// names a path: the fixed default path.
fn read_alias_in(keyring: Option<&Keyring>, name: &str) -> Result<OwnedObjectPath, SecretError> {
    let Some(keyring) = keyring else {
        let path = if name == DEFAULT_ALIAS {
            DEFAULT_COLLECTION_PATH
        } else {
            NO_OBJECT
        };
        return owned_path(path);
    };
    match keyring.alias(name)? {
        Some(collection) => owned_path(&client_path(keyring, collection)?),
        None => owned_path(NO_OBJECT),
    }
}

/// The label `CreateCollection` carries. A missing label is an argument error.
fn collection_label(mut properties: HashMap<String, OwnedVariant>) -> Result<String, SecretError> {
    let label = properties
        .remove(COLLECTION_LABEL)
        .ok_or_else(|| SecretError::InvalidArgs(format!("{COLLECTION_LABEL} is missing")))?;
    String::try_from(label).map_err(|error| SecretError::InvalidArgs(error.to_string()))
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
                            target: CollectionTarget::Id(collection),
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

/// Register the default collection's object at [`DEFAULT_COLLECTION_PATH`]. It is registered once
/// at startup, locked or not, because a client holds that path across a lock and an unlock.
pub async fn export_default_collection(server: &ObjectServer, state: &State) -> zbus::Result<()> {
    server
        .at(
            DEFAULT_COLLECTION_PATH,
            CollectionObject {
                state: state.clone(),
                target: CollectionTarget::Default,
            },
        )
        .await?;
    Ok(())
}

/// The `org.freedesktop.Secret.Service` object at [`SERVICE_PATH`].
pub struct ServiceObject {
    pub state: State,
    /// The unlock path. `Unlock` hands the prompt object it creates to this source.
    pub source: Arc<Mutex<UnlockKeySource>>,
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

    /// Asks for the login password. This only creates the prompt object: the request is offered
    /// when the caller calls `Prompt` on it, so a caller that never prompts never asks.
    async fn unlock(
        &self,
        objects: Vec<OwnedObjectPath>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<(Vec<OwnedObjectPath>, OwnedObjectPath), SecretError> {
        if lock(&self.state).keyring.is_some() {
            return Ok((objects, owned_path(NO_OBJECT)?));
        }
        let path = new_prompt(&self.state, Arc::clone(&self.source), server).await?;
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
        read_alias_in(shared.keyring.as_ref(), name)
    }

    /// Points `name` at `collection`, or removes the alias when `collection` is `/`. A locked
    /// keyring answers `IsLocked`. A path that is not a collection is an argument error.
    async fn set_alias(&self, name: &str, collection: OwnedObjectPath) -> Result<(), SecretError> {
        let mut shared = lock(&self.state);
        let keyring = shared.open_mut()?;
        match alias_target(keyring, collection.as_str())? {
            None => {
                keyring.clear_alias(name)?;
            }
            Some(id) => {
                if !keyring.collections()?.iter().any(|(known, _)| *known == id) {
                    return Err(SecretError::NoSuchObject(collection.as_str().to_owned()));
                }
                keyring.set_alias(name, id)?;
            }
        }
        Ok(())
    }

    /// Creates a collection labelled by its `Label` property. When `alias` already names one, that
    /// collection is returned and nothing is created, as the spec says. The prompt is `/`: creating
    /// needs no unlock, because a locked keyring refuses with `IsLocked`.
    async fn create_collection(
        &self,
        properties: HashMap<String, OwnedVariant>,
        alias: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<(OwnedObjectPath, OwnedObjectPath), SecretError> {
        let label = collection_label(properties)?;
        let (id, created, client) = {
            let mut shared = lock(&self.state);
            let keyring = shared.open_mut()?;
            let existing = if alias.is_empty() { None } else { keyring.alias(alias)? };
            let (id, created) = match existing {
                Some(id) => (id, false),
                None => {
                    let id = keyring.create_collection(&label)?;
                    if !alias.is_empty() {
                        keyring.set_alias(alias, id)?;
                    }
                    (id, true)
                }
            };
            let client = client_path(keyring, id)?;
            (id, created, client)
        };
        let path = collection_path(id);
        let collection = owned_path(&client)?;
        if created {
            server
                .at(
                    path.as_str(),
                    CollectionObject {
                        state: self.state.clone(),
                        target: CollectionTarget::Id(id),
                    },
                )
                .await
                .map_err(failed)?;
            ServiceObject::collection_created(&emitter, &collection)
                .await
                .map_err(failed)?;
        }
        Ok((collection, owned_path(NO_OBJECT)?))
    }

    /// The collections of the open store, or none while it is locked.
    #[zbus(property)]
    async fn collections(&self) -> Result<Vec<OwnedObjectPath>, SecretError> {
        let shared = lock(&self.state);
        collection_paths(shared.keyring.as_ref())
    }

    /// Sent on the service after a collection is deleted, so clients drop it from their lists.
    #[zbus(signal)]
    async fn collection_deleted(emitter: &SignalEmitter<'_>, collection: &OwnedObjectPath) -> zbus::Result<()>;

    /// Sent on the service after `CreateCollection` makes a collection, so clients learn of it.
    #[zbus(signal)]
    async fn collection_created(emitter: &SignalEmitter<'_>, collection: &OwnedObjectPath) -> zbus::Result<()>;
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
    Ok(Some((session.clone(), iv, value, item.content_type.clone())))
}

/// Decrypt a secret a client sent over its DH session, with the content type the client sent it
/// under. The store is checked first, so a locked keyring answers `IsLocked` whatever the client
/// sent. A ciphertext that does not decrypt under its session is an argument error.
fn plain_secret(shared: &SecretServiceState, secret: &WireSecret) -> Result<(Zeroizing<Vec<u8>>, String), SecretError> {
    shared.open()?;
    decrypt_wire(shared, secret)
}

/// [`plain_secret`] without the lock check. A deferred write decrypts while the store is still
/// locked, so the session key is used before the client can close its session.
fn decrypt_wire(shared: &SecretServiceState, secret: &WireSecret) -> Result<(Zeroizing<Vec<u8>>, String), SecretError> {
    let (session, iv, cipher, content_type) = secret;
    let key = session_id(session.as_str())
        .and_then(|id| shared.sessions.get(&id))
        .ok_or_else(|| SecretError::NoSession(session.to_string()))?;
    let plain = dh::decrypt(key, iv, cipher)
        .map_err(|_| SecretError::InvalidArgs("the secret does not decrypt under its session".to_owned()))?;
    Ok((plain, content_type.clone()))
}

/// The label and attributes `CreateItem` carries. A missing label is an argument error. Missing
/// attributes are an empty set, which the spec allows. The attributes come back sorted by name,
/// the order the store keeps them in, so an exact match can compare them directly.
fn item_properties(
    mut properties: HashMap<String, OwnedVariant>,
) -> Result<(String, Vec<(String, String)>), SecretError> {
    let label = properties
        .remove(ITEM_LABEL)
        .ok_or_else(|| SecretError::InvalidArgs(format!("{ITEM_LABEL} is missing")))?;
    let label = String::try_from(label).map_err(|error| SecretError::InvalidArgs(error.to_string()))?;
    let mut attributes: Vec<(String, String)> = match properties.remove(ITEM_ATTRIBUTES) {
        Some(value) => HashMap::<String, String>::try_from(value)
            .map_err(|error| SecretError::InvalidArgs(error.to_string()))?
            .into_iter()
            .collect(),
        None => Vec::new(),
    };
    attributes.sort();
    Ok((label, attributes))
}

/// Write one item into `collection`. With `replace`, an item of that collection whose attributes
/// are exactly `attributes` is updated in place and keeps its id. Otherwise a new item is added,
/// so a search that matches only a superset never overwrites anything. Returns the id and whether
/// the item was created. `attributes` must be sorted by name, as [`item_properties`] sorts them.
fn write_item(
    keyring: &mut Keyring,
    collection: CollectionId,
    label: &str,
    content_type: &str,
    secret: &[u8],
    attributes: &[(String, String)],
    replace: bool,
) -> Result<(ItemId, bool), SecretError> {
    let pairs: Vec<(&str, &str)> = attributes
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    if replace {
        for id in keyring.search_items(&pairs)? {
            let existing = keyring.item(id)?;
            if existing.is_some_and(|item| item.collection == collection && item.attributes == attributes) {
                keyring.update_item(id, label, content_type, secret, &pairs)?;
                return Ok((id, false));
            }
        }
    }
    let id = keyring.put_item(collection, label, content_type, secret, &pairs)?;
    Ok((id, true))
}

/// Which collection a [`CollectionObject`] answers for. The default object names whatever the
/// `default` alias names when a method runs, so it follows the alias across changes.
#[derive(Clone, Copy, Debug)]
pub enum CollectionTarget {
    /// A collection at its numeric path. Every collection has one while the store is open.
    Id(CollectionId),
    /// The collection the `default` alias names, at [`DEFAULT_COLLECTION_PATH`].
    Default,
}

/// Reads a collection's answer while the store is locked. The default collection has a fixed
/// label and no items to show, so it answers `fallback`. Any other collection is `IsLocked`.
fn locked_read<T>(target: CollectionTarget, fallback: T) -> Result<T, SecretError> {
    match target {
        CollectionTarget::Default => Ok(fallback),
        CollectionTarget::Id(_) => Err(SecretError::IsLocked("the keyring is locked".to_owned())),
    }
}

/// One collection of the store, answering at its path.
pub struct CollectionObject {
    pub state: State,
    pub target: CollectionTarget,
}

impl CollectionObject {
    /// The collection this object names in the open store. A default object with no `default`
    /// alias answers `NoSuchObject`.
    fn id_in(&self, keyring: &Keyring) -> Result<CollectionId, SecretError> {
        match self.target {
            CollectionTarget::Id(id) => Ok(id),
            CollectionTarget::Default => default_collection(keyring),
        }
    }
}

#[zbus::interface(name = "org.freedesktop.Secret.Collection")]
impl CollectionObject {
    /// Deletes the collection and its items, then tells the service so clients drop it. Removing
    /// the collection drops its alias too, since the store cascades to aliases. The client path is
    /// read before the delete, because the delete drops the alias that names the default path. The
    /// default object stays registered, so the next unlock finds it.
    async fn delete(&self, #[zbus(object_server)] server: &ObjectServer) -> Result<OwnedObjectPath, SecretError> {
        let (id, client, items) = {
            let mut shared = lock(&self.state);
            let keyring = shared.open_mut()?;
            let id = self.id_in(keyring)?;
            let client = client_path(keyring, id)?;
            let items = keyring.items_in(id)?;
            if !keyring.delete_collection(id)? {
                return Err(SecretError::NoSuchObject(client));
            }
            (id, client, items)
        };
        for item in items {
            let path = item_path(id, item);
            server.remove::<ItemObject, _>(path.as_str()).await.map_err(failed)?;
        }
        let path = collection_path(id);
        server
            .remove::<CollectionObject, _>(path.as_str())
            .await
            .map_err(failed)?;
        let service = server
            .interface::<_, ServiceObject>(SERVICE_PATH)
            .await
            .map_err(failed)?;
        ServiceObject::collection_deleted(service.signal_emitter(), &owned_path(&client)?)
            .await
            .map_err(failed)?;
        owned_path(NO_OBJECT)
    }

    async fn search_items(
        &self,
        attributes: HashMap<String, String>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<Vec<OwnedObjectPath>, SecretError> {
        let (collection, found) = {
            let shared = lock(&self.state);
            let keyring = shared.open()?;
            let collection = self.id_in(keyring)?;
            let pairs: Vec<(&str, &str)> = attributes
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str()))
                .collect();
            let mine = keyring.items_in(collection)?;
            let found: Vec<ItemId> = keyring
                .search_items(&pairs)?
                .into_iter()
                .filter(|id| mine.contains(id))
                .collect();
            (collection, found)
        };
        let mut paths = Vec::new();
        for id in found {
            let path = item_path(collection, id);
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
        properties: HashMap<String, OwnedVariant>,
        secret: WireSecret,
        replace: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<(OwnedObjectPath, OwnedObjectPath), SecretError> {
        let (label, attributes) = item_properties(properties)?;
        if lock(&self.state).keyring.is_none() {
            let prompt =
                defer_create_item(&self.state, server, self.target, label, attributes, &secret, replace).await?;
            return Ok((owned_path(NO_OBJECT)?, owned_path(&prompt)?));
        }
        let (collection, id, created) = {
            let mut shared = lock(&self.state);
            let (plain, content_type) = plain_secret(&shared, &secret)?;
            let keyring = shared.open_mut()?;
            let collection = self.id_in(keyring)?;
            let (id, created) = write_item(keyring, collection, &label, &content_type, &plain, &attributes, replace)?;
            (collection, id, created)
        };
        let path = item_path(collection, id);
        let item = owned_path(&path)?;
        if created {
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
            CollectionObject::item_created(&emitter, &item).await.map_err(failed)?;
        } else {
            CollectionObject::item_changed(&emitter, &item).await.map_err(failed)?;
        }
        Ok((item, owned_path(NO_OBJECT)?))
    }

    /// Sent on this collection after `CreateItem` adds an item.
    #[zbus(signal)]
    async fn item_created(emitter: &SignalEmitter<'_>, item: &OwnedObjectPath) -> zbus::Result<()>;

    /// Sent on this collection after `CreateItem` or `SetSecret` changes an item.
    #[zbus(signal)]
    async fn item_changed(emitter: &SignalEmitter<'_>, item: &OwnedObjectPath) -> zbus::Result<()>;

    /// Sent on this collection after an item is deleted.
    #[zbus(signal)]
    async fn item_deleted(emitter: &SignalEmitter<'_>, item: &OwnedObjectPath) -> zbus::Result<()>;

    #[zbus(property)]
    async fn items(&self) -> Result<Vec<OwnedObjectPath>, SecretError> {
        let shared = lock(&self.state);
        let Some(keyring) = shared.keyring.as_ref() else {
            return locked_read(self.target, Vec::new());
        };
        let collection = self.id_in(keyring)?;
        keyring
            .items_in(collection)?
            .into_iter()
            .map(|id| owned_path(&item_path(collection, id)))
            .collect()
    }

    #[zbus(property)]
    async fn label(&self) -> Result<String, SecretError> {
        let shared = lock(&self.state);
        let Some(keyring) = shared.keyring.as_ref() else {
            return locked_read(self.target, DEFAULT_LABEL.to_owned());
        };
        let id = self.id_in(keyring)?;
        let client = client_path(keyring, id)?;
        keyring
            .collections()?
            .into_iter()
            .find(|(known, _)| *known == id)
            .map(|(_, label)| label)
            .ok_or_else(|| SecretError::NoSuchObject(client))
    }

    /// Renames the collection. A locked keyring answers `IsLocked`.
    #[zbus(property)]
    async fn set_label(&self, label: String) -> Result<(), SecretError> {
        let mut shared = lock(&self.state);
        let keyring = shared.open_mut()?;
        let id = self.id_in(keyring)?;
        if keyring.rename_collection(id, &label)? {
            Ok(())
        } else {
            Err(SecretError::NoSuchObject(client_path(keyring, id)?))
        }
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
    /// Deletes the item and tells its collection, so clients drop it from their item lists.
    async fn delete(&self, #[zbus(object_server)] server: &ObjectServer) -> Result<OwnedObjectPath, SecretError> {
        let (owner, path) = {
            let mut shared = lock(&self.state);
            let keyring = shared.open_mut()?;
            let item = keyring
                .item(self.id)?
                .ok_or_else(|| SecretError::NoSuchObject(format!("item {}", self.id.raw())))?;
            keyring.delete_item(self.id)?;
            let owner = client_path(keyring, item.collection)?;
            (owner, item_path(item.collection, self.id))
        };
        server.remove::<ItemObject, _>(path.as_str()).await.map_err(failed)?;
        let owner = server
            .interface::<_, CollectionObject>(owner.as_str())
            .await
            .map_err(failed)?;
        CollectionObject::item_deleted(owner.signal_emitter(), &owned_path(&path)?)
            .await
            .map_err(failed)?;
        owned_path(NO_OBJECT)
    }

    async fn get_secret(&self, session: OwnedObjectPath) -> Result<WireSecret, SecretError> {
        let path = item_path_of(self.state.clone(), self.id)?;
        encrypt_item(&self.state, &session, &path)?.ok_or_else(|| SecretError::NoSuchObject(path))
    }

    /// Replaces the secret, keeping the label and attributes. The new secret arrives over the
    /// session the client opened, like `GetSecret` answers over it.
    async fn set_secret(
        &self,
        secret: WireSecret,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<(), SecretError> {
        let (owner, path) = {
            let mut shared = lock(&self.state);
            let (plain, content_type) = plain_secret(&shared, &secret)?;
            let keyring = shared.open_mut()?;
            let item = keyring
                .item(self.id)?
                .ok_or_else(|| SecretError::NoSuchObject(format!("item {}", self.id.raw())))?;
            let pairs: Vec<(&str, &str)> = item
                .attributes
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str()))
                .collect();
            keyring.update_item(self.id, &item.label, &content_type, &plain, &pairs)?;
            let owner = client_path(keyring, item.collection)?;
            (owner, item_path(item.collection, self.id))
        };
        let path = owned_path(&path)?;
        let owner = server
            .interface::<_, CollectionObject>(owner.as_str())
            .await
            .map_err(failed)?;
        CollectionObject::item_changed(owner.signal_emitter(), &path)
            .await
            .map_err(failed)
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

    /// Replaces the attributes, sorted by name as `CreateItem` stores them. The label and secret
    /// are kept.
    #[zbus(property)]
    async fn set_attributes(&self, attributes: HashMap<String, String>) -> Result<(), SecretError> {
        let mut pairs: Vec<(String, String)> = attributes.into_iter().collect();
        pairs.sort();
        self.rewrite(None, Some(pairs))
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

    /// Renames the item. The secret and attributes are kept.
    #[zbus(property)]
    async fn set_label(&self, label: String) -> Result<(), SecretError> {
        self.rewrite(Some(label), None)
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

impl ItemObject {
    /// Rewrites the label and/or attributes, keeping the secret and its content type, which are
    /// read back from the store, so a property write never touches the secret. A locked store
    /// answers `IsLocked`; a missing item, `NoSuchObject`.
    fn rewrite(&self, label: Option<String>, attributes: Option<Vec<(String, String)>>) -> Result<(), SecretError> {
        let mut shared = lock(&self.state);
        let keyring = shared.open_mut()?;
        let item = keyring
            .item(self.id)?
            .ok_or_else(|| SecretError::NoSuchObject(format!("item {}", self.id.raw())))?;
        let label = label.unwrap_or_else(|| item.label.clone());
        let attributes = attributes.unwrap_or_else(|| item.attributes.clone());
        let pairs: Vec<(&str, &str)> = attributes
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        keyring.update_item(self.id, &label, &item.content_type, &item.secret, &pairs)?;
        Ok(())
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

/// A password prompt the keyring raised. `Prompt` offers the request to the shell that claims
/// `prompt.secret`, and the answer reaches the prompt through [`crate::PromptAnswers`], which
/// emits `Completed` and removes this object. `Dismiss`, or an offer nobody can draw, finishes
/// it at once as dismissed. Each finished prompt removes its own object, so repeated `Unlock`
/// calls do not leak objects.
pub struct PromptObject {
    pub path: String,
    pub source: Arc<Mutex<UnlockKeySource>>,
    pub state: State,
}

#[zbus::interface(name = "org.freedesktop.Secret.Prompt")]
impl PromptObject {
    async fn prompt(
        &self,
        _window_id: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<(), SecretError> {
        let offered = lock_source(&self.source).offer_password(&self.path);
        match offered {
            Ok(()) => Ok(()),
            Err(reason) => {
                log::info!(%reason, "the password prompt could not be offered");
                drop_pending(&self.state, &self.path);
                finish_prompt(&self.path, &emitter, server, true).await
            }
        }
    }

    async fn dismiss(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> Result<(), SecretError> {
        lock_source(&self.source).dismiss_password(&self.path);
        drop_pending(&self.state, &self.path);
        finish_prompt(&self.path, &emitter, server, true).await
    }

    #[zbus(signal)]
    async fn completed(emitter: &SignalEmitter<'_>, dismissed: bool, result: Variant<'_>) -> zbus::Result<()>;
}

fn lock_source(source: &Mutex<UnlockKeySource>) -> MutexGuard<'_, UnlockKeySource> {
    // A panic in an earlier unlock poisons this lock. It must not stop every later prompt, so the
    // poisoned guard is taken over.
    source.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Emits `Completed` on the prompt at `path` from inside its own method, then removes it. That
/// method already holds the emitter, so the object is not looked up again.
async fn finish_prompt(
    path: &str,
    emitter: &SignalEmitter<'_>,
    server: &ObjectServer,
    dismissed: bool,
) -> Result<(), SecretError> {
    PromptObject::completed(emitter, dismissed, Variant::from("")).await?;
    server.remove::<PromptObject, _>(path).await.map_err(failed)?;
    Ok(())
}

/// Emits `Completed` on the prompt at `path` from outside its method, then removes it. On an
/// undismissed prompt it first applies the writes that waited on it, and `Completed` carries the
/// first item they made. `Completed` never carries the password. A dismissed prompt, or a write
/// that cannot be applied, drops the waiting writes and reports dismissed.
pub(crate) async fn emit_completed(server: &ObjectServer, path: &str, dismissed: bool) -> Result<(), SecretError> {
    let state = {
        let prompt = server.interface::<_, PromptObject>(path).await.map_err(failed)?;
        let prompt = prompt.get().await;
        prompt.state.clone()
    };
    let (dismissed, result) = if dismissed {
        drop_pending(&state, path);
        (true, None)
    } else {
        match apply_pending(server, &state, path).await {
            Ok(result) => (false, result),
            Err(error) => {
                log::warn!(?error, "a write that waited on the unlock could not be applied");
                (true, None)
            }
        }
    };
    {
        let prompt = server.interface::<_, PromptObject>(path).await.map_err(failed)?;
        let result = match result {
            Some(item) => Variant::from(item),
            None => Variant::from(""),
        };
        PromptObject::completed(prompt.signal_emitter(), dismissed, result).await?;
    }
    server.remove::<PromptObject, _>(path).await.map_err(failed)?;
    Ok(())
}

/// Remember the unlock source, so a collection can raise an unlock prompt for a locked write.
pub(crate) fn install_source(state: &State, source: &Arc<Mutex<UnlockKeySource>>) {
    lock(state).source = Some(Arc::downgrade(source));
}

/// Create and register a prompt object that offers the unlock when its caller calls `Prompt`.
async fn new_prompt(
    state: &State,
    source: Arc<Mutex<UnlockKeySource>>,
    server: &ObjectServer,
) -> Result<String, SecretError> {
    let id = lock(state).fresh_id();
    let path = format!("{PROMPT_PREFIX}{id}");
    let prompt = PromptObject {
        path: path.clone(),
        source,
        state: state.clone(),
    };
    server.at(path.as_str(), prompt).await.map_err(failed)?;
    Ok(path)
}

/// `CreateItem` on a locked store. The secret is decrypted now and held until the user unlocks;
/// the call answers `/` for the item and a prompt for the unlock, as the spec says.
async fn defer_create_item(
    state: &State,
    server: &ObjectServer,
    target: CollectionTarget,
    label: String,
    attributes: Vec<(String, String)>,
    secret: &WireSecret,
    replace: bool,
) -> Result<String, SecretError> {
    let source = lock(state)
        .source
        .as_ref()
        .and_then(Weak::upgrade)
        .ok_or_else(|| SecretError::Failed("the unlock is not available".to_owned()))?;
    let path = new_prompt(state, source, server).await?;
    if let Err(error) = queue_create_item(state, &path, target, label, attributes, secret, replace) {
        server.remove::<PromptObject, _>(path.as_str()).await.map_err(failed)?;
        return Err(error);
    }
    Ok(path)
}

/// Decrypt `secret` and park the write under `prompt`. Nothing is written to the store.
fn queue_create_item(
    state: &State,
    prompt: &str,
    target: CollectionTarget,
    label: String,
    attributes: Vec<(String, String)>,
    secret: &WireSecret,
    replace: bool,
) -> Result<(), SecretError> {
    let mut shared = lock(state);
    let (secret, content_type) = decrypt_wire(&shared, secret)?;
    shared
        .pending
        .entry(prompt.to_owned())
        .or_default()
        .push(PendingWrite::CreateItem {
            target,
            label,
            content_type,
            secret,
            attributes,
            replace,
        });
    Ok(())
}

/// Forget the writes that waited on `prompt`. Their secret buffers are zeroed as they drop.
fn drop_pending(state: &State, prompt: &str) {
    lock(state).pending.remove(prompt);
}

/// Write the items that waited on `prompt` into the now-open store. The waiting writes are taken
/// out first, so a second call finds nothing and a write is never applied twice. A locked store
/// answers `IsLocked` and the writes are gone.
fn apply_pending_state(state: &State, prompt: &str) -> Result<Vec<AppliedItem>, SecretError> {
    let mut shared = lock(state);
    let writes = shared.pending.remove(prompt).unwrap_or_default();
    let mut applied = Vec::new();
    for write in writes {
        let PendingWrite::CreateItem {
            target,
            label,
            content_type,
            secret,
            attributes,
            replace,
        } = write;
        let keyring = shared.open_mut()?;
        let collection = match target {
            CollectionTarget::Id(id) => id,
            CollectionTarget::Default => default_collection(keyring)?,
        };
        let (item, created) = write_item(
            keyring,
            collection,
            &label,
            &content_type,
            &secret,
            &attributes,
            replace,
        )?;
        applied.push(AppliedItem {
            collection,
            client: client_path(keyring, collection)?,
            item,
            created,
        });
    }
    Ok(applied)
}

/// Apply the writes that waited on `prompt`, register the objects they made and tell their
/// collections. Returns the path of the first item, for the prompt's `Completed` result.
async fn apply_pending(
    server: &ObjectServer,
    state: &State,
    prompt: &str,
) -> Result<Option<OwnedObjectPath>, SecretError> {
    let mut first = None;
    for applied in apply_pending_state(state, prompt)? {
        let path = item_path(applied.collection, applied.item);
        let item = owned_path(&path)?;
        let collection = server
            .interface::<_, CollectionObject>(applied.client.as_str())
            .await
            .map_err(failed)?;
        if applied.created {
            server
                .at(
                    path.as_str(),
                    ItemObject {
                        state: state.clone(),
                        id: applied.item,
                    },
                )
                .await
                .map_err(failed)?;
            CollectionObject::item_created(collection.signal_emitter(), &item)
                .await
                .map_err(failed)?;
        } else {
            CollectionObject::item_changed(collection.signal_emitter(), &item)
                .await
                .map_err(failed)?;
        }
        first.get_or_insert(item);
    }
    Ok(first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_keyring_store::Passphrase;

    fn scratch(label: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        std::env::temp_dir().join(format!("gnoblin-objects-{label}-{}-{nanos}.db", std::process::id()))
    }

    fn open_store(label: &str) -> (std::path::PathBuf, Keyring) {
        let path = scratch(label);
        let pass = Passphrase::new("correct horse".to_owned()).expect("a passphrase");
        let keyring = Keyring::create(&path, &pass).expect("create the store");
        (path, keyring)
    }

    fn session_path(id: u64) -> OwnedObjectPath {
        owned_path(&format!("{SESSION_PREFIX}{id}")).expect("a session path")
    }

    /// A state with an open store and session `id` holding the server half of a DH pair. The
    /// client half is returned, to encrypt what a client would send.
    fn state_with_session(keyring: Keyring, id: u64) -> (State, DhSessionKey) {
        let (server, client) = dh::session_pair();
        let state = new_state(Some(keyring));
        lock(&state).sessions.insert(id, server);
        (state, client)
    }

    fn attributes(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn a_locked_store_refuses_a_secret_before_anything_else() {
        let state = new_state(None);
        let wire: WireSecret = (session_path(1), vec![0; 16], vec![0; 16], "text/plain".to_owned());
        let shared = lock(&state);
        assert!(matches!(plain_secret(&shared, &wire), Err(SecretError::IsLocked(_))));
    }

    #[test]
    fn a_secret_under_an_unknown_session_is_refused() {
        let (path, keyring) = open_store("no-session");
        let state = new_state(Some(keyring));
        let wire: WireSecret = (session_path(9), vec![0; 16], vec![0; 16], "text/plain".to_owned());
        let shared = lock(&state);
        assert!(matches!(plain_secret(&shared, &wire), Err(SecretError::NoSession(_))));
        drop(shared);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_ciphertext_that_is_not_whole_blocks_is_an_argument_error() {
        let (path, keyring) = open_store("bad-cipher");
        let (state, client) = state_with_session(keyring, 1);
        let (iv, _) = dh::encrypt(&client, b"hunter2").expect("encrypt");
        let wire: WireSecret = (session_path(1), iv, vec![7; 15], "text/plain".to_owned());
        let shared = lock(&state);
        assert!(matches!(plain_secret(&shared, &wire), Err(SecretError::InvalidArgs(_))));
        drop(shared);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_secret_under_its_session_decrypts_with_its_content_type() {
        let (path, keyring) = open_store("round-trip");
        let (state, client) = state_with_session(keyring, 1);
        let (iv, cipher) = dh::encrypt(&client, b"hunter2").expect("encrypt");
        let wire: WireSecret = (session_path(1), iv, cipher, "text/plain".to_owned());
        let shared = lock(&state);
        let (plain, content_type) = plain_secret(&shared, &wire).expect("decrypts");
        assert_eq!(*plain, b"hunter2".to_vec());
        assert_eq!(content_type, "text/plain");
        drop(shared);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn create_item_without_a_label_is_an_argument_error() {
        let refusal = item_properties(HashMap::new()).expect_err("no label");
        assert!(matches!(refusal, SecretError::InvalidArgs(_)));
    }

    #[test]
    fn create_item_reads_the_label_and_sorts_the_attributes() {
        let properties = HashMap::from([
            (
                ITEM_LABEL.to_owned(),
                OwnedVariant::try_from(Variant::from("mail")).expect("a string"),
            ),
            (
                ITEM_ATTRIBUTES.to_owned(),
                OwnedVariant::from(HashMap::from([
                    ("b".to_owned(), "2".to_owned()),
                    ("a".to_owned(), "1".to_owned()),
                ])),
            ),
        ]);
        let (label, attrs) = item_properties(properties).expect("reads");
        assert_eq!(label, "mail");
        assert_eq!(attrs, attributes(&[("a", "1"), ("b", "2")]));
    }

    #[test]
    fn replace_updates_the_item_with_exactly_the_same_attributes() {
        let (path, mut keyring) = open_store("replace-exact");
        let collection = keyring.create_collection("Login").expect("collection");
        let (first, created) = write_item(
            &mut keyring,
            collection,
            "one",
            "text/plain",
            b"v1",
            &attributes(&[("app", "mail")]),
            true,
        )
        .expect("write");
        assert!(created);
        let (again, created) = write_item(
            &mut keyring,
            collection,
            "one",
            "text/plain",
            b"v2",
            &attributes(&[("app", "mail")]),
            true,
        )
        .expect("write");
        assert!(!created, "an exact match is updated, not added");
        assert_eq!(again, first);
        let stored = keyring.item(first).expect("read").expect("present");
        assert_eq!(*stored.secret, b"v2".to_vec());
        assert_eq!(stored.content_type, "text/plain");
        drop(keyring);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn replace_never_overwrites_an_item_with_extra_attributes() {
        let (path, mut keyring) = open_store("replace-superset");
        let collection = keyring.create_collection("Login").expect("collection");
        let (wide, _) = write_item(
            &mut keyring,
            collection,
            "wide",
            "text/plain",
            b"wide",
            &attributes(&[("app", "mail"), ("host", "x")]),
            true,
        )
        .expect("write");
        let (narrow, created) = write_item(
            &mut keyring,
            collection,
            "narrow",
            "text/plain",
            b"narrow",
            &attributes(&[("app", "mail")]),
            true,
        )
        .expect("write");
        assert!(created, "a superset match is not an exact match");
        assert_ne!(narrow, wide);
        assert_eq!(keyring.items_in(collection).expect("list").len(), 2);
        drop(keyring);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn without_replace_an_identical_item_is_added_alongside() {
        let (path, mut keyring) = open_store("no-replace");
        let collection = keyring.create_collection("Login").expect("collection");
        let (first, _) = write_item(
            &mut keyring,
            collection,
            "one",
            "text/plain",
            b"v1",
            &attributes(&[("app", "mail")]),
            false,
        )
        .expect("write");
        let (second, created) = write_item(
            &mut keyring,
            collection,
            "one",
            "text/plain",
            b"v2",
            &attributes(&[("app", "mail")]),
            false,
        )
        .expect("write");
        assert!(created);
        assert_ne!(second, first);
        drop(keyring);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn create_collection_without_a_label_is_an_argument_error() {
        let refusal = collection_label(HashMap::new()).expect_err("no label");
        assert!(matches!(refusal, SecretError::InvalidArgs(_)));
    }

    #[test]
    fn a_property_write_on_a_locked_store_is_refused() {
        let state = new_state(None);
        let item = ItemObject {
            state: state.clone(),
            id: ItemId::from_raw(1),
        };
        assert!(matches!(
            item.rewrite(Some("new".to_owned()), None),
            Err(SecretError::IsLocked(_))
        ));
    }

    #[test]
    fn relabelling_an_item_keeps_its_secret_and_attributes() {
        let (path, mut keyring) = open_store("relabel");
        let collection = keyring.create_collection("Login").expect("collection");
        let (id, _) = write_item(
            &mut keyring,
            collection,
            "old",
            "text/plain",
            b"v1",
            &attributes(&[("app", "mail")]),
            false,
        )
        .expect("write");
        let state = new_state(Some(keyring));
        ItemObject {
            state: state.clone(),
            id,
        }
        .rewrite(Some("new".to_owned()), None)
        .expect("relabels");
        let shared = lock(&state);
        let item = shared.open().expect("open").item(id).expect("reads").expect("present");
        assert_eq!(item.label, "new");
        assert_eq!(item.secret.as_slice(), b"v1");
        assert_eq!(item.attributes, attributes(&[("app", "mail")]));
        drop(shared);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn replacing_attributes_keeps_the_label_and_secret() {
        let (path, mut keyring) = open_store("reattr");
        let collection = keyring.create_collection("Login").expect("collection");
        let (id, _) = write_item(
            &mut keyring,
            collection,
            "keep",
            "text/plain",
            b"v1",
            &attributes(&[("app", "mail")]),
            false,
        )
        .expect("write");
        let state = new_state(Some(keyring));
        ItemObject {
            state: state.clone(),
            id,
        }
        .rewrite(None, Some(attributes(&[("zone", "9")])))
        .expect("reattributes");
        let shared = lock(&state);
        let item = shared.open().expect("open").item(id).expect("reads").expect("present");
        assert_eq!(item.label, "keep");
        assert_eq!(item.secret.as_slice(), b"v1");
        assert_eq!(item.attributes, attributes(&[("zone", "9")]));
        drop(shared);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_default_path_names_the_collection_the_default_alias_points_at() {
        let (path, keyring) = open_store("default-path");
        let login = keyring.create_collection("Login").expect("collection");
        let other = keyring.create_collection("Work").expect("collection");
        keyring.set_alias(DEFAULT_ALIAS, login).expect("alias");
        assert_eq!(client_path(&keyring, login).expect("path"), DEFAULT_COLLECTION_PATH);
        assert_eq!(client_path(&keyring, other).expect("path"), collection_path(other));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_default_object_with_no_default_alias_is_no_such_object() {
        let (path, keyring) = open_store("no-default");
        keyring.create_collection("Work").expect("collection");
        assert!(matches!(
            default_collection(&keyring),
            Err(SecretError::NoSuchObject(named)) if named == DEFAULT_COLLECTION_PATH
        ));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_default_path_follows_the_alias_when_it_moves() {
        let (path, keyring) = open_store("moved-default");
        let login = keyring.create_collection("Login").expect("collection");
        let work = keyring.create_collection("Work").expect("collection");
        keyring.set_alias(DEFAULT_ALIAS, login).expect("alias");
        keyring.set_alias(DEFAULT_ALIAS, work).expect("alias");
        assert_eq!(default_collection(&keyring).expect("default"), work);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn set_alias_accepts_the_default_path_and_still_refuses_other_objects() {
        let (path, keyring) = open_store("alias-default");
        let login = keyring.create_collection("Login").expect("collection");
        keyring.set_alias(DEFAULT_ALIAS, login).expect("alias");
        assert_eq!(
            alias_target(&keyring, DEFAULT_COLLECTION_PATH).expect("target"),
            Some(login)
        );
        assert_eq!(alias_target(&keyring, NO_OBJECT).expect("clear"), None);
        assert!(matches!(
            alias_target(&keyring, "/org/freedesktop/secrets/prompt/3"),
            Err(SecretError::InvalidArgs(_))
        ));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_locked_store_lists_only_the_default_path_and_names_it_for_the_default_alias() {
        let listed = collection_paths(None).expect("paths");
        assert_eq!(listed, vec![owned_path(DEFAULT_COLLECTION_PATH).expect("path")]);
        let named = read_alias_in(None, DEFAULT_ALIAS).expect("alias");
        assert_eq!(named.as_str(), DEFAULT_COLLECTION_PATH);
        let other = read_alias_in(None, "session").expect("alias");
        assert_eq!(other.as_str(), NO_OBJECT);
    }

    #[test]
    fn an_open_store_lists_and_names_the_default_collection_at_the_same_path() {
        let (path, keyring) = open_store("listed");
        let login = keyring.create_collection("Login").expect("collection");
        let work = keyring.create_collection("Work").expect("collection");
        keyring.set_alias(DEFAULT_ALIAS, login).expect("alias");
        let listed: Vec<String> = collection_paths(Some(&keyring))
            .expect("paths")
            .iter()
            .map(|p| p.as_str().to_owned())
            .collect();
        assert_eq!(listed, vec![DEFAULT_COLLECTION_PATH.to_owned(), collection_path(work)]);
        let named = read_alias_in(Some(&keyring), DEFAULT_ALIAS).expect("alias");
        assert_eq!(named.as_str(), DEFAULT_COLLECTION_PATH);
        let _ = std::fs::remove_file(&path);
    }

    /// A locked state with session `id`, and the client half of its DH pair.
    fn locked_state_with_session(id: u64) -> (State, DhSessionKey) {
        let (server, client) = dh::session_pair();
        let state = new_state(None);
        lock(&state).sessions.insert(id, server);
        (state, client)
    }

    fn wire(client: &DhSessionKey, session: u64, plain: &[u8]) -> WireSecret {
        let (iv, cipher) = dh::encrypt(client, plain).expect("encrypt");
        (session_path(session), iv, cipher, "text/plain".to_owned())
    }

    /// Unlock `state` with a fresh store that holds the default collection, as the unlock path does.
    fn unlock_into(state: &State, label: &str) -> std::path::PathBuf {
        let (path, keyring) = open_store(label);
        let collection = keyring.create_collection("Login").expect("collection");
        keyring.set_alias(DEFAULT_ALIAS, collection).expect("alias");
        install_keyring(state, Some(keyring));
        path
    }

    fn queue(
        state: &State,
        prompt: &str,
        client: &DhSessionKey,
        plain: &[u8],
        replace: bool,
    ) -> Result<(), SecretError> {
        queue_create_item(
            state,
            prompt,
            CollectionTarget::Default,
            "mail".to_owned(),
            attributes(&[("app", "mail")]),
            &wire(client, 1, plain),
            replace,
        )
    }

    #[test]
    fn a_deferred_write_stores_nothing_until_it_is_applied() {
        let (state, client) = locked_state_with_session(1);
        queue(&state, "/prompt/1", &client, b"hunter2", false).expect("queued while locked");
        assert_eq!(lock(&state).pending.get("/prompt/1").map(Vec::len), Some(1));
        let path = unlock_into(&state, "defer-wait");
        {
            let shared = lock(&state);
            let keyring = shared.open().expect("open");
            let collection = default_collection(keyring).expect("default");
            assert!(
                keyring.items_in(collection).expect("list").is_empty(),
                "nothing written yet"
            );
        }
        let applied = apply_pending_state(&state, "/prompt/1").expect("applied");
        assert_eq!(applied.len(), 1);
        assert!(applied[0].created);
        let shared = lock(&state);
        let stored = shared
            .open()
            .expect("open")
            .item(applied[0].item)
            .expect("read")
            .expect("present");
        assert_eq!(*stored.secret, b"hunter2".to_vec());
        assert_eq!(stored.content_type, "text/plain");
        assert_eq!(stored.attributes, attributes(&[("app", "mail")]));
        drop(shared);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_dropped_write_is_never_applied() {
        let (state, client) = locked_state_with_session(1);
        queue(&state, "/prompt/1", &client, b"hunter2", false).expect("queued");
        drop_pending(&state, "/prompt/1");
        let path = unlock_into(&state, "defer-drop");
        assert!(
            apply_pending_state(&state, "/prompt/1")
                .expect("nothing to apply")
                .is_empty()
        );
        let shared = lock(&state);
        let keyring = shared.open().expect("open");
        let collection = default_collection(keyring).expect("default");
        assert!(keyring.items_in(collection).expect("list").is_empty());
        drop(shared);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_deferred_write_is_applied_exactly_once() {
        let (state, client) = locked_state_with_session(1);
        queue(&state, "/prompt/1", &client, b"hunter2", false).expect("queued");
        let path = unlock_into(&state, "defer-once");
        assert_eq!(apply_pending_state(&state, "/prompt/1").expect("first").len(), 1);
        assert!(apply_pending_state(&state, "/prompt/1").expect("second").is_empty());
        let shared = lock(&state);
        let keyring = shared.open().expect("open");
        let collection = default_collection(keyring).expect("default");
        assert_eq!(keyring.items_in(collection).expect("list").len(), 1);
        drop(shared);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn writes_under_two_prompts_apply_independently() {
        let (state, client) = locked_state_with_session(1);
        queue(&state, "/prompt/1", &client, b"one", false).expect("queued");
        queue(&state, "/prompt/2", &client, b"two", false).expect("queued");
        let path = unlock_into(&state, "defer-two");
        assert_eq!(apply_pending_state(&state, "/prompt/1").expect("first").len(), 1);
        assert!(
            lock(&state).pending.contains_key("/prompt/2"),
            "the other write still waits"
        );
        assert_eq!(apply_pending_state(&state, "/prompt/2").expect("second").len(), 1);
        let shared = lock(&state);
        let keyring = shared.open().expect("open");
        let collection = default_collection(keyring).expect("default");
        assert_eq!(keyring.items_in(collection).expect("list").len(), 2);
        drop(shared);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_deferred_replace_updates_the_item_with_the_same_attributes() {
        let (state, client) = locked_state_with_session(1);
        let path = unlock_into(&state, "defer-replace");
        let first = {
            let mut shared = lock(&state);
            let keyring = shared.open_mut().expect("open");
            let collection = default_collection(keyring).expect("default");
            write_item(
                keyring,
                collection,
                "mail",
                "text/plain",
                b"old",
                &attributes(&[("app", "mail")]),
                false,
            )
            .expect("existing item")
            .0
        };
        queue(&state, "/prompt/1", &client, b"new", true).expect("queued");
        let applied = apply_pending_state(&state, "/prompt/1").expect("applied");
        assert_eq!(applied.len(), 1);
        assert!(!applied[0].created, "an exact match is updated, not added");
        assert_eq!(applied[0].item, first);
        let shared = lock(&state);
        let stored = shared
            .open()
            .expect("open")
            .item(first)
            .expect("read")
            .expect("present");
        assert_eq!(*stored.secret, b"new".to_vec());
        drop(shared);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn applying_to_a_locked_store_is_refused_and_the_write_is_forgotten() {
        let (state, client) = locked_state_with_session(1);
        queue(&state, "/prompt/1", &client, b"hunter2", false).expect("queued");
        let refusal = apply_pending_state(&state, "/prompt/1").expect_err("still locked");
        assert!(matches!(refusal, SecretError::IsLocked(_)));
        assert!(!lock(&state).pending.contains_key("/prompt/1"), "the write is gone");
    }

    #[test]
    fn a_deferred_write_under_an_unknown_session_is_refused_and_queues_nothing() {
        let (state, client) = locked_state_with_session(1);
        let refusal = queue_create_item(
            &state,
            "/prompt/1",
            CollectionTarget::Default,
            "mail".to_owned(),
            Vec::new(),
            &wire(&client, 9, b"hunter2"),
            false,
        )
        .expect_err("no such session");
        assert!(matches!(refusal, SecretError::NoSession(_)));
        assert!(lock(&state).pending.is_empty());
    }
}
