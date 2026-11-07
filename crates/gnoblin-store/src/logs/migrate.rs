//! The one-off import of the log file `gnoblin-log` wrote before the store took the log over.
//!
//! The old file is `logs.sqlite3` next to the store directory. The first writable open of the
//! store copies its rows into `log.sqlite3` in batches, then renames the old file to
//! `logs.sqlite3.migrated`. The old file is never deleted, and nothing is renamed until every row
//! has been copied.
//!
//! Progress lives in the `legacy_import` table of the new file, and each batch commits together
//! with its progress. So a crash, or a second process opening the same store, resumes after the
//! last committed batch and never copies a row twice. Old row ids are not kept: the new file
//! gives its own ids in the order the old rows were read.
//!
//! An import that cannot finish never stops the store from starting. It is logged as a warning
//! with a name from [`ImportError::code`], and the next writable open tries again.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior, params};

use super::INSERT_SQL;
use crate::log;

/// The file name the old `gnoblin-log` wrote, next to the store directory.
pub(super) const LEGACY_FILE: &str = "logs.sqlite3";

/// The suffix a legacy file gets once it is imported. A name already taken gets a number after it.
const MIGRATED_SUFFIX: &str = ".migrated";

/// The rows copied per transaction. A batch commits with its progress, so this bounds the work a
/// crash can repeat, which is none.
const BATCH_ROWS: usize = 1_000;

/// The columns the old table must have for its rows to be copied. `role` is optional: a file
/// written before that column existed gives each row its component as its role, as the old
/// `gnoblin-log` did when it added the column.
const REQUIRED_COLUMNS: &[&str] = &["id", "time_ms", "level", "component", "target", "message", "fields"];

/// The path of the legacy file for a store whose directory is `store_root`: its sibling
/// `logs.sqlite3`. The store directory is `$XDG_STATE_HOME/gnoblin/store`, so this is the file
/// the old default placed at `$XDG_STATE_HOME/gnoblin/logs.sqlite3`.
pub(crate) fn legacy_path(store_root: &Path) -> PathBuf {
    store_root.parent().unwrap_or(store_root).join(LEGACY_FILE)
}

/// Why an import did not finish. The store keeps running and the next writable open tries again.
#[derive(Debug)]
pub(crate) enum ImportError {
    /// The old file's `user_version` is not zero, which `gnoblin-log` never wrote, so a newer
    /// build wrote it. The file is left untouched.
    NewerSchema { version: i64 },
    /// The old table lacks a column the import needs. The file is left untouched.
    UnrecognisedSchema { missing: &'static str },
    /// The disk filled up. The batches already committed stay, and the next start resumes.
    DiskFull { imported: u64 },
    /// Any other SQLite failure. The batches already committed stay, and the next start resumes.
    Database { imported: u64, message: String },
    /// Every row was copied, but the old file could not be renamed. The next start renames it.
    Rename { message: String },
}

impl ImportError {
    /// The name the error is reported under. Each name is stable, so a log search can find it.
    pub(crate) fn code(&self) -> &'static str {
        match self {
            ImportError::NewerSchema { .. } => "newer-schema",
            ImportError::UnrecognisedSchema { .. } => "unrecognised-schema",
            ImportError::DiskFull { .. } => "disk-full",
            ImportError::Database { .. } => "database",
            ImportError::Rename { .. } => "rename-failed",
        }
    }

    /// Maps a SQLite failure to its name, keeping how many rows were committed before it.
    fn from_sqlite(imported: u64, error: rusqlite::Error) -> Self {
        match &error {
            rusqlite::Error::SqliteFailure(failure, _) if failure.code == ErrorCode::DiskFull => {
                ImportError::DiskFull { imported }
            }
            _ => ImportError::Database {
                imported,
                message: error.to_string(),
            },
        }
    }
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::NewerSchema { version } => {
                write!(
                    f,
                    "the old file has schema version {version}, which a newer build wrote"
                )
            }
            ImportError::UnrecognisedSchema { missing } => {
                write!(f, "the old table has no `{missing}` column")
            }
            ImportError::DiskFull { imported } => write!(
                f,
                "the disk is full after {imported} rows; the rest resume on the next start"
            ),
            ImportError::Database { imported, message } => {
                write!(f, "{message} after {imported} rows; the rest resume on the next start")
            }
            ImportError::Rename { message } => write!(f, "all rows were copied, but {message}"),
        }
    }
}

/// One row of the old table, as the import copies it.
struct LegacyRow {
    id: i64,
    time_ms: i64,
    level: String,
    component: String,
    role: String,
    target: String,
    message: String,
    fields: String,
}

/// Imports the legacy file at `legacy` into the store file behind `store`, then renames it.
///
/// Does nothing when there is no legacy file, and when the import has already finished and the
/// file has gone. A file that a finished import left behind is renamed again.
///
/// # Errors
///
/// The reason the import stopped. Rows committed before the failure stay in the store.
pub(crate) fn run(store: &mut Connection, legacy: &Path) -> Result<(), ImportError> {
    if !legacy.exists() {
        return Ok(());
    }
    let imported = {
        // A read-write open, so SQLite can finish a write-ahead log the old writer left. Nothing
        // is written to the rows themselves.
        let source = Connection::open_with_flags(legacy, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|error| ImportError::from_sqlite(0, error))?;
        copy_rows(store, &source)?
    };
    if imported > 0 {
        log::info!("imported {imported} log records from {}", legacy.display());
    }
    rename_migrated(legacy)
}

/// Copies every row of the legacy table into the store, one committed batch at a time. Returns
/// how many rows this call copied; zero when the import had finished already.
fn copy_rows(store: &mut Connection, source: &Connection) -> Result<u64, ImportError> {
    let version: i64 = source
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| ImportError::from_sqlite(0, error))?;
    if version != 0 {
        return Err(ImportError::NewerSchema { version });
    }
    let columns = table_columns(source)?;
    if let Some(missing) = REQUIRED_COLUMNS.iter().find(|name| !columns.iter().any(|c| c == *name)) {
        return Err(ImportError::UnrecognisedSchema { missing });
    }
    // A file without a role column keeps the role in the component column, so copy that.
    let role = if columns.iter().any(|name| name == "role") {
        "role"
    } else {
        "component"
    };
    let select = format!(
        "SELECT id, time_ms, level, component, {role}, target, message, fields \
         FROM log_records WHERE id > ?1 ORDER BY id LIMIT {BATCH_ROWS}"
    );

    let mut copied: u64 = 0;
    loop {
        let transaction = store
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| ImportError::from_sqlite(copied, error))?;
        let (after, done) = progress(&transaction).map_err(|error| ImportError::from_sqlite(copied, error))?;
        if done {
            return Ok(copied);
        }
        let rows = read_batch(source, &select, after).map_err(|error| ImportError::from_sqlite(copied, error))?;
        let finished = rows.len() < BATCH_ROWS;
        let last_id = rows.last().map_or(after, |row| row.id);
        {
            let mut insert = transaction
                .prepare_cached(INSERT_SQL)
                .map_err(|error| ImportError::from_sqlite(copied, error))?;
            for row in &rows {
                insert
                    .execute(params![
                        row.time_ms,
                        row.level,
                        row.component,
                        row.component,
                        row.role,
                        row.target,
                        row.message,
                        row.fields,
                    ])
                    .map_err(|error| ImportError::from_sqlite(copied, error))?;
            }
        }
        transaction
            .execute(
                "INSERT INTO legacy_import (singleton, last_id, done) VALUES (1, ?1, ?2)
                 ON CONFLICT (singleton) DO UPDATE SET last_id = excluded.last_id, done = excluded.done",
                params![last_id, i64::from(finished)],
            )
            .map_err(|error| ImportError::from_sqlite(copied, error))?;
        transaction
            .commit()
            .map_err(|error| ImportError::from_sqlite(copied, error))?;
        copied += rows.len() as u64;
        if finished {
            return Ok(copied);
        }
    }
}

/// The names of the columns of the legacy `log_records` table. An absent table has none, so the
/// caller reports the first required column as missing.
fn table_columns(source: &Connection) -> Result<Vec<String>, ImportError> {
    let mut statement = source
        .prepare("SELECT name FROM pragma_table_info('log_records')")
        .map_err(|error| ImportError::from_sqlite(0, error))?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| ImportError::from_sqlite(0, error))?;
    names
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| ImportError::from_sqlite(0, error))
}

/// The id the last committed batch reached and whether the import is finished, from the new
/// file. A file with no progress row has copied nothing.
fn progress(transaction: &rusqlite::Transaction<'_>) -> rusqlite::Result<(i64, bool)> {
    let row: Option<(i64, i64)> = transaction
        .query_row(
            "SELECT last_id, done FROM legacy_import WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(row.map_or((0, false), |(last_id, done)| (last_id, done != 0)))
}

/// Reads up to one batch of legacy rows with ids above `after`, oldest first.
fn read_batch(source: &Connection, select: &str, after: i64) -> rusqlite::Result<Vec<LegacyRow>> {
    let mut statement = source.prepare_cached(select)?;
    let rows = statement.query_map([after], |row| {
        Ok(LegacyRow {
            id: row.get(0)?,
            time_ms: row.get(1)?,
            level: row.get(2)?,
            component: row.get(3)?,
            role: row.get(4)?,
            target: row.get(5)?,
            message: row.get(6)?,
            fields: row.get(7)?,
        })
    })?;
    rows.collect()
}

/// Renames the imported legacy file to `logs.sqlite3.migrated`, or the next free name after it,
/// so an earlier import's file is never overwritten.
fn rename_migrated(legacy: &Path) -> Result<(), ImportError> {
    let base = legacy.with_file_name(format!(
        "{}{MIGRATED_SUFFIX}",
        legacy.file_name().and_then(|name| name.to_str()).unwrap_or(LEGACY_FILE)
    ));
    let mut target = base.clone();
    let mut number = 1;
    while target.exists() {
        target = base.with_file_name(format!(
            "{}.{number}",
            base.file_name().and_then(|name| name.to_str()).unwrap_or(LEGACY_FILE)
        ));
        number += 1;
    }
    fs::rename(legacy, &target).map_err(|error| ImportError::Rename {
        message: format!("cannot rename {} to {}: {error}", legacy.display(), target.display()),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use rusqlite::{Connection, params};

    use crate::logs::{FILE_NAME, Logs, Retention};

    /// A fresh directory for one test. It holds the store directory, which is where the store
    /// file goes, and the old file beside it, as on a real machine.
    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gnoblin-store-migrate-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("store")).expect("creates the store directory");
        dir
    }

    /// The new store file for a test directory.
    fn store_file(dir: &Path) -> PathBuf {
        dir.join("store").join(FILE_NAME)
    }

    /// The old `gnoblin-log` file for a test directory.
    fn legacy_file(dir: &Path) -> PathBuf {
        dir.join("logs.sqlite3")
    }

    /// Writes an old-schema file with `rows` records whose messages are `row 0`, `row 1` and so
    /// on. The `role` column is present only when `with_role` is set, which is the shape before
    /// the column was added.
    fn old_file(path: &Path, rows: i64, with_role: bool) {
        let mut connection = Connection::open(path).expect("opens the old file");
        let role = if with_role {
            "role TEXT NOT NULL DEFAULT '',"
        } else {
            ""
        };
        connection
            .execute_batch(&format!(
                "CREATE TABLE log_records (
                    id INTEGER PRIMARY KEY,
                    time_ms INTEGER NOT NULL,
                    level TEXT NOT NULL,
                    component TEXT NOT NULL,
                    {role}
                    target TEXT NOT NULL,
                    message TEXT NOT NULL,
                    fields TEXT NOT NULL
                );"
            ))
            .expect("creates the old table");
        let insert = if with_role {
            "INSERT INTO log_records (time_ms, level, component, role, target, message, fields)
             VALUES (?1, 'INFO', 'compositor', 'layout', 'gnoblin::main', ?2, '')"
        } else {
            "INSERT INTO log_records (time_ms, level, component, target, message, fields)
             VALUES (?1, 'INFO', 'compositor', 'gnoblin::main', ?2, '')"
        };
        let transaction = connection.transaction().expect("begins");
        for index in 0..rows {
            transaction
                .execute(insert, params![index, format!("row {index}")])
                .expect("inserts an old row");
        }
        transaction.commit().expect("commits the old rows");
    }

    /// The (message, role) of every record in the store file, in id order, read on a connection
    /// of its own so the store is not involved.
    fn stored(dir: &Path) -> Vec<(String, String)> {
        let connection = Connection::open(store_file(dir)).expect("opens the store file");
        let mut statement = connection
            .prepare("SELECT message, role FROM log_records ORDER BY id")
            .expect("prepares");
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("queries");
        rows.collect::<rusqlite::Result<Vec<_>>>().expect("reads")
    }

    /// Opens the store file writable with the legacy file beside it, which runs the import.
    fn open_with_legacy(dir: &Path) -> Logs {
        Logs::open(&store_file(dir), true, Retention::default(), Some(&legacy_file(dir)))
            .expect("the store opens even when the import stops")
    }

    #[test]
    fn an_old_file_with_a_role_column_is_imported_and_renamed() {
        let dir = scratch("with-role");
        old_file(&legacy_file(&dir), 3, true);
        drop(open_with_legacy(&dir));

        let records = stored(&dir);
        assert_eq!(records.len(), 3, "{records:?}");
        assert!(records.iter().all(|(_, role)| role == "layout"), "{records:?}");
        assert!(!legacy_file(&dir).exists(), "the old file is renamed away");
        assert!(
            dir.join("logs.sqlite3.migrated").exists(),
            "the old file is kept, renamed"
        );
    }

    #[test]
    fn an_old_file_without_a_role_column_takes_the_component_as_its_role() {
        let dir = scratch("no-role");
        old_file(&legacy_file(&dir), 2, false);
        drop(open_with_legacy(&dir));

        let records = stored(&dir);
        assert_eq!(records.len(), 2, "{records:?}");
        assert!(records.iter().all(|(_, role)| role == "compositor"), "{records:?}");
    }

    #[test]
    fn an_import_resumes_after_a_committed_batch_without_duplicates() {
        let dir = scratch("resume");
        old_file(&legacy_file(&dir), 2_500, true);
        // The store as a crash would leave it: the first batch of 1,000 rows is committed with
        // its progress, and nothing after it is.
        drop(Logs::open(&store_file(&dir), true, Retention::default(), None).expect("creates the store"));
        {
            let mut connection = Connection::open(store_file(&dir)).expect("opens the store file");
            let transaction = connection.transaction().expect("begins");
            for index in 0..1_000 {
                transaction
                    .execute(
                        "INSERT INTO log_records (time_ms, level, component, key, role, target, message, fields)
                         VALUES (?1, 'INFO', 'compositor', 'compositor', 'layout', 'gnoblin::main', ?2, '')",
                        params![index, format!("row {index}")],
                    )
                    .expect("inserts a committed row");
            }
            transaction
                .execute(
                    "INSERT INTO legacy_import (singleton, last_id, done) VALUES (1, 1000, 0)",
                    [],
                )
                .expect("records the progress");
            transaction.commit().expect("commits the first batch");
        }

        drop(open_with_legacy(&dir));

        let records = stored(&dir);
        let messages: std::collections::HashSet<&str> = records.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(records.len(), 2_500, "every row is present once");
        assert_eq!(messages.len(), 2_500, "no row is copied twice");
        assert!(!legacy_file(&dir).exists());
    }

    #[test]
    fn reopening_after_the_import_does_not_copy_again() {
        let dir = scratch("reopen");
        old_file(&legacy_file(&dir), 5, true);
        drop(open_with_legacy(&dir));
        drop(open_with_legacy(&dir));
        assert_eq!(stored(&dir).len(), 5);
    }

    #[test]
    fn a_rename_collision_takes_the_next_free_name() {
        let dir = scratch("collision");
        fs::write(dir.join("logs.sqlite3.migrated"), b"earlier import").expect("writes the earlier file");
        old_file(&legacy_file(&dir), 1, true);
        drop(open_with_legacy(&dir));

        assert_eq!(
            fs::read(dir.join("logs.sqlite3.migrated")).expect("reads the earlier file"),
            b"earlier import",
            "an earlier import's file is never overwritten"
        );
        assert!(dir.join("logs.sqlite3.migrated.1").exists());
        assert_eq!(stored(&dir).len(), 1);
    }

    #[test]
    fn a_newer_schema_is_left_untouched_and_the_store_still_opens() {
        let dir = scratch("newer");
        old_file(&legacy_file(&dir), 2, true);
        Connection::open(legacy_file(&dir))
            .expect("opens the old file")
            .pragma_update(None, "user_version", 7)
            .expect("marks a newer schema");
        drop(open_with_legacy(&dir));

        assert!(legacy_file(&dir).exists(), "a newer file is never renamed");
        assert!(stored(&dir).is_empty(), "no row is copied from a newer file");
    }

    #[test]
    fn an_unrecognised_schema_is_left_untouched_and_the_store_still_opens() {
        let dir = scratch("unrecognised");
        Connection::open(legacy_file(&dir))
            .expect("opens the old file")
            .execute_batch(
                "CREATE TABLE log_records (id INTEGER PRIMARY KEY, time_ms INTEGER NOT NULL,
                    level TEXT NOT NULL, component TEXT NOT NULL, target TEXT NOT NULL,
                    fields TEXT NOT NULL);
                 INSERT INTO log_records (time_ms, level, component, target, fields)
                    VALUES (1, 'INFO', 'compositor', 'x', '');",
            )
            .expect("creates a table without a message column");
        drop(open_with_legacy(&dir));

        assert!(legacy_file(&dir).exists(), "an unrecognised file is never renamed");
        assert!(stored(&dir).is_empty());
    }

    #[test]
    fn a_damaged_old_file_is_left_untouched_and_the_store_still_opens() {
        let dir = scratch("damaged");
        fs::write(legacy_file(&dir), b"this is not a database at all").expect("writes the damage");
        drop(open_with_legacy(&dir));

        assert!(legacy_file(&dir).exists(), "a damaged file is never renamed");
        assert!(stored(&dir).is_empty());
    }
}
