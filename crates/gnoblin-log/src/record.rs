//! The record a logged event becomes, and the sink that receives batches of them.
//!
//! The layer turns each event into a [`LogRecord`] and queues it without blocking. A writer
//! thread drains the queue in batches and hands each batch to a [`RecordSink`]. The sink does the
//! storing, so this crate never depends on a storage crate: `gnoblin-store` implements the sink,
//! and the binaries pass it to [`crate::init::init`] through [`SinkHandle`].

use std::fmt;
use std::sync::Arc;

/// One logged event, as a sink stores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRecord {
    /// Milliseconds since the Unix epoch, taken when the event was logged.
    pub time_ms: i64,
    /// The level name the writer uses: `ERROR`, `WARN`, `INFO`, `DEBUG` or `TRACE`.
    pub level: &'static str,
    /// The component that logged the event, as named by the `*_in!` call, or `ext` for a
    /// dependency's event.
    pub component: String,
    /// The process role that logged the event, such as `compositor` or `portal`.
    pub role: &'static str,
    /// The event's target, which is `gnoblin::<component>` for a gnoblin event.
    pub target: String,
    /// The event's message, with secrets scrubbed by the layer.
    pub message: String,
    /// The event's fields other than `message`, as `name=value` pairs.
    pub fields: String,
}

/// Where the writer sends the batches of records it has taken from the queue.
///
/// A sink runs on the writer thread, never on the thread that logs, so a slow sink delays only
/// the records behind it in the queue. When the queue is full the layer drops records and counts
/// them; the sink never sees a drop, only the warning the writer adds for it.
///
/// A sink reports its own failures, because the writer cannot act on them. It must not panic.
pub trait RecordSink: Send + Sync {
    /// Stores `records`, oldest first. Called once per batch, with at least one record.
    fn write_batch(&self, records: &[LogRecord]);
}

/// A boxed [`RecordSink`] that [`crate::init::LogOptions`] can hold and compare.
///
/// Two handles are equal when they share the same sink, which is what an options comparison
/// needs: a clone of the handle is the same sink, and a second sink is a different one.
#[derive(Clone)]
pub struct SinkHandle(Arc<dyn RecordSink>);

impl SinkHandle {
    /// Wraps `sink` so the options can hold it.
    pub fn new(sink: Box<dyn RecordSink>) -> Self {
        Self(Arc::from(sink))
    }

    #[cfg(feature = "init")]
    /// The shared sink, for the writer thread to own.
    pub(crate) fn shared(&self) -> Arc<dyn RecordSink> {
        Arc::clone(&self.0)
    }
}

impl fmt::Debug for SinkHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("SinkHandle").finish_non_exhaustive()
    }
}

impl PartialEq for SinkHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for SinkHandle {}
