//! The schema of the keyring file. The version lives in `PRAGMA user_version`, which is inside the
//! encrypted file, so a file's version cannot be read or changed without the key.

use rusqlite::Connection;

use crate::KeyringError;

/// The schema version this crate writes and reads. A file with any other version is refused
/// rather than read with the wrong layout.
pub(crate) const VERSION: u32 = 1;

/// Version 1. Secrets are stored as BLOBs: the whole file is encrypted, so a secret needs no
/// second layer. Deleting a row is made to overwrite its bytes by `PRAGMA secure_delete`, which
/// the connection sets before any write.
const V1: &str = "
    CREATE TABLE meta (
        name TEXT PRIMARY KEY NOT NULL,
        value TEXT NOT NULL
    );
    CREATE TABLE collections (
        id INTEGER PRIMARY KEY,
        label TEXT NOT NULL
    );
    CREATE TABLE aliases (
        name TEXT PRIMARY KEY NOT NULL,
        collection_id INTEGER NOT NULL REFERENCES collections (id) ON DELETE CASCADE
    );
    CREATE TABLE items (
        id INTEGER PRIMARY KEY,
        collection_id INTEGER NOT NULL REFERENCES collections (id) ON DELETE CASCADE,
        label TEXT NOT NULL,
        secret BLOB NOT NULL,
        created_ms INTEGER NOT NULL,
        modified_ms INTEGER NOT NULL
    );
    CREATE TABLE attributes (
        item_id INTEGER NOT NULL REFERENCES items (id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        value TEXT NOT NULL,
        PRIMARY KEY (item_id, name)
    );
    CREATE INDEX attributes_by_value ON attributes (name, value);
";

/// Writes the version-1 schema into an empty file, in one transaction with its version.
pub(crate) fn create(conn: &mut Connection) -> Result<(), KeyringError> {
    let transaction = conn.transaction()?;
    transaction.execute_batch(V1)?;
    transaction.pragma_update(None, "user_version", VERSION)?;
    transaction.commit()?;
    Ok(())
}

/// Refuses a file whose version this crate does not read. Version 0 is an empty or foreign file:
/// a keyring file always carries its schema.
pub(crate) fn check(conn: &Connection) -> Result<(), KeyringError> {
    let found: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if found == VERSION {
        return Ok(());
    }
    if found > VERSION {
        return Err(KeyringError::NewerSchema { found });
    }
    Err(KeyringError::Corrupt(format!(
        "schema version {found} is not a keyring file"
    )))
}
