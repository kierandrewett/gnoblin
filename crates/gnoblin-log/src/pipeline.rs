//! The queue between the logging call and a [`RecordSink`].
//!
//! The layer turns each event into a [`LogRecord`] and queues it without blocking. When the queue
//! is full the record is dropped and counted, so a slow sink never stalls the code that logs. A
//! writer thread takes the queue in batches of at most [`BATCH_LIMIT`] records and hands each batch
//! to the sink, so a slow sink only delays the records behind it. A flush waits, for a bounded
//! time, until everything logged before it has reached the sink.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::__tracing::{Event, Subscriber};
use crate::event::Logged;
use crate::record::{LogRecord, RecordSink};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

/// How many records may wait for the writer before new ones are counted as dropped.
const QUEUE_CAPACITY: usize = 4096;

/// The most records one batch hands to the sink.
const BATCH_LIMIT: usize = 256;

/// How long a flush waits for the writer before giving up.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

/// The component of the record that says how many events the full queue discarded.
const DROPPED_COMPONENT: &str = "log";

/// What the layer sends to the writer thread.
enum Message {
    Record(LogRecord),
    /// Asks the writer to acknowledge once every earlier message has been handed to the sink.
    Flush(SyncSender<()>),
}

/// The queue into one sink, and the count of records it could not take.
///
/// Clones share the queue, so several layers can feed one writer. The writer stops when the last
/// clone is dropped and the queue closes.
#[derive(Clone)]
pub struct Pipeline {
    sender: SyncSender<Message>,
    dropped: Arc<AtomicU64>,
    /// The process role, which every record from this pipeline carries.
    role: &'static str,
}

impl Pipeline {
    /// Starts a writer thread that hands batches to `sink`. Records carry `role` as the process
    /// that logged them, and so does the record that reports dropped events.
    pub fn start(sink: Arc<dyn RecordSink>, role: &'static str) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        let dropped = Arc::new(AtomicU64::new(0));
        let writer_dropped = Arc::clone(&dropped);
        thread::Builder::new()
            .name("gnoblin-log".to_owned())
            .spawn(move || write_loop(sink.as_ref(), receiver, role, &writer_dropped))?;
        Ok(Self { sender, dropped, role })
    }

    /// Waits, for a bounded time, until the writer has handed everything logged so far to the sink.
    pub fn flush(&self) {
        let (acknowledge, acknowledged) = mpsc::sync_channel(1);
        if self.sender.send(Message::Flush(acknowledge)).is_ok() {
            let _ = acknowledged.recv_timeout(FLUSH_TIMEOUT);
        }
    }
}

/// The layer that turns each event into a [`LogRecord`] and queues it without blocking.
pub struct PipelineLayer {
    pipeline: Pipeline,
}

impl PipelineLayer {
    /// Builds the layer that queues into `pipeline`.
    pub fn new(pipeline: Pipeline) -> Self {
        Self { pipeline }
    }
}

impl<S> Layer<S> for PipelineLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let logged = Logged::from_event(event);
        let fields = logged.fields_line();
        let record = LogRecord {
            time_ms: now_ms(),
            level: event.metadata().level().as_str(),
            key: logged.component.clone(),
            component: logged.component,
            role: self.pipeline.role,
            target: logged.target,
            message: logged.message,
            fields,
        };
        if let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) =
            self.pipeline.sender.try_send(Message::Record(record))
        {
            self.pipeline.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Hands batches to `sink` until the queue closes, and acknowledges each flush after the batch
/// before it has been handed over.
fn write_loop(sink: &dyn RecordSink, receiver: Receiver<Message>, role: &'static str, dropped: &AtomicU64) {
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
        if !records.is_empty() {
            sink.write_batch(&records);
        }
        records.clear();
        for acknowledge in acknowledgements.drain(..) {
            let _ = acknowledge.send(());
        }
    }
}

/// The record that says how many events the full queue discarded.
pub(crate) fn dropped_record(role: &'static str, lost: u64) -> LogRecord {
    LogRecord {
        time_ms: now_ms(),
        level: "WARN",
        key: DROPPED_COMPONENT.to_owned(),
        component: DROPPED_COMPONENT.to_owned(),
        role,
        target: "gnoblin::log".to_owned(),
        message: format!("dropped {lost} log records because the queue was full"),
        fields: String::new(),
    }
}

/// The current time in milliseconds since the Unix epoch, or 0 when the clock is before it.
pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use tracing_subscriber::layer::SubscriberExt;

    /// A sink that keeps every batch it is handed.
    #[derive(Default)]
    struct Recording {
        batches: Mutex<Vec<Vec<LogRecord>>>,
    }

    impl RecordSink for Recording {
        fn write_batch(&self, records: &[LogRecord]) {
            self.batches.lock().expect("the batches").push(records.to_vec());
        }
    }

    /// A sink that waits for `gate` before it keeps anything, so a test can fill the queue.
    struct Gated {
        gate: Arc<Mutex<()>>,
        recording: Recording,
    }

    impl RecordSink for Gated {
        fn write_batch(&self, records: &[LogRecord]) {
            drop(self.gate.lock().expect("the gate"));
            self.recording.write_batch(records);
        }
    }

    /// Runs `body` with a subscriber that queues into `pipeline`, on this thread only.
    fn logging<T>(pipeline: &Pipeline, body: impl FnOnce() -> T) -> T {
        let subscriber = tracing_subscriber::registry().with(PipelineLayer::new(pipeline.clone()));
        crate::__tracing::subscriber::with_default(subscriber, body)
    }

    /// The messages of every record in `recording`, oldest first, with their batch sizes.
    fn messages(recording: &Recording) -> (Vec<String>, Vec<usize>) {
        let batches = recording.batches.lock().expect("the batches");
        let sizes = batches.iter().map(Vec::len).collect();
        let messages = batches.iter().flatten().map(|record| record.message.clone()).collect();
        (messages, sizes)
    }

    #[test]
    fn records_reach_the_sink_in_order() {
        let recording = Arc::new(Recording::default());
        let pipeline = Pipeline::start(recording.clone(), "compositor").expect("the writer starts");

        logging(&pipeline, || {
            crate::__tracing::info!(component = "wayland", "first");
            crate::__tracing::info!(component = "wayland", "second");
            crate::__tracing::info!(component = "wayland", "third");
        });
        pipeline.flush();

        let (stored, _) = messages(&recording);
        assert_eq!(stored, ["first", "second", "third"]);
    }

    #[test]
    fn every_record_carries_the_pipelines_role_and_component() {
        let recording = Arc::new(Recording::default());
        let pipeline = Pipeline::start(recording.clone(), "services").expect("the writer starts");

        logging(&pipeline, || crate::__tracing::info!(component = "bus", "connected"));
        pipeline.flush();

        let batches = recording.batches.lock().expect("the batches");
        let record = &batches[0][0];
        assert_eq!((record.role, record.component.as_str()), ("services", "bus"));
        assert_eq!(record.level, "INFO");
    }

    #[test]
    fn a_full_queue_counts_what_it_drops_and_reports_it_once() {
        let gate = Arc::new(Mutex::new(()));
        let sink = Arc::new(Gated {
            gate: Arc::clone(&gate),
            recording: Recording::default(),
        });
        let pipeline = Pipeline::start(sink.clone(), "compositor").expect("the writer starts");

        // The writer blocks on the first batch it hands over, so the queue fills and the rest drop.
        let held = gate.lock().expect("the gate");
        let logged: usize = 5_000;
        logging(&pipeline, || {
            for index in 0..logged {
                crate::__tracing::info!(component = "wayland", "event {index}");
            }
        });
        drop(held);
        pipeline.flush();

        let (stored, _) = messages(&sink.recording);
        let events = stored.iter().filter(|message| message.starts_with("event ")).count();
        let lost: usize = stored
            .iter()
            .filter_map(|message| message.strip_prefix("dropped "))
            .filter_map(|rest| rest.split(' ').next()?.parse::<usize>().ok())
            .sum();
        assert!(lost > 0, "the queue cannot hold {logged} records, so some were dropped");
        assert_eq!(
            events + lost,
            logged,
            "every event is either stored or counted as dropped"
        );
        assert_eq!(
            stored.iter().filter(|message| message.starts_with("dropped ")).count(),
            1,
            "the count is reported once, after the queue drains"
        );
    }
}
