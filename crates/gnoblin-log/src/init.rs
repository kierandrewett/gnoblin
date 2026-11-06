//! Installs the gnoblin log for one process.
//!
//! [`init`] installs one `tracing` subscriber. Its console layer follows the resolved
//! [`ConsoleOutput`]: the native journal (the `journal` module), the `<N>` stream format, the pretty or
//! plain line format, JSON lines, or nothing. Its database layer, the `store` module, stores each
//! event as a row in a SQLite file, so that a session can be inspected after the fact; it is on
//! in every mode. All layers share one filter, taken from `GNOBLIN_LOG`, and the database location
//! comes from `GNOBLIN_LOG_DB`; see [`LogOptions`].
//!
//! Only the binaries use this module: they enable the `init` feature. The crates that log use the
//! macros of the facade.

mod options;

use std::io::IsTerminal as _;
use std::panic;

use crate::ProcessRole;
use crate::console::{ConsoleFormat, JsonFormat, StreamFormat};
use crate::journal::{JournalLayer, SOCKET_PATH};
use crate::store::{DatabaseLayer, Sink};
use tracing_subscriber::Registry;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt::format::DefaultFields;
use tracing_subscriber::fmt::{self, FormatEvent};
use tracing_subscriber::layer::{Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;

/// The retention policy the database is opened with, re-exported beside the options.
pub use crate::store::Retention;
/// The options a process sets up logging with, and the environment variables that name them.
pub use options::{
    Console, ConsoleOutput, DATABASE_VARIABLE, FILTER_VARIABLE, FORMAT_VARIABLE, LogOptions, LogSettings, Probe,
};

/// Keeps the database writer alive. Dropping it waits, for a bounded time, for pending
/// records to reach the database, so a process should hold it until it is done logging.
#[must_use = "dropping the guard flushes the log database immediately"]
pub struct LogGuard {
    sink: Option<Sink>,
}

impl Drop for LogGuard {
    fn drop(&mut self) {
        if let Some(sink) = &self.sink {
            sink.flush();
        }
    }
}

/// Installs the process-wide subscriber for `role`, with `options`.
///
/// Problems with the configuration do not stop the process: an invalid filter falls back to
/// `info`, an unknown format is ignored, a database that cannot be opened leaves the console
/// working, and a journal that cannot be reached writes to stderr instead. Each problem is
/// reported as a warning once the subscriber is installed. A panic is logged as an error
/// before the previous panic hook runs.
///
/// Calling this after another subscriber is installed leaves that subscriber in place.
pub fn init(role: ProcessRole, options: &LogOptions) -> LogGuard {
    let role = role.name();
    let mut problems = options.problems.clone();

    let filter = match EnvFilter::try_new(&options.filter) {
        Ok(filter) => filter,
        Err(error) => {
            problems.push(format!("ignoring {FILTER_VARIABLE}={:?}: {error}", options.filter));
            EnvFilter::new(options::DEFAULT_FILTER)
        }
    };

    let mut layers: Vec<Box<dyn Layer<Registry> + Send + Sync>> = Vec::new();
    match options.output {
        ConsoleOutput::Off => {}
        ConsoleOutput::Journal => match JournalLayer::connect(role) {
            Ok(journal) => layers.push(journal.with_filter(filter.clone()).boxed()),
            Err(error) => {
                problems.push(format!(
                    "no journal at {SOCKET_PATH} ({error}), logging to stderr instead"
                ));
                layers.push(console_layer(StreamFormat, filter.clone()));
            }
        },
        ConsoleOutput::JournalStream => layers.push(console_layer(StreamFormat, filter.clone())),
        ConsoleOutput::Pretty => layers.push(console_layer(
            ConsoleFormat {
                role,
                colour: std::io::stderr().is_terminal(),
            },
            filter.clone(),
        )),
        ConsoleOutput::Json => layers.push(console_layer(JsonFormat { role }, filter.clone())),
        ConsoleOutput::Plain => layers.push(console_layer(ConsoleFormat { role, colour: false }, filter.clone())),
    }

    let mut sink = None;
    if let Some(path) = &options.database {
        match crate::store::open(path, role, options.retention) {
            Ok(opened) => {
                layers.push(DatabaseLayer::new(opened.clone(), role).with_filter(filter).boxed());
                sink = Some(opened);
            }
            Err(error) => problems.push(format!("no log database: {error}")),
        }
    }

    let installed = tracing_subscriber::registry().with(layers).try_init();
    if installed.is_ok() {
        install_panic_hook();
        for problem in &problems {
            crate::warn_in!("log", "{problem}");
        }
    }
    LogGuard { sink }
}

/// A console layer that writes each event on stderr in `format`, filtered by `filter`.
fn console_layer<F>(format: F, filter: EnvFilter) -> Box<dyn Layer<Registry> + Send + Sync>
where
    F: FormatEvent<Registry, DefaultFields> + Send + Sync + 'static,
{
    fmt::layer::<Registry>()
        .event_format(format)
        .with_writer(std::io::stderr)
        .with_filter(filter)
        .boxed()
}

/// Routes panics through the log as errors, keeping the previous hook's output.
fn install_panic_hook() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |information| {
        crate::error_in!("log", panic = %information, "a thread panicked");
        previous(information);
    }));
}
