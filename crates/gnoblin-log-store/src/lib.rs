//! The database half of the log: one row per event, written by a thread that batches inserts
//! so that the call which logs never waits on the disk.

use std::fmt::{self, Debug, Write as _};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gnoblin_log::__tracing::field::{Field, Visit};
use gnoblin_log::__tracing::{Event, Subscriber};
use rusqlite::{Connection, params};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

/// How many records may wait for the writer before new ones are counted as dropped.
const QUEUE_CAPACITY: usize = 4096;

/// The most records one transaction writes.
const BATCH_LIMIT: usize = 256;

/// How long a flush waits for the writer before giving up.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

/// How long a write waits for another process holding the database.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS log_records (
        id INTEGER PRIMARY KEY,
        time_ms INTEGER NOT NULL,
        level TEXT NOT NULL,
        component TEXT NOT NULL,
        target TEXT NOT NULL,
        message TEXT NOT NULL,
        fields TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS log_records_time ON log_records (time_ms);
";

const INSERT: &str = "INSERT INTO log_records (time_ms, level, component, target, message, fields) \
                      VALUES (?1, ?2, ?3, ?4, ?5, ?6)";

/// One logged event, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogRecord {
    /// Milliseconds since the Unix epoch.
    pub(crate) time_ms: i64,
    pub(crate) level: &'static str,
    /// The process role that logged the event, stored in the `component` column.
    pub(crate) role: &'static str,
    pub(crate) target: String,
    pub(crate) message: String,
    /// The event's fields other than `message`, as `name=value` pairs.
    pub(crate) fields: String,
}

/// What the layer sends to the writer thread.
pub(crate) enum Message {
    Record(LogRecord),
    /// Asks the writer to acknowledge once every earlier message is written.
    Flush(SyncSender<()>),
}

/// The writer's queue, and the count of records it could not take.
#[derive(Clone)]
pub struct Sink {
    sender: SyncSender<Message>,
    dropped: Arc<AtomicU64>,
}

impl Sink {
    /// Waits, for a bounded time, until the writer has stored everything logged so far.
    pub fn flush(&self) {
        let (acknowledge, acknowledged) = mpsc::sync_channel(1);
        if self.sender.send(Message::Flush(acknowledge)).is_ok() {
            let _ = acknowledged.recv_timeout(FLUSH_TIMEOUT);
        }
    }
}

/// Creates `directory` and any missing parents, private to the user. The configuration reader
/// refuses a state directory that others can open, so the log must not create it more open.
fn create_private_directory(directory: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    fs::DirBuilder::new().recursive(true).mode(0o700).create(directory)
}

/// Opens the database at `path`, starts its writer thread and returns the queue into it.
pub fn open(path: &Path, role: &'static str) -> Result<Sink, String> {
    if let Some(parent) = path.parent() {
        create_private_directory(parent).map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    let mut connection = Connection::open(path).map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    prepare(&mut connection).map_err(|error| format!("cannot prepare {}: {error}", path.display()))?;

    let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
    let dropped = Arc::new(AtomicU64::new(0));
    let writer_dropped = Arc::clone(&dropped);
    thread::Builder::new()
        .name("gnoblin-log".to_owned())
        .spawn(move || write_loop(connection, receiver, role, &writer_dropped))
        .map_err(|error| format!("cannot start the log writer: {error}"))?;
    Ok(Sink { sender, dropped })
}

/// The layer that turns each event into a [`LogRecord`] and queues it without blocking.
pub struct DatabaseLayer {
    sink: Sink,
    role: &'static str,
}

impl DatabaseLayer {
    pub fn new(sink: Sink, role: &'static str) -> Self {
        Self { sink, role }
    }
}

impl<S> Layer<S> for DatabaseLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let metadata = event.metadata();
        let mut fields = FieldCollector::default();
        event.record(&mut fields);
        let record = LogRecord {
            time_ms: now_ms(),
            level: metadata.level().as_str(),
            role: self.role,
            target: metadata.target().to_owned(),
            message: fields.message,
            fields: fields.rest,
        };
        if let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) =
            self.sink.sender.try_send(Message::Record(record))
        {
            self.sink.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Collects an event's `message` and its other fields.
#[derive(Default)]
struct FieldCollector {
    message: String,
    rest: String,
}

impl FieldCollector {
    fn push_field(&mut self, name: &str, value: fmt::Arguments<'_>) {
        if !self.rest.is_empty() {
            self.rest.push(' ');
        }
        let _ = write!(self.rest, "{name}={value}");
    }
}

impl Visit for FieldCollector {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            self.push_field(field.name(), format_args!("{value:?}"));
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            self.push_field(field.name(), format_args!("{value:?}"));
        }
    }
}

/// Stores records in batches until its queue closes, and acknowledges each flush after the
/// batch before it is written.
fn write_loop(mut connection: Connection, receiver: Receiver<Message>, role: &'static str, dropped: &AtomicU64) {
    let mut records = Vec::with_capacity(BATCH_LIMIT);
    let mut acknowledgements = Vec::new();
    while let Ok(first) = receiver.recv() {
        for message in std::iter::once(first).chain(receiver.try_iter().take(BATCH_LIMIT - 1)) {
            match message {
                Message::Record(record) => records.push(record),
                Message::Flush(acknowledge) => acknowledgements.push(acknowledge),
            }
        }
        let lost = dropped.swap(0, Ordering::Relaxed);
        if lost > 0 {
            records.push(dropped_record(role, lost));
        }
        if !records.is_empty()
            && let Err(error) = insert_batch(&mut connection, &records)
        {
            eprintln!("gnoblin-log: cannot store {} records: {error}", records.len());
        }
        records.clear();
        for acknowledge in acknowledgements.drain(..) {
            let _ = acknowledge.send(());
        }
    }
}

/// The record that says how many events the full queue discarded.
fn dropped_record(role: &'static str, lost: u64) -> LogRecord {
    LogRecord {
        time_ms: now_ms(),
        level: "WARN",
        role,
        target: module_path!().to_owned(),
        message: format!("dropped {lost} log records because the queue was full"),
        fields: String::new(),
    }
}

/// Sets the connection's busy wait, its journal mode and its schema.
fn prepare(connection: &mut Connection) -> rusqlite::Result<()> {
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.execute_batch(SCHEMA)
}

/// Writes `records` in one transaction.
fn insert_batch(connection: &mut Connection, records: &[LogRecord]) -> rusqlite::Result<()> {
    let transaction = connection.transaction()?;
    {
        let mut statement = transaction.prepare_cached(INSERT)?;
        for record in records {
            statement.execute(params![
                record.time_ms,
                record.level,
                record.role,
                record.target,
                record.message,
                record.fields,
            ])?;
        }
    }
    transaction.commit()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(message: &str, fields: &str) -> LogRecord {
        LogRecord {
            time_ms: 1_000,
            level: "INFO",
            role: "compositor",
            target: "gnoblin::main".to_owned(),
            message: message.to_owned(),
            fields: fields.to_owned(),
        }
    }

    fn database() -> Connection {
        let mut connection = Connection::open_in_memory().expect("an in-memory database");
        prepare(&mut connection).expect("the schema");
        connection
    }

    #[test]
    fn a_batch_is_stored_in_order_with_its_fields() {
        let mut connection = database();
        let batch = [record("started", "role=\"compositor\""), record("stopped", "")];

        insert_batch(&mut connection, &batch).expect("the batch is written");

        let stored: Vec<(String, String, String)> = connection
            .prepare("SELECT component, message, fields FROM log_records ORDER BY id")
            .expect("the query")
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("the rows")
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            stored,
            vec![
                (
                    "compositor".to_owned(),
                    "started".to_owned(),
                    "role=\"compositor\"".to_owned()
                ),
                ("compositor".to_owned(), "stopped".to_owned(), String::new()),
            ]
        );
    }

    #[test]
    fn a_dropped_count_is_stored_as_a_warning() {
        let mut connection = database();
        insert_batch(&mut connection, &[dropped_record("services", 7)]).expect("the batch is written");

        let (level, message): (String, String) = connection
            .query_row("SELECT level, message FROM log_records", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .expect("the row");
        assert_eq!(level, "WARN");
        assert_eq!(message, "dropped 7 log records because the queue was full");
    }

    #[test]
    fn the_schema_can_be_prepared_twice() {
        let mut connection = database();
        prepare(&mut connection).expect("a second process prepares the same file");
    }
}
