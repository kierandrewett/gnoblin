//! The `log` kind: the process's log records, appended as they happen and kept in their own file.
//!
//! The file is `log.sqlite3` in the store directory. The `gnoblin-log` pipeline hands batches of
//! records to [`Logs`] through its [`RecordSink`] trait, so `gnoblin-log` never depends on this
//! crate. A batch is one transaction. A failed batch is reported once on stderr, not through the
//! logger, because logging the failure would feed the same pipeline. Retention runs when the file
//! opens and again once a minute while records arrive.
//!
//! The kind is not a key-value namespace: its name is reserved, so no namespace can collide with
//! its file. Reads go through [`LogQuery`], which compiles to bound SQL and takes no SQL text.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use gnoblin_log::record::{LogRecord, RecordSink};
use rusqlite::types::Value;
use rusqlite::{Connection, params, params_from_iter};

use crate::{StoreError, kv, log};

mod migrate;
mod retention;

pub(crate) use migrate::legacy_path;
pub use retention::Retention;

/// The file name of the log kind, inside the store directory.
pub(crate) const FILE_NAME: &str = "log.sqlite3";

/// The name no key-value namespace may take, because it is the log kind's file stem.
pub(crate) const RESERVED_NAME: &str = "log";

/// How often retention runs while records are arriving. It also runs at every writable open.
const TRIM_EVERY: Duration = Duration::from_secs(60);

/// The migrations of the log file. Version 1 is the record table, with the columns gnoblin-log
/// wrote before the store took the kind over, so a file migrated from it keeps its shape.
/// Version 2 is the progress table of the one-off import of that old file (see `migrate`).
/// Version 3 adds the indexed `key` column. Existing rows take their component as their key,
/// because the component is the area that logged them. Each step is forward-only.
pub(crate) const LOG_STEPS: &[&str] = &[
    "CREATE TABLE log_records (
        id INTEGER PRIMARY KEY,
        time_ms INTEGER NOT NULL,
        level TEXT NOT NULL,
        component TEXT NOT NULL,
        role TEXT NOT NULL DEFAULT '',
        target TEXT NOT NULL,
        message TEXT NOT NULL,
        fields TEXT NOT NULL
    );
    CREATE INDEX log_records_time ON log_records (time_ms);",
    "CREATE TABLE legacy_import (
        singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
        last_id INTEGER NOT NULL,
        done INTEGER NOT NULL
    );",
    "ALTER TABLE log_records ADD COLUMN key TEXT NOT NULL DEFAULT '';
    UPDATE log_records SET key = component;
    CREATE INDEX log_records_key ON log_records (key, id);",
];

/// The insert every record uses. The statement is cached on the writer connection.
const INSERT_SQL: &str = "INSERT INTO log_records (time_ms, level, component, key, role, target, message, fields)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)";

/// The columns a [`StoredRecord`] reads, in the order [`read_record`] takes them.
const SELECT_COLUMNS: &str = "id, time_ms, level, component, role, target, message, fields";

/// One record as the store keeps it, with the id the store gave it on append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRecord {
    /// The record's id: it rises with each append, so ids give the order records were written.
    pub id: i64,
    /// Milliseconds since the Unix epoch, taken when the event was logged.
    pub time_ms: i64,
    /// The level name the writer uses: `ERROR`, `WARN`, `INFO`, `DEBUG` or `TRACE`.
    pub level: String,
    /// The component that logged the event.
    pub component: String,
    /// The process role that logged the event, such as `compositor`. Empty when none was given.
    pub role: String,
    /// The event's target, which is `gnoblin::<component>` for a gnoblin event.
    pub target: String,
    /// The event's message, with secrets scrubbed by the layer that logged it.
    pub message: String,
    /// The event's other fields, as `name=value` pairs.
    pub fields: String,
}

/// The conditions a read of the log kind keeps. Every condition that is set must hold; an unset
/// one matches every record. The default query matches everything, oldest first, with no limit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogQuery {
    /// Only records at or after this time, in milliseconds since the Unix epoch.
    pub since_ms: Option<i64>,
    /// Only records strictly before this time.
    pub until_ms: Option<i64>,
    /// Only records at this level or above, named as the writer names them (`INFO`, `WARN`, ...).
    /// The comparison ignores case; a name the writer never produces ranks as an error.
    pub min_level: Option<String>,
    /// Only records written by this process role, compared exactly.
    pub role: Option<String>,
    /// Only records logged by this component, compared exactly.
    pub component: Option<String>,
    /// Only records whose target matches exactly.
    pub target: Option<String>,
    /// Only records whose message contains this text. It must not be empty.
    pub contains: Option<String>,
    /// Only records with an id above this. A follower sets it to the newest id it has seen.
    pub after_id: Option<i64>,
    /// Only records with an id at or below this, so one read sees a fixed prefix of the log.
    pub up_to_id: Option<i64>,
    /// The most records a search returns. Counts and groups ignore it.
    pub limit: Option<u32>,
    /// Whether a search returns the newest records first rather than the oldest.
    pub newest_first: bool,
}

/// The column a [`Logs::group_by_count`] groups its records by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupBy {
    /// Group by level name.
    Level,
    /// Group by process role.
    Role,
    /// Group by component.
    Component,
    /// Group by event target.
    Target,
}

/// The log kind of one store: a handle to its file. Clones share the file and its writer.
///
/// A handle is a [`RecordSink`], so the logger can take it directly. A read-only handle accepts
/// batches and writes nothing.
#[derive(Clone)]
pub struct Logs {
    inner: Arc<LogsInner>,
}

/// The shared state behind every clone of a [`Logs`].
struct LogsInner {
    /// The file, for the failure report.
    path: PathBuf,
    /// The limits applied at open and while records arrive.
    retention: Retention,
    /// The writer, or `None` when the store is read-only.
    writer: Option<Mutex<LogsWriter>>,
    /// A read-only connection, so a search never waits on a batch being written.
    reader: Mutex<Connection>,
    /// Set while the last write failed, so a run of failures is reported once, not per batch.
    failing: AtomicBool,
}

/// The writer connection and when retention last ran on it.
struct LogsWriter {
    connection: Connection,
    trimmed_at: Instant,
}

impl Logs {
    /// Opens the log file at `path`.
    ///
    /// A writable open creates the file if needed, applies `retention`, and when the file is
    /// damaged moves it aside and starts a fresh one. A read-only open needs the file to exist
    /// and to be at the latest version, and it writes nothing.
    ///
    /// # Errors
    ///
    /// `store-unavailable` when a read-only open finds no file. `corrupt` when a read-only open
    /// finds a damaged file, or a writable open still finds one after it was moved aside.
    /// `migration-failed` when a step fails or the file came from a newer store.
    pub(crate) fn open(
        path: &Path,
        writable: bool,
        retention: Retention,
        legacy: Option<&Path>,
    ) -> Result<Logs, StoreError> {
        if !writable && !path.exists() {
            return Err(StoreError::StoreUnavailable(format!(
                "{} does not exist and the store is read-only",
                path.display()
            )));
        }
        let writer = if writable {
            let mut connection = open_connection(path, true)?;
            // The one-off import of the old gnoblin-log file runs before retention, so imported
            // rows are trimmed by the same rules as the rest. A failure is named on stderr and
            // never stops the store from starting; the next writable open tries again.
            if let Some(legacy) = legacy
                && let Err(error) = migrate::run(&mut connection, legacy)
            {
                log::warn!("cannot import the old log file ({}): {error}", error.code());
            }
            // Retention is best effort at open: a failure is logged, and the file still serves.
            if let Err(error) = retention::apply(&connection, retention, kv::now_ms()) {
                log::warn!("cannot apply log retention to {}: {error}", path.display());
            }
            Some(Mutex::new(LogsWriter {
                connection,
                trimmed_at: Instant::now(),
            }))
        } else {
            // Check the file the way a writable open would, without writing to it.
            drop(open_connection(path, false)?);
            None
        };
        let reader = kv::open_reader(path)?;
        Ok(Logs {
            inner: Arc::new(LogsInner {
                path: path.to_path_buf(),
                retention,
                writer,
                reader: Mutex::new(reader),
                failing: AtomicBool::new(false),
            }),
        })
    }

    /// Returns the records that match `query`, in id order, with its limit applied.
    ///
    /// # Errors
    ///
    /// `invalid-query` for an empty `contains`. `database` for any other SQLite failure.
    pub fn search(&self, query: &LogQuery) -> Result<Vec<StoredRecord>, StoreError> {
        let (filter, mut values) = filter(query)?;
        let order = if query.newest_first { "DESC" } else { "ASC" };
        let mut sql = format!("SELECT {SELECT_COLUMNS} FROM log_records{filter} ORDER BY id {order}");
        if let Some(limit) = query.limit {
            sql.push_str(" LIMIT ?");
            values.push(Value::Integer(i64::from(limit)));
        }
        let reader = lock(&self.inner.reader);
        let mut statement = reader.prepare_cached(&sql).map_err(kv::map_sqlite)?;
        let rows = statement
            .query_map(params_from_iter(values), read_record)
            .map_err(kv::map_sqlite)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(kv::map_sqlite)
    }

    /// Counts the records that match `query`. The limit and the order do not apply.
    ///
    /// # Errors
    ///
    /// `invalid-query` for an empty `contains`. `database` for any other SQLite failure.
    pub fn count(&self, query: &LogQuery) -> Result<u64, StoreError> {
        let (filter, values) = filter(query)?;
        let reader = lock(&self.inner.reader);
        let rows: i64 = reader
            .query_row(
                &format!("SELECT COUNT(*) FROM log_records{filter}"),
                params_from_iter(values),
                |row| row.get(0),
            )
            .map_err(kv::map_sqlite)?;
        Ok(u64::try_from(rows.max(0)).unwrap_or(0))
    }

    /// Counts the records that match `query` in each group of `by`, in the group's order. The
    /// limit and the order do not apply.
    ///
    /// # Errors
    ///
    /// `invalid-query` for an empty `contains`. `database` for any other SQLite failure.
    pub fn group_by_count(&self, query: &LogQuery, by: GroupBy) -> Result<Vec<(String, u64)>, StoreError> {
        let (filter, values) = filter(query)?;
        // The column comes from a closed set, so it is safe to place in the text of the SQL.
        let column = match by {
            GroupBy::Level => "level",
            GroupBy::Role => "role",
            GroupBy::Component => "component",
            GroupBy::Target => "target",
        };
        let sql = format!("SELECT {column}, COUNT(*) FROM log_records{filter} GROUP BY {column} ORDER BY {column}");
        let reader = lock(&self.inner.reader);
        let mut statement = reader.prepare_cached(&sql).map_err(kv::map_sqlite)?;
        let rows = statement
            .query_map(params_from_iter(values), |row| {
                let count: i64 = row.get(1)?;
                Ok((row.get::<_, String>(0)?, u64::try_from(count.max(0)).unwrap_or(0)))
            })
            .map_err(kv::map_sqlite)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(kv::map_sqlite)
    }

    /// The id of the newest record, or 0 when the log is empty. A follower reads from here.
    ///
    /// # Errors
    ///
    /// `database` for any SQLite failure.
    pub fn newest_id(&self) -> Result<i64, StoreError> {
        let reader = lock(&self.inner.reader);
        reader
            .query_row("SELECT COALESCE(MAX(id), 0) FROM log_records", [], |row| row.get(0))
            .map_err(kv::map_sqlite)
    }

    /// Runs the write path for one batch. A failure is reported once per run of failures.
    fn append(&self, records: &[LogRecord]) {
        let Some(writer) = &self.inner.writer else {
            return;
        };
        let mut writer = lock(writer);
        match insert(&mut writer.connection, records) {
            Ok(()) => self.inner.failing.store(false, Ordering::Relaxed),
            Err(error) => self.report(&error),
        }
        if writer.trimmed_at.elapsed() >= TRIM_EVERY {
            writer.trimmed_at = Instant::now();
            let retention = self.inner.retention;
            if let Err(error) = retention::apply(&writer.connection, retention, kv::now_ms()) {
                self.report(&kv::map_sqlite(error));
            }
        }
    }

    /// Reports a failed write on stderr, once per run of failures. Stderr rather than the logger,
    /// because a log line about the log would go back through the same pipeline.
    fn report(&self, error: &StoreError) {
        if !self.inner.failing.swap(true, Ordering::Relaxed) {
            eprintln!(
                "gnoblin-store: cannot write the log to {}: {error}; records are dropped until it recovers",
                self.inner.path.display()
            );
        }
    }
}

impl RecordSink for Logs {
    /// Appends `records` in one transaction, then applies retention if a minute has passed.
    fn write_batch(&self, records: &[LogRecord]) {
        self.append(records);
    }
}

/// Opens the file with [`kv::open_file`]. A damaged file is moved aside and a fresh one opened in
/// its place, so the caller gets a working file rather than a failure. A read-only open never
/// moves anything.
fn open_connection(path: &Path, writable: bool) -> Result<Connection, StoreError> {
    match kv::open_file(path, writable, LOG_STEPS) {
        Err(StoreError::Corrupt(reason)) if writable => {
            kv::quarantine(path, &reason);
            kv::open_file(path, true, LOG_STEPS)
        }
        other => other,
    }
}

/// Appends `records` in one transaction, oldest first.
fn insert(connection: &mut Connection, records: &[LogRecord]) -> Result<(), StoreError> {
    let transaction = connection.transaction().map_err(kv::map_sqlite)?;
    {
        let mut statement = transaction.prepare_cached(INSERT_SQL).map_err(kv::map_sqlite)?;
        for record in records {
            statement
                .execute(params![
                    record.time_ms,
                    record.level,
                    record.component,
                    record.key,
                    record.role,
                    record.target,
                    record.message,
                    record.fields,
                ])
                .map_err(kv::map_sqlite)?;
        }
    }
    transaction.commit().map_err(kv::map_sqlite)
}

/// Reads one row selected with [`SELECT_COLUMNS`].
fn read_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredRecord> {
    Ok(StoredRecord {
        id: row.get(0)?,
        time_ms: row.get(1)?,
        level: row.get(2)?,
        component: row.get(3)?,
        role: row.get(4)?,
        target: row.get(5)?,
        message: row.get(6)?,
        fields: row.get(7)?,
    })
}

/// The SQL that ranks a record's level, as the number [`level_rank`] gives the same name. The
/// placeholder takes the rank of the minimum level.
const LEVEL_RANK_SQL: &str = "(CASE level WHEN 'TRACE' THEN 0 WHEN 'DEBUG' THEN 1 WHEN 'INFO' THEN 2 \
    WHEN 'WARN' THEN 3 ELSE 4 END) >= ?";

/// The WHERE clause for `query` and its values, in placeholder order. An empty clause matches
/// every record.
fn filter(query: &LogQuery) -> Result<(String, Vec<Value>), StoreError> {
    let mut clauses: Vec<&str> = Vec::new();
    let mut values: Vec<Value> = Vec::new();
    if let Some(since) = query.since_ms {
        clauses.push("time_ms >= ?");
        values.push(Value::Integer(since));
    }
    if let Some(until) = query.until_ms {
        clauses.push("time_ms < ?");
        values.push(Value::Integer(until));
    }
    if let Some(level) = &query.min_level {
        clauses.push(LEVEL_RANK_SQL);
        values.push(Value::Integer(level_rank(level)));
    }
    if let Some(role) = &query.role {
        clauses.push("role = ?");
        values.push(Value::Text(role.clone()));
    }
    if let Some(component) = &query.component {
        clauses.push("component = ?");
        values.push(Value::Text(component.clone()));
    }
    if let Some(target) = &query.target {
        clauses.push("target = ?");
        values.push(Value::Text(target.clone()));
    }
    if let Some(text) = &query.contains {
        if text.is_empty() {
            return Err(StoreError::InvalidQuery("contains needs some text".to_owned()));
        }
        clauses.push("instr(message, ?) > 0");
        values.push(Value::Text(text.clone()));
    }
    if let Some(after) = query.after_id {
        clauses.push("id > ?");
        values.push(Value::Integer(after));
    }
    if let Some(up_to) = query.up_to_id {
        clauses.push("id <= ?");
        values.push(Value::Integer(up_to));
    }
    if clauses.is_empty() {
        return Ok((String::new(), values));
    }
    Ok((format!(" WHERE {}", clauses.join(" AND ")), values))
}

/// The rank of a level name: higher is more severe. Names the writer does not produce rank as
/// errors, so an unknown name never hides a record.
pub(crate) fn level_rank(name: &str) -> i64 {
    if name.eq_ignore_ascii_case("TRACE") {
        0
    } else if name.eq_ignore_ascii_case("DEBUG") {
        1
    } else if name.eq_ignore_ascii_case("INFO") {
        2
    } else if name.eq_ignore_ascii_case("WARN") {
        3
    } else {
        4
    }
}

/// Locks `mutex`, ignoring poison, as the rest of the store does.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::scratch;

    /// A record with the given time, level, role and component; the target follows the component.
    fn record(time_ms: i64, level: &'static str, role: &'static str, component: &str, message: &str) -> LogRecord {
        LogRecord {
            time_ms,
            level,
            component: component.to_owned(),
            key: component.to_owned(),
            role,
            target: format!("gnoblin::{component}"),
            message: message.to_owned(),
            fields: String::new(),
        }
    }

    /// Opens a writable log file in a fresh scratch directory.
    fn open(label: &str, retention: Retention) -> (Logs, PathBuf) {
        let dir = scratch(label);
        let logs = Logs::open(&dir.join(FILE_NAME), true, retention, None).expect("opens");
        (logs, dir)
    }

    /// A writable log file with the five records every filter test reads.
    fn seeded(label: &str) -> (Logs, PathBuf) {
        let (logs, dir) = open(label, Retention::default());
        logs.write_batch(&[
            record(1_000, "INFO", "compositor", "layout", "layout settled"),
            record(2_000, "DEBUG", "compositor", "layout", "frame timing"),
            record(3_000, "WARN", "portal", "dialogs", "dialog slow to open"),
            record(4_000, "ERROR", "compositor", "outputs", "output lost"),
            record(5_000, "TRACE", "portal", "dialogs", "trace detail"),
        ]);
        (logs, dir)
    }

    fn messages(records: &[StoredRecord]) -> Vec<&str> {
        records.iter().map(|record| record.message.as_str()).collect()
    }

    #[test]
    fn appended_records_read_back_oldest_first_with_increasing_ids() {
        let (logs, _dir) = seeded("append-order");
        let all = logs.search(&LogQuery::default()).expect("searches");
        assert_eq!(
            messages(&all),
            [
                "layout settled",
                "frame timing",
                "dialog slow to open",
                "output lost",
                "trace detail"
            ]
        );
        assert!(all.windows(2).all(|pair| pair[0].id < pair[1].id));
        assert_eq!(all[0].role, "compositor");
        assert_eq!(all[0].target, "gnoblin::layout");
        assert_eq!(logs.newest_id().expect("newest"), all[4].id);
    }

    #[test]
    fn a_level_filter_keeps_that_level_and_above() {
        let (logs, _dir) = seeded("level");
        let query = LogQuery {
            min_level: Some("warn".to_owned()),
            ..LogQuery::default()
        };
        assert_eq!(
            messages(&logs.search(&query).expect("searches")),
            ["dialog slow to open", "output lost"]
        );
    }

    #[test]
    fn role_component_and_target_are_exact_matches() {
        let (logs, _dir) = seeded("exact");
        let by_role = LogQuery {
            role: Some("portal".to_owned()),
            ..LogQuery::default()
        };
        assert_eq!(logs.count(&by_role).expect("counts"), 2);

        let by_component = LogQuery {
            component: Some("layout".to_owned()),
            ..LogQuery::default()
        };
        assert_eq!(logs.count(&by_component).expect("counts"), 2);

        let by_target = LogQuery {
            target: Some("gnoblin::outputs".to_owned()),
            ..LogQuery::default()
        };
        assert_eq!(messages(&logs.search(&by_target).expect("searches")), ["output lost"]);

        let partial = LogQuery {
            component: Some("layo".to_owned()),
            ..LogQuery::default()
        };
        assert_eq!(logs.count(&partial).expect("counts"), 0, "a component is not a prefix");
    }

    #[test]
    fn a_time_range_includes_since_and_excludes_until() {
        let (logs, _dir) = seeded("time");
        let query = LogQuery {
            since_ms: Some(2_000),
            until_ms: Some(4_000),
            ..LogQuery::default()
        };
        assert_eq!(
            messages(&logs.search(&query).expect("searches")),
            ["frame timing", "dialog slow to open"]
        );
    }

    #[test]
    fn contains_matches_message_text() {
        let (logs, _dir) = seeded("contains");
        let query = LogQuery {
            contains: Some("slow".to_owned()),
            ..LogQuery::default()
        };
        assert_eq!(
            messages(&logs.search(&query).expect("searches")),
            ["dialog slow to open"]
        );

        let empty = LogQuery {
            contains: Some(String::new()),
            ..LogQuery::default()
        };
        assert!(matches!(logs.search(&empty), Err(StoreError::InvalidQuery(_))));
    }

    #[test]
    fn after_and_up_to_ids_bound_a_follow() {
        let (logs, _dir) = seeded("ids");
        let all = logs.search(&LogQuery::default()).expect("searches");
        let after = LogQuery {
            after_id: Some(all[1].id),
            up_to_id: Some(all[3].id),
            ..LogQuery::default()
        };
        assert_eq!(
            messages(&logs.search(&after).expect("searches")),
            ["dialog slow to open", "output lost"]
        );
    }

    #[test]
    fn each_record_is_stored_under_the_key_it_carries() {
        let (logs, dir) = open("keyed", Retention::default());
        let mut keyed = record(1_000, "INFO", "compositor", "layout", "settled");
        keyed.key = "outputs".to_owned();
        logs.write_batch(&[keyed]);
        let connection = Connection::open(dir.join(FILE_NAME)).expect("opens the file");
        let (component, key): (String, String) = connection
            .query_row("SELECT component, key FROM log_records", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .expect("reads the row");
        assert_eq!((component.as_str(), key.as_str()), ("layout", "outputs"));
    }

    #[test]
    fn a_version_two_file_gains_the_key_from_each_records_component() {
        let dir = scratch("key-migration");
        let path = dir.join(FILE_NAME);
        {
            let connection = Connection::open(&path).expect("creates the file");
            connection.execute_batch(LOG_STEPS[0]).expect("version 1 schema");
            connection.execute_batch(LOG_STEPS[1]).expect("version 2 schema");
            connection
                .pragma_update(None, "user_version", 2)
                .expect("marks version 2");
            connection
                .execute(
                    "INSERT INTO log_records (time_ms, level, component, role, target, message, fields)
                     VALUES (1, 'INFO', 'layout', 'compositor', 'gnoblin::layout', 'old row', '')",
                    [],
                )
                .expect("inserts a row from before the key");
        }
        drop(Logs::open(&path, true, Retention::default(), None).expect("migrates the file"));

        let connection = Connection::open(&path).expect("reopens the file");
        let key: String = connection
            .query_row("SELECT key FROM log_records", [], |row| row.get(0))
            .expect("reads the key");
        assert_eq!(key, "layout");
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("reads the version");
        assert_eq!(version, LOG_STEPS.len() as i64);
    }

    #[test]
    fn a_limit_takes_the_oldest_or_newest_records() {
        let (logs, _dir) = seeded("limit");
        let oldest = LogQuery {
            limit: Some(2),
            ..LogQuery::default()
        };
        assert_eq!(
            messages(&logs.search(&oldest).expect("searches")),
            ["layout settled", "frame timing"]
        );

        let newest = LogQuery {
            limit: Some(2),
            newest_first: true,
            ..LogQuery::default()
        };
        assert_eq!(
            messages(&logs.search(&newest).expect("searches")),
            ["trace detail", "output lost"]
        );
        assert_eq!(logs.count(&newest).expect("counts"), 5, "a count ignores the limit");
    }

    #[test]
    fn counts_and_groups_follow_the_query() {
        let (logs, _dir) = seeded("groups");
        assert_eq!(logs.count(&LogQuery::default()).expect("counts"), 5);
        assert_eq!(
            logs.group_by_count(&LogQuery::default(), GroupBy::Role)
                .expect("groups"),
            [("compositor".to_owned(), 3), ("portal".to_owned(), 2)]
        );
        let warnings = LogQuery {
            min_level: Some("WARN".to_owned()),
            ..LogQuery::default()
        };
        assert_eq!(
            logs.group_by_count(&warnings, GroupBy::Level).expect("groups"),
            [("ERROR".to_owned(), 1), ("WARN".to_owned(), 1)]
        );
    }

    #[test]
    fn a_batch_of_a_thousand_records_is_stored_whole() {
        let (logs, _dir) = open("batch", Retention::default());
        let batch: Vec<LogRecord> = (0..1_000)
            .map(|index| record(index, "INFO", "compositor", "layout", &format!("row {index}")))
            .collect();
        logs.write_batch(&batch);
        assert_eq!(logs.count(&LogQuery::default()).expect("counts"), 1_000);
        let last = LogQuery {
            limit: Some(1),
            newest_first: true,
            ..LogQuery::default()
        };
        assert_eq!(messages(&logs.search(&last).expect("searches")), ["row 999"]);
    }

    #[test]
    fn a_writable_open_applies_the_age_limit() {
        let (logs, dir) = open("age", Retention::default());
        logs.write_batch(&[
            record(1, "INFO", "compositor", "layout", "ancient"),
            record(kv::now_ms(), "INFO", "compositor", "layout", "recent"),
        ]);
        drop(logs);

        let kept = Logs::open(
            &dir.join(FILE_NAME),
            true,
            Retention {
                max_age_days: 30,
                max_bytes: None,
            },
            None,
        )
        .expect("reopens");
        assert_eq!(
            messages(&kept.search(&LogQuery::default()).expect("searches")),
            ["recent"]
        );
    }

    #[test]
    fn a_writable_open_applies_the_size_limit() {
        let (logs, dir) = open("size", Retention::default());
        let big = "x".repeat(512);
        let batch: Vec<LogRecord> = (0..2_000)
            .map(|index| record(index, "INFO", "compositor", "layout", &big))
            .collect();
        logs.write_batch(&batch);
        drop(logs);

        let limit = 256 * 1024;
        let kept = Logs::open(
            &dir.join(FILE_NAME),
            true,
            Retention {
                max_age_days: 0,
                max_bytes: Some(limit),
            },
            None,
        )
        .expect("reopens");
        let remaining = kept.count(&LogQuery::default()).expect("counts");
        assert!(
            remaining > 0 && remaining < 2_000,
            "the size pass trims some records, kept {remaining}"
        );
        let newest = LogQuery {
            limit: Some(1),
            newest_first: true,
            ..LogQuery::default()
        };
        assert_eq!(
            kept.search(&newest).expect("searches")[0].id,
            2_000,
            "the newest survive"
        );
    }

    #[test]
    fn a_read_only_handle_reads_and_writes_nothing() {
        let (logs, dir) = seeded("read-only");
        drop(logs);

        let reader = Logs::open(&dir.join(FILE_NAME), false, Retention::default(), None).expect("opens");
        reader.write_batch(&[record(9_000, "INFO", "compositor", "layout", "ignored")]);
        assert_eq!(reader.count(&LogQuery::default()).expect("counts"), 5);
    }

    #[test]
    fn a_read_only_open_needs_the_file() {
        let dir = scratch("missing");
        let error = Logs::open(&dir.join(FILE_NAME), false, Retention::default(), None)
            .err()
            .expect("refused");
        assert!(matches!(error, StoreError::StoreUnavailable(_)), "{error:?}");
    }

    #[test]
    fn a_damaged_file_is_moved_aside_and_a_fresh_one_opens() {
        let dir = scratch("corrupt");
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, b"this is not a database at all, just text").expect("writes");

        let logs = Logs::open(&path, true, Retention::default(), None).expect("opens a fresh file");
        assert_eq!(logs.count(&LogQuery::default()).expect("counts"), 0);

        let moved: Vec<_> = std::fs::read_dir(&dir)
            .expect("lists")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("log.corrupt-"))
            .collect();
        assert_eq!(moved.len(), 1, "the damaged file was kept aside: {moved:?}");
    }
}
