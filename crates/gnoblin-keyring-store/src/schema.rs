//! The schema of the keyring file. The version lives in `PRAGMA user_version`, which is inside the
//! encrypted file, so a file's version cannot be read or changed without the key.

use rusqlite::Connection;

use crate::KeyringError;
use crate::log;

/// The schema version this crate writes. A version-1 file is upgraded on open; a newer file is
/// refused rather than read with the wrong layout.
pub(crate) const VERSION: u32 = 2;

/// Version 1. Secrets are stored as BLOBs: the whole file is encrypted, so a secret needs no
/// second layer. Deleting a row is made to overwrite its bytes by `PRAGMA secure_delete`, which
/// the connection sets before any write.
pub(crate) const V1: &str = "
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

/// The step from version 1 to version 2: each secret records its MIME type. Every version-1 item
/// was stored by a client that wrote plain text, so the existing rows take `text/plain`.
const V1_TO_V2: &str = "ALTER TABLE items ADD COLUMN content_type TEXT NOT NULL DEFAULT 'text/plain';";

/// Writes the current schema into an empty file, in one transaction with its version. A new file
/// takes the same steps an upgraded one does, so there is one definition of each version's layout.
pub(crate) fn create(conn: &mut Connection) -> Result<(), KeyringError> {
    let transaction = conn.transaction()?;
    transaction.execute_batch(V1)?;
    transaction.execute_batch(V1_TO_V2)?;
    transaction.pragma_update(None, "user_version", VERSION)?;
    transaction.commit()?;
    Ok(())
}

/// Brings the file to the current schema, upgrading a version-1 file in place, or refuses it.
/// Version 0 is an empty or foreign file: a keyring file always carries its schema.
pub(crate) fn check(conn: &mut Connection) -> Result<(), KeyringError> {
    let found: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    match found {
        VERSION => Ok(()),
        1 => upgrade_from_1(conn),
        _ if found > VERSION => Err(KeyringError::NewerSchema { found }),
        _ => Err(KeyringError::Corrupt(format!(
            "schema version {found} is not a keyring file"
        ))),
    }
}

/// Applies the version-1 to version-2 step and stamps the new version, in one transaction, so a
/// failure part way leaves the file at version 1 and the next open tries again.
fn upgrade_from_1(conn: &mut Connection) -> Result<(), KeyringError> {
    let transaction = conn.transaction()?;
    transaction.execute_batch(V1_TO_V2)?;
    transaction.pragma_update(None, "user_version", VERSION)?;
    transaction.commit()?;
    log::info!("upgraded the keyring file from schema 1 to schema {VERSION}");
    Ok(())
}
