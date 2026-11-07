//! The schema of a namespace file, and the migrations that move it between versions.
//!
//! The version lives in SQLite's `PRAGMA user_version`, which is part of the file, so a
//! migration and the version it sets commit in one transaction: a migration that fails leaves
//! the file at the version it had. The store owner writes the steps here and nothing else
//! changes the schema.

use rusqlite::Connection;

use crate::{StoreError, kv};

/// The migrations, in order. Step `n` (counting from 1) moves a file from version `n - 1` to
/// version `n`. A step is never edited once a release has shipped it; a change is a new step.
pub(crate) const STEPS: &[&str] = &[
    // Version 1: the items, their full-text index for contains-text, and the counters.
    //
    // `number` has no declared type so SQLite keeps the class of what was stored: an integer
    // stays an integer and a double stays a double, and a number range compares them
    // numerically. `text` is NULL for every value that is not text.
    "CREATE TABLE items (
        key TEXT PRIMARY KEY NOT NULL,
        rev INTEGER NOT NULL,
        updated_ms INTEGER NOT NULL,
        value BLOB NOT NULL,
        number,
        text TEXT
    );
    CREATE INDEX items_updated ON items (updated_ms, key);
    CREATE VIRTUAL TABLE items_fts USING fts5 (text, tokenize = 'trigram');
    CREATE TABLE meta (name TEXT PRIMARY KEY NOT NULL, value INTEGER NOT NULL);
    INSERT INTO meta (name, value) VALUES ('revision', 0), ('bytes', 0);",
];

/// Brings `conn` up to the latest version of `steps`, one transaction per step.
///
/// A file already past the last step was written by a newer store and is refused rather than
/// read with an older schema. A failed step is reported with its version and the file keeps the
/// version of the last step that committed.
pub(crate) fn migrate(conn: &mut Connection, steps: &[&str]) -> Result<(), StoreError> {
    let current: u32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(kv::map_sqlite)?;
    if current as usize > steps.len() {
        return Err(StoreError::Migration {
            version: current,
            message: "the file was written by a newer store".to_owned(),
        });
    }
    for (index, step) in steps.iter().enumerate().skip(current as usize) {
        let version = index as u32 + 1;
        let transaction = conn.transaction().map_err(kv::map_sqlite)?;
        transaction.execute_batch(step).map_err(|error| StoreError::Migration {
            version,
            message: error.to_string(),
        })?;
        transaction
            .pragma_update(None, "user_version", version)
            .map_err(kv::map_sqlite)?;
        transaction.commit().map_err(kv::map_sqlite)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_step_leaves_the_committed_version_and_a_later_run_resumes() {
        let mut conn = Connection::open_in_memory().expect("opens");
        let broken = ["CREATE TABLE first (a INTEGER);", "CREATE TABLE second (; BROKEN"];

        let error = migrate(&mut conn, &broken).expect_err("the second step fails");
        assert!(matches!(error, StoreError::Migration { version: 2, .. }), "{error:?}");

        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("reads");
        assert_eq!(version, 1, "the first step committed, the second rolled back");

        let fixed = ["CREATE TABLE first (a INTEGER);", "CREATE TABLE second (a INTEGER);"];
        migrate(&mut conn, &fixed).expect("resumes at version 2");
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("reads");
        assert_eq!(version, 2);
    }

    #[test]
    fn a_file_from_a_newer_store_is_refused() {
        let mut conn = Connection::open_in_memory().expect("opens");
        conn.pragma_update(None, "user_version", 9).expect("sets");
        assert!(matches!(
            migrate(&mut conn, STEPS),
            Err(StoreError::Migration { version: 9, .. })
        ));
    }
}
