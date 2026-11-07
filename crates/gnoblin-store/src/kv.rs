//! The in-process `kv` provider: one SQLite file per namespace, served through a writer and a
//! small pool of readers.
//!
//! Writes go through one connection behind a mutex, in `BEGIN IMMEDIATE` transactions, so two
//! writers never interleave and a batch commits or rolls back as one. Reads take a connection
//! from a pool of `query_only` connections; WAL gives each reader a snapshot, so a read never
//! waits for a write and never sees half a batch.
//!
//! A batch checks its quota, its revisions and its value sizes inside the transaction. A refusal
//! rolls the whole batch back, so the file never holds a state no caller asked for.
//!
//! Watchers receive each committed change over a `std::sync::mpsc` channel. The send happens
//! while the writer lock is held, after the commit, so every watcher sees changes in commit
//! order and never sees one that was rolled back.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gnoblin_model::Value;
use rusqlite::types::Value as SqlValue;
use rusqlite::{
    Connection, ErrorCode, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params_from_iter,
};

use crate::encode::{decode, encode};
use crate::name::{covers, validate_key, validate_prefix};
use crate::query::{Order, Query};
use crate::schema::{self, STEPS};
use crate::{Options, State, StoreError};

use crate::log;

/// How many read connections a namespace keeps warm. More readers than this are opened on demand
/// and closed when they are returned, so an idle namespace holds only a few file handles.
const READER_POOL: usize = 4;

/// How long a connection waits on a lock before SQLite reports it busy.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// One stored item, as a consumer reads it back.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The key the value is stored under.
    pub key: String,
    /// The decoded value.
    pub value: Value,
    /// The namespace revision of the write that stored this value. Revisions only grow, and every
    /// write takes the next one, so a larger revision is a later write.
    pub revision: u64,
    /// The write time in milliseconds since the Unix epoch.
    pub updated_ms: i64,
}

/// One change reported to a watcher, after its batch committed.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// A value was stored. The entry holds the value as written.
    Put(Entry),
    /// A value was removed. Only the delete that removed it is reported, not a delete of a
    /// missing key.
    Delete {
        /// The key that was removed.
        key: String,
        /// The revision the delete took.
        revision: u64,
    },
}

impl Change {
    /// The key this change is for.
    pub fn key(&self) -> &str {
        match self {
            Change::Put(entry) => &entry.key,
            Change::Delete { key, .. } => key,
        }
    }
}

/// One write in a batch. Build with [`Op::put`] or [`Op::delete`].
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Store `value` under `key`, replacing any value there.
    Put {
        /// The key to store under.
        key: String,
        /// The value to store.
        value: Value,
        /// When set, the put applies only if the key's current revision is this one. `Some(0)`
        /// means the key must be absent, so the put creates it and never overwrites.
        if_revision: Option<u64>,
    },
    /// Remove the value under `key`.
    Delete {
        /// The key to remove.
        key: String,
        /// When set, the delete applies only if the key's current revision is this one.
        if_revision: Option<u64>,
    },
}

impl Op {
    /// A put with no revision check.
    pub fn put(key: &str, value: Value) -> Op {
        Op::Put {
            key: key.to_owned(),
            value,
            if_revision: None,
        }
    }

    /// A delete with no revision check.
    pub fn delete(key: &str) -> Op {
        Op::Delete {
            key: key.to_owned(),
            if_revision: None,
        }
    }

    /// Adds a revision check to this op: it applies only if the key is at `revision`.
    pub fn if_revision(self, revision: u64) -> Op {
        match self {
            Op::Put { key, value, .. } => Op::Put {
                key,
                value,
                if_revision: Some(revision),
            },
            Op::Delete { key, .. } => Op::Delete {
                key,
                if_revision: Some(revision),
            },
        }
    }
}

/// What one op of a committed batch did. Outcomes come back in op order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The put stored a value at `revision`.
    Put {
        /// The revision the write took.
        revision: u64,
    },
    /// The delete ran. `removed` is false when the key was already absent.
    Delete {
        /// Whether a value was removed.
        removed: bool,
    },
}

/// A namespace's open file: the writer, the reader pool and the lifecycle state.
pub(crate) struct Kv {
    path: PathBuf,
    writable: bool,
    options: Options,
    state: Mutex<State>,
    writer: Mutex<Option<Connection>>,
    readers: Mutex<Vec<Connection>>,
    watchers: Mutex<Vec<(String, Sender<Change>)>>,
    disk_full_logged: AtomicBool,
}

impl Kv {
    /// Opens the namespace file at `path`, checking, migrating and taking its lock.
    ///
    /// A damaged file is quarantined and the returned namespace is [`State::Quarantined`], so
    /// this returns `Ok` for damage. It returns `Err` only when the file is fine but cannot be
    /// opened or migrated, and that namespace is then not created.
    pub(crate) fn open(path: PathBuf, options: Options) -> Result<Kv, StoreError> {
        if !options.writable && !path.exists() {
            return Err(StoreError::StoreUnavailable(format!(
                "{} does not exist and the store is read-only",
                path.display()
            )));
        }
        match open_file(&path, options.writable) {
            Ok(conn) => Ok(Kv::new(path, options, State::Ready, options.writable.then_some(conn))),
            Err(StoreError::Corrupt(reason)) => {
                quarantine(&path, &reason);
                Ok(Kv::new(path, options, State::Quarantined, None))
            }
            Err(error) => Err(error),
        }
    }

    fn new(path: PathBuf, options: Options, state: State, writer: Option<Connection>) -> Kv {
        Kv {
            path,
            writable: options.writable,
            options,
            state: Mutex::new(state),
            writer: Mutex::new(writer),
            readers: Mutex::new(Vec::new()),
            watchers: Mutex::new(Vec::new()),
            disk_full_logged: AtomicBool::new(false),
        }
    }

    /// The current lifecycle state.
    pub(crate) fn state(&self) -> State {
        *lock(&self.state)
    }

    /// Whether writes are allowed at all. A quarantined namespace can still report `true` here,
    /// because the store was opened writable; its writes fail with `store-unavailable`.
    pub(crate) fn writable(&self) -> bool {
        self.writable
    }

    /// Moves to `next` if the lifecycle table allows it. Any other move is a bug in this crate,
    /// so it is reported as `internal` and the state does not change.
    fn move_to(&self, next: State) -> Result<(), StoreError> {
        let mut state = lock(&self.state);
        if !state.can_move_to(next) {
            return Err(StoreError::Internal(format!(
                "refused {} -> {}",
                state.as_str(),
                next.as_str()
            )));
        }
        *state = next;
        Ok(())
    }

    /// Refuses a verb unless the namespace is ready.
    fn ensure_ready(&self) -> Result<(), StoreError> {
        match self.state() {
            State::Ready => Ok(()),
            other => Err(unavailable(other)),
        }
    }

    /// Moves to `closing`, drops the writer and readers, and ends every watch. Any verb after
    /// this is refused.
    pub(crate) fn close(&self) {
        // A refused move here means the state was already closing, which is the goal anyway.
        let _ = self.move_to(State::Closing);
        *lock(&self.writer) = None;
        lock(&self.readers).clear();
        lock(&self.watchers).clear();
    }

    /// Runs `ops` in one `BEGIN IMMEDIATE` transaction and returns their outcomes.
    pub(crate) fn apply(&self, ops: &[Op]) -> Result<Vec<Outcome>, StoreError> {
        if !self.writable {
            return Err(StoreError::ReadOnly);
        }
        let mut writer = lock(&self.writer);
        self.ensure_ready()?;
        let conn = writer
            .as_mut()
            .ok_or_else(|| StoreError::StoreUnavailable("the namespace has no writer".to_owned()))?;

        let result = self.apply_in(conn, ops);
        match result {
            Ok((outcomes, changes)) => {
                self.notify(&changes);
                Ok(outcomes)
            }
            Err(error) => {
                if matches!(error, StoreError::DiskFull(_)) && !self.disk_full_logged.swap(true, Ordering::Relaxed) {
                    log::warn!("disk full writing {}: the batch was rolled back", self.path.display());
                }
                Err(error)
            }
        }
    }

    /// The body of [`Kv::apply`], run with the writer held. The transaction commits only if
    /// every op succeeds; on the first error it is dropped and rolls back.
    fn apply_in(&self, conn: &mut Connection, ops: &[Op]) -> Result<(Vec<Outcome>, Vec<Change>), StoreError> {
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite)?;
        let now = now_ms();
        let mut outcomes = Vec::with_capacity(ops.len());
        let mut changes = Vec::with_capacity(ops.len());
        for op in ops {
            match op {
                Op::Put {
                    key,
                    value,
                    if_revision,
                } => {
                    let (outcome, change) = self.put(&tx, key, value, *if_revision, now)?;
                    outcomes.push(outcome);
                    changes.push(change);
                }
                Op::Delete { key, if_revision } => {
                    let (outcome, change) = self.delete(&tx, key, *if_revision)?;
                    outcomes.push(outcome);
                    changes.extend(change);
                }
            }
        }
        tx.commit().map_err(map_sqlite)?;
        Ok((outcomes, changes))
    }

    /// Stores one value inside `tx`, checking the value size, the revision and the quota.
    fn put(
        &self,
        tx: &Transaction<'_>,
        key: &str,
        value: &Value,
        if_revision: Option<u64>,
        now: i64,
    ) -> Result<(Outcome, Change), StoreError> {
        validate_key(key)?;
        let bytes = encode(value, self.options.value_limit)?;
        let (current_revision, current_size) = current(tx, key)?;
        check_revision(key, if_revision, current_revision)?;

        let new_size = key.len() as u64 + bytes.len() as u64;
        let total = bytes_used(tx)?;
        let after = total - current_size.unwrap_or(0) + new_size;
        if after > self.options.quota {
            return Err(StoreError::QuotaExceeded {
                current: after,
                limit: self.options.quota,
            });
        }

        let revision = next_revision(tx)?;
        set_bytes(tx, after)?;
        let (number, text) = columns(value);
        tx.execute(
            "INSERT INTO items (key, rev, updated_ms, value, number, text) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(key) DO UPDATE SET rev = excluded.rev, updated_ms = excluded.updated_ms,
                 value = excluded.value, number = excluded.number, text = excluded.text",
            rusqlite::params![key, revision as i64, now, bytes, number, text],
        )
        .map_err(map_sqlite)?;

        // The trigram index mirrors the text column by row id. It is replaced on every write so
        // it never holds a stale string.
        let rowid: i64 = tx
            .query_row("SELECT rowid FROM items WHERE key = ?1", [key], |row| row.get(0))
            .map_err(map_sqlite)?;
        tx.execute("DELETE FROM items_fts WHERE rowid = ?1", [rowid])
            .map_err(map_sqlite)?;
        if matches!(text, SqlValue::Text(_)) {
            tx.execute(
                "INSERT INTO items_fts (rowid, text) VALUES (?1, ?2)",
                rusqlite::params![rowid, text],
            )
            .map_err(map_sqlite)?;
        }

        let entry = Entry {
            key: key.to_owned(),
            value: value.clone(),
            revision,
            updated_ms: now,
        };
        Ok((Outcome::Put { revision }, Change::Put(entry)))
    }

    /// Removes one value inside `tx`. A missing key is not an error: it returns `removed: false`
    /// and no change, unless the revision check fails.
    fn delete(
        &self,
        tx: &Transaction<'_>,
        key: &str,
        if_revision: Option<u64>,
    ) -> Result<(Outcome, Option<Change>), StoreError> {
        validate_key(key)?;
        let (current_revision, current_size) = current(tx, key)?;
        check_revision(key, if_revision, current_revision)?;
        let Some(size) = current_size else {
            return Ok((Outcome::Delete { removed: false }, None));
        };

        let rowid: i64 = tx
            .query_row("SELECT rowid FROM items WHERE key = ?1", [key], |row| row.get(0))
            .map_err(map_sqlite)?;
        tx.execute("DELETE FROM items_fts WHERE rowid = ?1", [rowid])
            .map_err(map_sqlite)?;
        tx.execute("DELETE FROM items WHERE key = ?1", [key])
            .map_err(map_sqlite)?;
        let total = bytes_used(tx)?;
        set_bytes(tx, total - size)?;
        let revision = next_revision(tx)?;
        Ok((
            Outcome::Delete { removed: true },
            Some(Change::Delete {
                key: key.to_owned(),
                revision,
            }),
        ))
    }

    /// Sends each change to the watchers whose prefix covers its key. A watcher whose receiver
    /// has been dropped is removed. Called with the writer held, so the order is commit order.
    fn notify(&self, changes: &[Change]) {
        let mut watchers = lock(&self.watchers);
        for change in changes {
            watchers.retain(|(prefix, sender)| {
                if !covers(prefix, change.key()) {
                    return true;
                }
                sender.send(change.clone()).is_ok()
            });
        }
    }

    /// Reads the entry under `key`.
    pub(crate) fn get(&self, key: &str) -> Result<Option<Entry>, StoreError> {
        validate_key(key)?;
        let Some(reader) = self.reader()? else {
            return Ok(None);
        };
        let raw = reader
            .query_row(
                "SELECT key, rev, updated_ms, value FROM items WHERE key = ?1",
                [key],
                raw_entry,
            )
            .optional()
            .map_err(map_sqlite)?;
        raw.map(decode_entry).transpose()
    }

    /// Lists keys under `prefix` after `after`, in key order, at most `limit` (clamped to 1–1000).
    pub(crate) fn list(&self, prefix: &str, after: Option<&str>, limit: usize) -> Result<Vec<String>, StoreError> {
        validate_prefix(prefix)?;
        if let Some(after) = after {
            validate_key(after)?;
        }
        let Some(reader) = self.reader()? else {
            return Ok(Vec::new());
        };
        let mut query = Query::new()
            .prefix(prefix)
            .order(Order::KeyAsc)
            .limit(limit.clamp(1, 1000));
        if let Some(after) = after {
            query = query.after(after);
        }
        let (sql, params) = query.compile("key", true)?;
        let mut statement = reader.prepare(&sql).map_err(map_sqlite)?;
        let rows = statement
            .query_map(params_from_iter(params), |row| row.get::<_, String>(0))
            .map_err(map_sqlite)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(map_sqlite)
    }

    /// Finds the entries that match `query`, in its order, with its limit and offset.
    pub(crate) fn search(&self, query: &Query) -> Result<Vec<Entry>, StoreError> {
        let Some(reader) = self.reader()? else {
            return Ok(Vec::new());
        };
        let (sql, params) = query.compile("key, rev, updated_ms, value", true)?;
        let mut statement = reader.prepare(&sql).map_err(map_sqlite)?;
        let rows = statement
            .query_map(params_from_iter(params), raw_entry)
            .map_err(map_sqlite)?;
        let raw = rows.collect::<Result<Vec<_>, _>>().map_err(map_sqlite)?;
        raw.into_iter().map(decode_entry).collect()
    }

    /// Counts the entries that match `query`, ignoring its limit and offset.
    pub(crate) fn count(&self, query: &Query) -> Result<u64, StoreError> {
        let Some(reader) = self.reader()? else {
            return Ok(0);
        };
        let (sql, params) = query.compile("COUNT(*)", false)?;
        let count: i64 = reader
            .query_row(&sql, params_from_iter(params), |row| row.get(0))
            .map_err(map_sqlite)?;
        Ok(count as u64)
    }

    /// Groups the keys that match `query` by their first `segments` segments.
    ///
    /// A key with `segments` or fewer segments groups under its whole key. Groups come back in
    /// key order, with the count of matching keys in each.
    pub(crate) fn group_by_count(&self, query: &Query, segments: usize) -> Result<Vec<(String, u64)>, StoreError> {
        if segments == 0 {
            return Err(StoreError::InvalidQuery(
                "group-by needs at least one segment".to_owned(),
            ));
        }
        let Some(reader) = self.reader()? else {
            return Ok(Vec::new());
        };
        let (sql, params) = query.compile("key", false)?;
        let mut statement = reader.prepare(&sql).map_err(map_sqlite)?;
        let keys = statement
            .query_map(params_from_iter(params), |row| row.get::<_, String>(0))
            .map_err(map_sqlite)?;
        let mut groups: BTreeMap<String, u64> = BTreeMap::new();
        for key in keys {
            let key = key.map_err(map_sqlite)?;
            let group = key.split('/').take(segments).collect::<Vec<_>>().join("/");
            *groups.entry(group).or_insert(0) += 1;
        }
        Ok(groups.into_iter().collect())
    }

    /// Subscribes to changes under `prefix`. A quarantined namespace returns a receiver that is
    /// already closed, so a caller sees end-of-stream rather than waiting forever.
    pub(crate) fn watch(&self, prefix: &str) -> Result<Receiver<Change>, StoreError> {
        validate_prefix(prefix)?;
        let (sender, receiver) = channel();
        match self.state() {
            State::Ready => lock(&self.watchers).push((prefix.to_owned(), sender)),
            State::Quarantined => drop(sender),
            other => return Err(unavailable(other)),
        }
        Ok(receiver)
    }

    /// Takes a read connection from the pool, or opens one. `None` means the namespace is
    /// quarantined, so the read is served as empty.
    fn reader(&self) -> Result<Option<Pooled<'_>>, StoreError> {
        match self.state() {
            State::Ready => {}
            State::Quarantined => return Ok(None),
            other => return Err(unavailable(other)),
        }
        let conn = match lock(&self.readers).pop() {
            Some(conn) => conn,
            None => open_reader(&self.path)?,
        };
        Ok(Some(Pooled {
            kv: self,
            conn: Some(conn),
        }))
    }
}

/// A read connection on loan from the pool. Dropping it returns the connection, unless the
/// namespace has closed or the pool is full.
struct Pooled<'a> {
    kv: &'a Kv,
    conn: Option<Connection>,
}

impl std::ops::Deref for Pooled<'_> {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        self.conn.as_ref().expect("a pooled connection is present until drop")
    }
}

impl Drop for Pooled<'_> {
    fn drop(&mut self) {
        let Some(conn) = self.conn.take() else {
            return;
        };
        if self.kv.state() != State::Ready {
            return;
        }
        let mut readers = lock(&self.kv.readers);
        if readers.len() < READER_POOL {
            readers.push(conn);
        }
    }
}

/// The stored revision and size of `key`, or `(0, None)` when it is absent. The size is the
/// key bytes plus the encoded value bytes, the same figure the quota counts.
fn current(tx: &Transaction<'_>, key: &str) -> Result<(u64, Option<u64>), StoreError> {
    let row: Option<(i64, i64)> = tx
        .query_row("SELECT rev, length(value) FROM items WHERE key = ?1", [key], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .optional()
        .map_err(map_sqlite)?;
    Ok(match row {
        Some((revision, len)) => (revision as u64, Some(key.len() as u64 + len as u64)),
        None => (0, None),
    })
}

/// Refuses a write whose `if_revision` does not match what is stored. An absent key is revision
/// 0, so `Some(0)` creates a key only when it does not exist.
fn check_revision(key: &str, expected: Option<u64>, found: u64) -> Result<(), StoreError> {
    match expected {
        Some(expected) if expected != found => Err(StoreError::RevisionConflict {
            key: key.to_owned(),
            expected,
            found,
        }),
        _ => Ok(()),
    }
}

/// The bytes the namespace holds now, from the `meta` counter kept in step with the rows.
fn bytes_used(tx: &Transaction<'_>) -> Result<u64, StoreError> {
    let value: i64 = tx
        .query_row("SELECT value FROM meta WHERE name = 'bytes'", [], |row| row.get(0))
        .map_err(map_sqlite)?;
    Ok(value as u64)
}

fn set_bytes(tx: &Transaction<'_>, bytes: u64) -> Result<(), StoreError> {
    tx.execute("UPDATE meta SET value = ?1 WHERE name = 'bytes'", [bytes as i64])
        .map_err(map_sqlite)?;
    Ok(())
}

/// Takes the next namespace revision and stores it, so revisions never repeat.
fn next_revision(tx: &Transaction<'_>) -> Result<u64, StoreError> {
    let revision: i64 = tx
        .query_row(
            "UPDATE meta SET value = value + 1 WHERE name = 'revision' RETURNING value",
            [],
            |row| row.get(0),
        )
        .map_err(map_sqlite)?;
    Ok(revision as u64)
}

/// The `number` and `text` columns for a value. `number` keeps numbers as numbers so a range
/// compares them; `text` keeps text so contains can search it. Every other kind stores NULL.
fn columns(value: &Value) -> (SqlValue, SqlValue) {
    match value {
        Value::Int(number) => (SqlValue::Integer(*number), SqlValue::Null),
        Value::Uint(number) if *number <= i64::MAX as u64 => (SqlValue::Integer(*number as i64), SqlValue::Null),
        Value::Double(number) => (SqlValue::Real(*number), SqlValue::Null),
        Value::Text(text) => (SqlValue::Null, SqlValue::Text(text.clone())),
        _ => (SqlValue::Null, SqlValue::Null),
    }
}

/// A row before its value is decoded: the key, revision, write time and encoded bytes.
struct RawEntry {
    key: String,
    revision: u64,
    updated_ms: i64,
    bytes: Vec<u8>,
}

fn raw_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawEntry> {
    Ok(RawEntry {
        key: row.get(0)?,
        revision: row.get::<_, i64>(1)? as u64,
        updated_ms: row.get(2)?,
        bytes: row.get(3)?,
    })
}

fn decode_entry(raw: RawEntry) -> Result<Entry, StoreError> {
    let value = decode(&raw.bytes)?;
    Ok(Entry {
        key: raw.key,
        value,
        revision: raw.revision,
        updated_ms: raw.updated_ms,
    })
}

/// Opens the file, sets it up for use, and checks it. Returns [`StoreError::Corrupt`] for a file
/// that is not a database or fails `quick_check`, which is what triggers quarantine.
fn open_file(path: &Path, writable: bool) -> Result<Connection, StoreError> {
    let flags = if writable {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let mut conn = Connection::open_with_flags(path, flags).map_err(map_sqlite)?;
    conn.busy_timeout(BUSY_TIMEOUT).map_err(map_sqlite)?;

    let check: String = conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(map_sqlite)?;
    if check != "ok" {
        return Err(StoreError::Corrupt(check));
    }

    if writable {
        // WAL lets readers run beside the writer. NORMAL sync is safe in WAL: a crash can lose
        // the last commit but never corrupts the file.
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))
            .map_err(map_sqlite)?;
        conn.pragma_update(None, "synchronous", "NORMAL").map_err(map_sqlite)?;
        schema::migrate(&mut conn, STEPS)?;
    } else {
        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(map_sqlite)?;
        if version != schema::latest() {
            return Err(StoreError::Migration {
                version,
                message: "a read-only store needs the file at the latest version".to_owned(),
            });
        }
    }
    Ok(conn)
}

/// Opens a read connection: `query_only` so it cannot write even by mistake.
fn open_reader(path: &Path) -> Result<Connection, StoreError> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(map_sqlite)?;
    conn.busy_timeout(BUSY_TIMEOUT).map_err(map_sqlite)?;
    conn.pragma_update(None, "query_only", true).map_err(map_sqlite)?;
    Ok(conn)
}

/// Moves a damaged file and its WAL sidecars aside as `<name>.corrupt-<UTC time>`, and logs it.
/// Never blocks: a rename that fails is logged and the namespace is still quarantined.
fn quarantine(path: &Path, reason: &str) {
    let stamp = utc_stamp(now_ms() / 1000);
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let target = path.with_file_name(format!("{stem}.corrupt-{stamp}"));
    log::error!("quarantining {}: {reason}", path.display());
    for suffix in ["", "-wal", "-shm"] {
        let from = PathBuf::from(format!("{}{suffix}", path.display()));
        if !from.exists() {
            continue;
        }
        let to = PathBuf::from(format!("{}{suffix}", target.display()));
        if let Err(error) = std::fs::rename(&from, &to) {
            log::error!("cannot move {} aside: {error}", from.display());
        }
    }
}

/// The milliseconds since the Unix epoch, or 0 when the clock is before it.
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Formats Unix seconds as a compact UTC stamp, `YYYYMMDDTHHMMSSZ`, for quarantine names.
///
/// The date comes from the civil-day algorithm, which is exact for the proleptic Gregorian
/// calendar, so no time crate is needed for one file name.
fn utc_stamp(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Shift the epoch to 0000-03-01 so leap days fall at the end of each year.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// The refusal for a verb on a namespace that is not ready.
fn unavailable(state: State) -> StoreError {
    StoreError::StoreUnavailable(format!("the namespace is {}", state.as_str()))
}

/// Maps a SQLite error to the store's named errors. A full disk and a damaged file have their own
/// codes; every other failure is `database` with SQLite's message.
pub(crate) fn map_sqlite(error: rusqlite::Error) -> StoreError {
    let message = error.to_string();
    match &error {
        rusqlite::Error::SqliteFailure(failure, _) => match failure.code {
            ErrorCode::DiskFull => StoreError::DiskFull(message),
            ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt => StoreError::Corrupt(message),
            _ => StoreError::Database(message),
        },
        _ => StoreError::Database(message),
    }
}

/// Locks `mutex`, ignoring poison, as [`crate::lock`] does.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
