//! Installs the gnoblin log for one process.
//!
//! [`init`] installs one `tracing` subscriber with two sinks. The console sink, from
//! `gnoblin-log-console`, prints one line per event on stderr. The database sink, from
//! `gnoblin-log-store`, stores each event as a row in a SQLite file, so that a session can be
//! inspected after the fact. Both sinks share one filter, taken from `GNOBLIN_LOG`, and the
//! database location comes from `GNOBLIN_LOG_DB`; see [`LogOptions`].
//!
//! Only the binaries name this crate. The crates that log use the macros of `gnoblin-log`.

mod options;

use std::panic;

use gnoblin_log::ProcessRole;
use gnoblin_log_console::ConsoleFormat;
use gnoblin_log_store::{DatabaseLayer, Sink};
use tracing_subscriber::Registry;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::layer::{Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;

pub use gnoblin_log_store::Retention;
pub use options::{DATABASE_VARIABLE, FILTER_VARIABLE, LogOptions, LogSettings};

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
/// `info`, and a database that cannot be opened leaves the console working. Each problem is
/// reported as a warning once the subscriber is installed. A panic is logged as an error
/// before the previous panic hook runs.
///
/// Calling this after another subscriber is installed leaves that subscriber in place.
pub fn init(role: ProcessRole, options: &LogOptions) -> LogGuard {
    let role = role.name();
    let mut problems = Vec::new();

    let filter = match EnvFilter::try_new(&options.filter) {
        Ok(filter) => filter,
        Err(error) => {
            problems.push(format!("ignoring {FILTER_VARIABLE}={:?}: {error}", options.filter));
            EnvFilter::new(options::DEFAULT_FILTER)
        }
    };

    let mut layers: Vec<Box<dyn Layer<Registry> + Send + Sync>> = Vec::new();
    if options.console {
        layers.push(
            tracing_subscriber::fmt::layer()
                .event_format(ConsoleFormat { role })
                .with_writer(std::io::stderr)
                .with_filter(filter.clone())
                .boxed(),
        );
    }

    let mut sink = None;
    if let Some(path) = &options.database {
        match gnoblin_log_store::open(path, role, options.retention) {
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
            gnoblin_log::warn!("{problem}");
        }
    }
    LogGuard { sink }
}

/// Routes panics through the log as errors, keeping the previous hook's output.
fn install_panic_hook() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |information| {
        gnoblin_log::error!(panic = %information, "a thread panicked");
        previous(information);
    }));
}
