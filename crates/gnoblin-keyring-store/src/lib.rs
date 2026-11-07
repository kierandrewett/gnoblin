//! The keyring's encrypted database: one SQLCipher file holding collections, items and their
//! attributes, opened with a passphrase.
//!
//! A [`Keyring`] is one connection to one file. Every page of the file is encrypted under a key
//! derived from the [`Passphrase`], so a copy of the file reveals nothing without it. The file is
//! not a service: the secret service, its prompts and its locking are the layers above this one.
//!
//! Two properties are deliberate. The key-derivation count is set on every open, because the key
//! is a function of it and a file cannot be opened with a different count. And a key that does not
//! open the file is reported as [`KeyringError::WrongKey`], because an encrypted header cannot be
//! told apart from a damaged one.

mod schema;

#[cfg(test)]
mod tests;

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, params, params_from_iter};
use zeroize::Zeroizing;

gnoblin_log::logger!(log = "keyring-store");

/// The PBKDF2 rounds SQLCipher runs on the passphrase. Set explicitly on every open rather than
/// trusting the library default: the key is a function of this count, so a silent change of
/// default would make every existing file unopenable.
const KDF_ITERATIONS: u32 = 256_000;

/// The passphrase the file is keyed with. Its bytes are wiped when it is dropped.
pub struct Passphrase(Zeroizing<String>);

impl Passphrase {
    /// Takes ownership of `text`. An empty passphrase is refused: it would key the file with
    /// nothing, and a file that any process can open is no keyring.
    pub fn new(text: String) -> Result<Passphrase, KeyringError> {
        if text.is_empty() {
            return Err(KeyringError::EmptyPassphrase);
        }
        Ok(Passphrase(Zeroizing::new(text)))
    }
}

/// A keyring file, open on one connection.
pub struct Keyring {
    conn: Connection,
}

/// The identity of a collection in one file. It is never reused within a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CollectionId(i64);

/// The identity of an item in one file. It is never reused within a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ItemId(i64);

impl CollectionId {
    /// The row number this id names in the file. Paths on the bus carry it, so the service
    /// reads it back when a caller names a collection.
    pub fn raw(self) -> i64 {
        self.0
    }

    /// The id for row `raw`. It names nothing until a lookup finds a collection with it.
    pub fn from_raw(raw: i64) -> CollectionId {
        CollectionId(raw)
    }
}

impl ItemId {
    /// The row number this id names in the file.
    pub fn raw(self) -> i64 {
        self.0
    }

    /// The id for row `raw`. It names nothing until a lookup finds an item with it.
    pub fn from_raw(raw: i64) -> ItemId {
        ItemId(raw)
    }
}

/// One stored item. The secret is wiped when the item is dropped.
pub struct Item {
    pub id: ItemId,
    pub collection: CollectionId,
    pub label: String,
    /// The MIME type the secret was written as, such as `text/plain`. The keyring stores it and
    /// does not interpret it, so a client reads the secret back with the type it wrote.
    pub content_type: String,
    pub secret: Zeroizing<Vec<u8>>,
    /// The attributes in name order, each a name and its value.
    pub attributes: Vec<(String, String)>,
    /// Creation time, in milliseconds since the Unix epoch.
    pub created_ms: i64,
    /// Last modification time, in milliseconds since the Unix epoch.
    pub modified_ms: i64,
}

/// Why a keyring operation failed.
#[derive(Debug)]
pub enum KeyringError {
    /// The passphrase was empty.
    EmptyPassphrase,
    /// No file exists at the path, so there is nothing to open.
    Missing(PathBuf),
    /// A file already exists at the path, so the keyring is not created over it.
    Exists(PathBuf),
    /// The passphrase does not open the file. An encrypted header cannot be told apart from a
    /// damaged one, so a file whose first page is damaged is reported the same way.
    WrongKey,
    /// The key opened the file, but its contents fail the integrity check, or the file is not a
    /// keyring file.
    Corrupt(String),
    /// The file was written by a newer schema than this crate reads.
    NewerSchema { found: u32 },
    /// Any other SQLite failure.
    Sqlite(rusqlite::Error),
    /// A file could not be created or inspected.
    Io(std::io::Error),
}

impl fmt::Display for KeyringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPassphrase => write!(f, "the passphrase is empty"),
            Self::Missing(path) => write!(f, "no keyring file at {}", path.display()),
            Self::Exists(path) => write!(f, "a keyring file already exists at {}", path.display()),
            Self::WrongKey => write!(f, "the passphrase does not open the keyring, or its header is damaged"),
            Self::Corrupt(detail) => write!(f, "the keyring file is damaged: {detail}"),
            Self::NewerSchema { found } => write!(
                f,
                "the keyring file uses schema {found}, but this build reads schema {}",
                schema::VERSION
            ),
            Self::Sqlite(error) => write!(f, "{error}"),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for KeyringError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for KeyringError {
    /// A failure after the key has opened the file is never the key's fault: a page that is
    /// damaged or a file that changed under us is corruption.
    fn from(error: rusqlite::Error) -> Self {
        if is_corruption(&error) {
            Self::Corrupt(error.to_string())
        } else {
            Self::Sqlite(error)
        }
    }
}

impl Keyring {
    /// Creates a new keyring file at `path`. The file must not exist: creating over one would
    /// silently replace another keyring.
    pub fn create(path: &Path, passphrase: &Passphrase) -> Result<Keyring, KeyringError> {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::AlreadyExists => KeyringError::Exists(path.to_owned()),
                _ => KeyringError::Io(error),
            })?;
        let mut conn = connect(path, passphrase, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        schema::create(&mut conn)?;
        log::info!("created a keyring file");
        Ok(Keyring { conn })
    }

    /// Opens the keyring file at `path` with `passphrase`, checking its integrity and schema
    /// before any caller reads from it.
    pub fn open(path: &Path, passphrase: &Passphrase) -> Result<Keyring, KeyringError> {
        match std::fs::metadata(path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(KeyringError::Missing(path.to_owned()));
            }
            Err(error) => return Err(KeyringError::Io(error)),
        }
        let mut conn = connect(path, passphrase, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        let verdict: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if verdict != "ok" {
            log::warn!("the keyring file failed its integrity check: {verdict}");
            return Err(KeyringError::Corrupt(verdict));
        }
        schema::check(&mut conn)?;
        Ok(Keyring { conn })
    }

    /// Re-keys the open file to `passphrase`. SQLCipher rewrites every page, so the file is
    /// unreadable with the old key once this returns, and a failure part way leaves the old key
    /// valid.
    pub fn rekey(&mut self, passphrase: &Passphrase) -> Result<(), KeyringError> {
        self.conn.pragma_update(None, "rekey", passphrase.0.as_str())?;
        log::info!("re-keyed the keyring file");
        Ok(())
    }

    /// Creates a collection with a human-readable `label`.
    pub fn create_collection(&self, label: &str) -> Result<CollectionId, KeyringError> {
        self.conn
            .execute("INSERT INTO collections (label) VALUES (?1)", params![label])?;
        Ok(CollectionId(self.conn.last_insert_rowid()))
    }

    /// Points the well-known `name` (such as `default`) at `collection`, replacing any earlier
    /// target.
    pub fn set_alias(&self, name: &str, collection: CollectionId) -> Result<(), KeyringError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO aliases (name, collection_id) VALUES (?1, ?2)",
            params![name, collection.0],
        )?;
        Ok(())
    }

    /// The collection `name` points at, if an alias with that name exists.
    pub fn alias(&self, name: &str) -> Result<Option<CollectionId>, KeyringError> {
        let id = self
            .conn
            .query_row(
                "SELECT collection_id FROM aliases WHERE name = ?1",
                params![name],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        Ok(id.map(CollectionId))
    }

    /// Stores a new item in `collection`. The secret and every attribute are written in one
    /// transaction, so an item is never seen without its attributes. Attribute names must be
    /// unique within the item. `content_type` is the MIME type the secret was written as.
    pub fn put_item(
        &mut self,
        collection: CollectionId,
        label: &str,
        content_type: &str,
        secret: &[u8],
        attributes: &[(&str, &str)],
    ) -> Result<ItemId, KeyringError> {
        let now = now_ms();
        let transaction = self.conn.transaction()?;
        transaction.execute(
            "INSERT INTO items (collection_id, label, content_type, secret, created_ms, modified_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![collection.0, label, content_type, secret, now],
        )?;
        let id = transaction.last_insert_rowid();
        for (name, value) in attributes {
            transaction.execute(
                "INSERT INTO attributes (item_id, name, value) VALUES (?1, ?2, ?3)",
                params![id, name, value],
            )?;
        }
        transaction.commit()?;
        Ok(ItemId(id))
    }

    /// The item with `id`, or `None` when it does not exist.
    pub fn item(&self, id: ItemId) -> Result<Option<Item>, KeyringError> {
        let row = self
            .conn
            .query_row(
                "SELECT collection_id, label, content_type, secret, created_ms, modified_ms FROM items WHERE id = ?1",
                params![id.0],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((collection, label, content_type, secret, created_ms, modified_ms)) = row else {
            return Ok(None);
        };
        let mut statement = self
            .conn
            .prepare("SELECT name, value FROM attributes WHERE item_id = ?1 ORDER BY name")?;
        let attributes = statement
            .query_map(params![id.0], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(Item {
            id,
            collection: CollectionId(collection),
            label,
            content_type,
            secret: Zeroizing::new(secret),
            attributes,
            created_ms,
            modified_ms,
        }))
    }

    /// The ids of every item that carries all of `attributes` with exactly those values, in id
    /// order. An empty filter matches every item.
    pub fn search_items(&self, attributes: &[(&str, &str)]) -> Result<Vec<ItemId>, KeyringError> {
        // The SQL is built from fixed fragments; every name and value is a bound parameter.
        let mut sql = String::from("SELECT id FROM items WHERE 1 = 1");
        for _ in attributes {
            sql.push_str(
                " AND EXISTS (SELECT 1 FROM attributes WHERE attributes.item_id = items.id \
                 AND attributes.name = ? AND attributes.value = ?)",
            );
        }
        sql.push_str(" ORDER BY id");
        let bound = attributes.iter().flat_map(|(name, value)| [*name, *value]);
        let mut statement = self.conn.prepare(&sql)?;
        let ids = statement
            .query_map(params_from_iter(bound), |row| row.get::<_, i64>(0).map(ItemId))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Every collection in the file, in id order, with its label.
    pub fn collections(&self) -> Result<Vec<(CollectionId, String)>, KeyringError> {
        let mut statement = self.conn.prepare("SELECT id, label FROM collections ORDER BY id")?;
        let collections = statement
            .query_map([], |row| {
                Ok((CollectionId(row.get::<_, i64>(0)?), row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(collections)
    }

    /// The ids of every item in `collection`, in id order.
    pub fn items_in(&self, collection: CollectionId) -> Result<Vec<ItemId>, KeyringError> {
        let mut statement = self
            .conn
            .prepare("SELECT id FROM items WHERE collection_id = ?1 ORDER BY id")?;
        let ids = statement
            .query_map(params![collection.0], |row| row.get::<_, i64>(0).map(ItemId))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Deletes the item with `id`, and its attributes. Returns whether an item was removed.
    pub fn delete_item(&mut self, id: ItemId) -> Result<bool, KeyringError> {
        let removed = self.conn.execute("DELETE FROM items WHERE id = ?1", params![id.0])?;
        Ok(removed > 0)
    }

    /// Replaces the label, content type, secret and attributes of the item `id` in one
    /// transaction, so a reader never sees the new secret with the old attributes. The item keeps
    /// its id and creation time. Returns `false`, and changes nothing, when no item has that id.
    pub fn update_item(
        &mut self,
        id: ItemId,
        label: &str,
        content_type: &str,
        secret: &[u8],
        attributes: &[(&str, &str)],
    ) -> Result<bool, KeyringError> {
        let now = now_ms();
        let transaction = self.conn.transaction()?;
        let updated = transaction.execute(
            "UPDATE items SET label = ?2, content_type = ?3, secret = ?4, modified_ms = ?5 WHERE id = ?1",
            params![id.0, label, content_type, secret, now],
        )?;
        if updated == 0 {
            return Ok(false);
        }
        transaction.execute("DELETE FROM attributes WHERE item_id = ?1", params![id.0])?;
        for (name, value) in attributes {
            transaction.execute(
                "INSERT INTO attributes (item_id, name, value) VALUES (?1, ?2, ?3)",
                params![id.0, name, value],
            )?;
        }
        transaction.commit()?;
        Ok(true)
    }

    /// Deletes the collection `id` with its items, their attributes and any alias that names it.
    /// The schema cascades those; `foreign_keys` is on for every connection (see `connect`).
    /// Returns whether a collection was removed.
    pub fn delete_collection(&mut self, id: CollectionId) -> Result<bool, KeyringError> {
        let removed = self
            .conn
            .execute("DELETE FROM collections WHERE id = ?1", params![id.0])?;
        Ok(removed > 0)
    }
}

/// Opens `path` and applies the key. The order matters: SQLCipher reads the key settings when
/// the key is applied, so they are set first, and the key is proved by reading the schema before
/// anything else touches the file.
fn connect(path: &Path, passphrase: &Passphrase, flags: OpenFlags) -> Result<Connection, KeyringError> {
    let conn = Connection::open_with_flags(path, flags)?;
    conn.pragma_update(None, "kdf_iter", KDF_ITERATIONS)?;
    conn.pragma_update(None, "key", passphrase.0.as_str())?;
    // Deleted secrets are overwritten rather than left in free pages.
    conn.pragma_update(None, "secure_delete", true)?;
    // Temporary tables and sort spills stay in memory, not in a plaintext temp file. SQLite's
    // value 2 is `MEMORY`.
    conn.pragma_update(None, "temp_store", 2)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
        .map_err(|error| {
            if is_not_a_database(&error) {
                log::warn!("the passphrase does not open the keyring file");
                KeyringError::WrongKey
            } else {
                KeyringError::from(error)
            }
        })?;
    Ok(conn)
}

/// Whether SQLite reports the file as not a database: the key is wrong, or the file is not a
/// keyring file at all.
fn is_not_a_database(error: &rusqlite::Error) -> bool {
    matches!(error, rusqlite::Error::SqliteFailure(code, _) if code.code == ErrorCode::NotADatabase)
}

/// Whether SQLite reports damage to pages it has already read through the key.
fn is_corruption(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(code, _)
            if matches!(code.code, ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase)
    )
}

/// Milliseconds since the Unix epoch, for the creation and modification times of an item.
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
