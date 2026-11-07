//! The typed store: one SQLite file per namespace, behind a query API that takes no SQL.
//!
//! A [`Store`] is a directory of namespace files. A [`Namespace`] is one file, opened once and
//! shared: it owns a single writer connection, a small pool of read-only connections, and the
//! lifecycle [`State`]. Consumers see only the verbs here. No query string is parsed as SQL, and
//! no connection leaves this crate.
//!
//! The store never blocks start. A file that is damaged is moved aside by [`Store::namespace`]
//! and the namespace comes back [`State::Quarantined`]: its reads are empty and its writes are
//! refused with `store-unavailable`. The store owner's code defaults then apply, and the cause is
//! logged through the `gnoblin-log` facade.
//!
//! Modules: `name` checks namespace names and keys, `encode` is the value encoding, `schema` owns
//! the migrations, `query` compiles the typed [`Query`] to bound SQL, `kv` is the in-process
//! provider that does the work, and `logs` is the `log` kind: an append-only table of records
//! that a [`gnoblin_log::record::RecordSink`] writes into.

mod encode;
mod kv;
mod logs;
mod name;
mod query;
mod schema;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gnoblin_model::Value;

pub use kv::{Change, Entry, Op, Outcome};
pub use logs::{GroupBy, LogQuery, Logs, Retention, StoredRecord};
pub use query::{Order, Query};

use kv::Kv;

gnoblin_log::logger!(log = "store");

/// The default per-namespace quota: 16 MiB across every key and value in one file.
pub const DEFAULT_QUOTA: u64 = 16 * 1024 * 1024;

/// The default per-value limit: 64 KiB of encoded value.
pub const DEFAULT_VALUE_LIMIT: usize = 64 * 1024;

/// Settings a store opens with. The defaults are the limits in design/store.md; a host can
/// lower or raise them when it constructs the store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// The most bytes one namespace may hold, counting each key and its encoded value.
    pub quota: u64,
    /// The most encoded bytes one value may take.
    pub value_limit: usize,
    /// Whether writes are allowed. When `false`, the namespace file must already exist and be at
    /// the latest schema, it is opened read-only, and every write returns `read-only`.
    pub writable: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            quota: DEFAULT_QUOTA,
            value_limit: DEFAULT_VALUE_LIMIT,
            writable: true,
        }
    }
}

/// The lifecycle of one namespace. The table in [`State::can_move_to`] is the only way between
/// states; any other move is a bug in this crate and is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// The file is being opened, checked and migrated. No caller sees this state.
    Opening,
    /// The file is open and every verb is served.
    Ready,
    /// The file was damaged and moved aside. Reads are empty and writes are refused.
    Quarantined,
    /// The store is shutting down. Every verb is refused.
    Closing,
}

impl State {
    /// Whether the lifecycle may move from `self` to `next`.
    ///
    /// `Opening` leads to `Ready`, `Quarantined` or `Closing` (an open that failed for another
    /// reason). `Ready` and `Quarantined` lead only to `Closing`, and `Closing` is final.
    pub const fn can_move_to(self, next: State) -> bool {
        matches!(
            (self, next),
            (State::Opening, State::Ready | State::Quarantined | State::Closing)
                | (State::Ready | State::Quarantined, State::Closing)
        )
    }

    /// The state's name in logs and in error messages.
    pub const fn as_str(self) -> &'static str {
        match self {
            State::Opening => "opening",
            State::Ready => "ready",
            State::Quarantined => "quarantined",
            State::Closing => "closing",
        }
    }
}

/// Every error a store verb returns. [`StoreError::code`] gives the stable name a front end shows.
#[derive(Debug)]
pub enum StoreError {
    /// A namespace name is not 1 to 64 bytes of `a-z`, `0-9`, `-` and `_`.
    InvalidName(String),
    /// A key or prefix breaks the key rules: 1 to 256 bytes, no NUL, no edge `/`.
    InvalidKey(String),
    /// A query names an impossible condition, such as an empty contains-text.
    InvalidQuery(String),
    /// The value has a type this store does not keep, such as an object id or a NaN.
    ValueNotStorable(String),
    /// The encoded value is over the limit. `size` is what it would take.
    ValueTooLarge { size: usize, limit: usize },
    /// An `if_revision` did not match the stored revision. `found` is 0 when the key is absent.
    RevisionConflict { key: String, expected: u64, found: u64 },
    /// The write would take the namespace past its quota. `current` is the total it would reach.
    QuotaExceeded { current: u64, limit: u64 },
    /// The namespace was opened with `writable = false`.
    ReadOnly,
    /// The namespace cannot serve the verb: it is quarantined, closing, or never opened.
    StoreUnavailable(String),
    /// SQLite reported a full disk. Reads continue and one warning is logged.
    DiskFull(String),
    /// A migration step failed, or the file came from a newer store. The file keeps the version
    /// of the last step that committed.
    Migration { version: u32, message: String },
    /// A stored value does not decode. The row is damaged and is never guessed at.
    Undecodable(String),
    /// The file is not a database or fails `quick_check`. Opening it quarantines the file.
    Corrupt(String),
    /// Any other SQLite failure.
    Database(String),
    /// A bug in this crate, such as a refused lifecycle move.
    Internal(String),
}

impl StoreError {
    /// The stable name of the error, as design/store.md lists it where one is given.
    pub fn code(&self) -> &'static str {
        match self {
            StoreError::InvalidName(_) => "invalid-name",
            StoreError::InvalidKey(_) => "invalid-key",
            StoreError::InvalidQuery(_) => "invalid-query",
            StoreError::ValueNotStorable(_) => "value-not-storable",
            StoreError::ValueTooLarge { .. } => "value-too-large",
            StoreError::RevisionConflict { .. } => "revision-conflict",
            StoreError::QuotaExceeded { .. } => "quota-exceeded",
            StoreError::ReadOnly => "read-only",
            StoreError::StoreUnavailable(_) => "store-unavailable",
            StoreError::DiskFull(_) => "disk-full",
            StoreError::Migration { .. } => "migration-failed",
            StoreError::Undecodable(_) => "undecodable",
            StoreError::Corrupt(_) => "corrupt",
            StoreError::Database(_) => "database",
            StoreError::Internal(_) => "internal",
        }
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::InvalidName(reason)
            | StoreError::InvalidKey(reason)
            | StoreError::InvalidQuery(reason)
            | StoreError::ValueNotStorable(reason)
            | StoreError::StoreUnavailable(reason)
            | StoreError::DiskFull(reason)
            | StoreError::Undecodable(reason)
            | StoreError::Corrupt(reason)
            | StoreError::Database(reason)
            | StoreError::Internal(reason) => write!(f, "{}: {reason}", self.code()),
            StoreError::ValueTooLarge { size, limit } => {
                write!(f, "value-too-large: {size} bytes, the limit is {limit}")
            }
            StoreError::RevisionConflict { key, expected, found } => {
                write!(
                    f,
                    "revision-conflict: {key} is at revision {found}, expected {expected}"
                )
            }
            StoreError::QuotaExceeded { current, limit } => {
                write!(
                    f,
                    "quota-exceeded: the store would hold {current} bytes, the quota is {limit}"
                )
            }
            StoreError::ReadOnly => write!(f, "read-only: the store was opened with writable = false"),
            StoreError::Migration { version, message } => {
                write!(f, "migration-failed at version {version}: {message}")
            }
        }
    }
}

impl std::error::Error for StoreError {}

/// A directory of namespace files. Opening a store creates the directory and nothing else;
/// each namespace file is created the first time it is opened.
pub struct Store {
    root: PathBuf,
    options: Options,
    namespaces: Mutex<HashMap<String, Arc<Kv>>>,
}

impl Store {
    /// Opens the store at `root` with the default [`Options`].
    ///
    /// # Errors
    ///
    /// `store-unavailable` when the directory cannot be created.
    pub fn open(root: impl AsRef<Path>) -> Result<Store, StoreError> {
        Self::with_options(root, Options::default())
    }

    /// Opens the store at `root` with `options`, which apply to every namespace in it.
    ///
    /// The directory is created with mode `0700`, because the files in it hold user state.
    ///
    /// # Errors
    ///
    /// `store-unavailable` when the directory cannot be created.
    pub fn with_options(root: impl AsRef<Path>, options: Options) -> Result<Store, StoreError> {
        use std::os::unix::fs::DirBuilderExt;

        let root = root.as_ref().to_path_buf();
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&root)
            .map_err(|error| StoreError::StoreUnavailable(format!("cannot create {}: {error}", root.display())))?;
        Ok(Store {
            root,
            options,
            namespaces: Mutex::new(HashMap::new()),
        })
    }

    /// The directory the namespace files live in.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the namespace `name`, opening its file on first use and sharing the open file
    /// after that.
    ///
    /// A damaged file does not fail this call. The namespace comes back quarantined, and its
    /// [`Namespace::state`] says so.
    ///
    /// # Errors
    ///
    /// `invalid-name` for a bad name. `store-unavailable` when the file cannot be opened, such as
    /// a read-only store with no file. `migration-failed` when a migration step fails.
    pub fn namespace(&self, name: &str) -> Result<Namespace, StoreError> {
        name::validate_name(name)?;
        if name == logs::RESERVED_NAME {
            return Err(StoreError::InvalidName(format!("{name} belongs to the log kind")));
        }
        let mut namespaces = lock(&self.namespaces);
        let kv = match namespaces.get(name) {
            Some(kv) => Arc::clone(kv),
            None => {
                let path = self.root.join(format!("{name}.sqlite3"));
                let kv = Arc::new(Kv::open(path, self.options)?);
                namespaces.insert(name.to_owned(), Arc::clone(&kv));
                kv
            }
        };
        Ok(Namespace {
            name: name.to_owned(),
            kv,
        })
    }

    /// Opens the `log` kind: the append-only record table at `log.sqlite3` in this store's
    /// directory. Hand the result to the logging layer as its record sink, and keep it for the
    /// search verbs.
    ///
    /// `retention` says how much history to keep. A writable open applies it at once.
    ///
    /// # Errors
    ///
    /// `store-unavailable` when the file is missing and the store is read-only. `migration-failed`
    /// when a migration step fails or the file came from a newer store. A damaged file is moved
    /// aside and a fresh one is opened, as for namespaces.
    pub fn logs(&self, retention: Retention) -> Result<Logs, StoreError> {
        Logs::open(&self.root.join(logs::FILE_NAME), self.options.writable, retention)
    }

    /// Moves every open namespace to `closing`. Their verbs are refused from then on, and their
    /// connections close when the last handle to them is dropped.
    pub fn close(&self) {
        for kv in lock(&self.namespaces).values() {
            kv.close();
        }
    }
}

/// One namespace: a handle to its open file. Clones share the file.
#[derive(Clone)]
pub struct Namespace {
    name: String,
    kv: Arc<Kv>,
}

impl Namespace {
    /// The namespace's name, as it was passed to [`Store::namespace`].
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The current lifecycle state.
    pub fn state(&self) -> State {
        self.kv.state()
    }

    /// Whether writes are allowed on this namespace.
    pub fn writable(&self) -> bool {
        self.kv.writable()
    }

    /// Stores `value` under `key` and returns its revision. Replaces any previous value.
    ///
    /// # Errors
    ///
    /// Any error of [`Namespace::batch`] for one put.
    pub fn put(&self, key: &str, value: Value) -> Result<u64, StoreError> {
        match self.batch(&[Op::put(key, value)])?.as_slice() {
            [Outcome::Put { revision }] => Ok(*revision),
            other => Err(StoreError::Internal(format!("a put returned {other:?}"))),
        }
    }

    /// Removes the value under `key`. Returns whether a value was there to remove.
    ///
    /// # Errors
    ///
    /// Any error of [`Namespace::batch`] for one delete.
    pub fn delete(&self, key: &str) -> Result<bool, StoreError> {
        match self.batch(&[Op::delete(key)])?.as_slice() {
            [Outcome::Delete { removed }] => Ok(*removed),
            other => Err(StoreError::Internal(format!("a delete returned {other:?}"))),
        }
    }

    /// Runs `ops` in one transaction. Every op applies, or none does: a conflict, a quota breach
    /// or a disk error rolls the whole batch back. The outcomes come back in op order.
    ///
    /// # Errors
    ///
    /// The first refusal, with its code: `invalid-key`, `value-not-storable`, `value-too-large`,
    /// `revision-conflict`, `quota-exceeded`, `read-only`, `disk-full` or `store-unavailable`.
    pub fn batch(&self, ops: &[Op]) -> Result<Vec<Outcome>, StoreError> {
        self.kv.apply(ops)
    }

    /// Reads the value and revision under `key`, or `None` when there is none.
    ///
    /// # Errors
    ///
    /// `invalid-key` for a bad key, `undecodable` for a damaged row, `store-unavailable` when
    /// the namespace is closing.
    pub fn get(&self, key: &str) -> Result<Option<Entry>, StoreError> {
        self.kv.get(key)
    }

    /// Lists keys under `prefix` on whole segments, in key order. Pass the last key of one page
    /// as `after` to get the next. `limit` is clamped to 1 to 1000.
    ///
    /// # Errors
    ///
    /// `invalid-key` for a bad prefix or cursor.
    pub fn list(&self, prefix: &str, after: Option<&str>, limit: usize) -> Result<Vec<String>, StoreError> {
        self.kv.list(prefix, after, limit)
    }

    /// Finds the values that match `query`, in the query's order, with its limit and offset.
    ///
    /// # Errors
    ///
    /// `invalid-key` or `invalid-query` for a bad condition, `undecodable` for a damaged row.
    pub fn search(&self, query: &Query) -> Result<Vec<Entry>, StoreError> {
        self.kv.search(query)
    }

    /// Counts the values that match `query`. The query's limit and offset do not apply.
    ///
    /// # Errors
    ///
    /// As [`Namespace::search`], for the condition.
    pub fn count(&self, query: &Query) -> Result<u64, StoreError> {
        self.kv.count(query)
    }

    /// Counts the matching keys by their first `segments` segments, in key order. A key with
    /// fewer segments groups under its whole key. The query's limit and offset do not apply.
    ///
    /// # Errors
    ///
    /// `invalid-query` when `segments` is 0, and as [`Namespace::search`] for the condition.
    pub fn group_by_count(&self, query: &Query, segments: usize) -> Result<Vec<(String, u64)>, StoreError> {
        self.kv.group_by_count(query, segments)
    }

    /// Streams the changes under `prefix`, one per key per commit, in commit order.
    ///
    /// Changes come over an unbounded `std::sync::mpsc` channel and are sent while the writer
    /// lock is held, so commit order is the delivery order. A callback would run under that
    /// lock and could not write back, so it is not offered. Dropping the receiver ends the
    /// subscription.
    ///
    /// # Errors
    ///
    /// `invalid-key` for a bad prefix.
    pub fn watch(&self, prefix: &str) -> Result<Receiver<Change>, StoreError> {
        self.kv.watch(prefix)
    }
}

/// The default store directory: `$XDG_STATE_HOME/gnoblin/store`, or `$HOME/.local/state/...`
/// when `XDG_STATE_HOME` is unset or relative. `None` when neither is set.
pub fn default_root(lookup: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let non_empty = |value: Option<String>| value.filter(|text| !text.is_empty());
    let state_home = match non_empty(lookup("XDG_STATE_HOME")) {
        Some(path) if Path::new(&path).is_absolute() => PathBuf::from(path),
        _ => PathBuf::from(non_empty(lookup("HOME"))?).join(".local/state"),
    };
    Some(state_home.join("gnoblin").join("store"))
}

/// Locks `mutex`, ignoring poison. A panic elsewhere leaves the maps and connections in a state
/// each caller already checks, so the store keeps serving rather than cascading the panic.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
