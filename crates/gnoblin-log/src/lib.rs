//! Logging for every gnoblin process.
//!
//! [`init`] installs one `tracing` subscriber with two sinks. The console sink prints one line
//! per event on stderr, tagged with the component's name. The database sink stores each event
//! as a row in a SQLite file, so that a session can be inspected after the fact. Both sinks
//! share one filter, taken from `GNOBLIN_LOG`, and the database location comes from
//! `GNOBLIN_LOG_DB`; see [`LogOptions`].

mod console;
mod database;
mod options;

use std::panic;

use tracing_subscriber::Registry;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::layer::{Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;

pub use options::{DATABASE_VARIABLE, FILTER_VARIABLE, LogOptions};

use console::ConsoleFormat;
use database::{DatabaseLayer, Sink};

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

/// Installs the process-wide subscriber for the component named `component`, reading the
/// options from the environment.
///
/// Problems with the configuration do not stop the process: an invalid filter falls back to
/// `info`, and a database that cannot be opened leaves the console working. Each problem is
/// reported as a warning once the subscriber is installed. A panic is logged as an error
/// before the previous panic hook runs.
///
/// Calling this after another subscriber is installed leaves that subscriber in place.
pub fn init(component: &'static str) -> LogGuard {
    init_with(component, &LogOptions::from_env())
}

/// Installs the process-wide subscriber for `component` with `options`.
///
/// See [`init`] for the behaviour.
pub fn init_with(component: &'static str, options: &LogOptions) -> LogGuard {
    let mut problems = Vec::new();

    let filter = match EnvFilter::try_new(&options.filter) {
        Ok(filter) => filter,
        Err(error) => {
            problems.push(format!("ignoring {FILTER_VARIABLE}={:?}: {error}", options.filter));
            EnvFilter::new(options::DEFAULT_FILTER)
        }
    };

    let mut layers: Vec<Box<dyn Layer<Registry> + Send + Sync>> = vec![
        tracing_subscriber::fmt::layer()
            .event_format(ConsoleFormat { component })
            .with_writer(std::io::stderr)
            .with_filter(filter.clone())
            .boxed(),
    ];

    let mut sink = None;
    if let Some(path) = &options.database {
        match database::open(path, component) {
            Ok(opened) => {
                layers.push(
                    DatabaseLayer::new(opened.clone(), component)
                        .with_filter(filter)
                        .boxed(),
                );
                sink = Some(opened);
            }
            Err(error) => problems.push(format!("no log database: {error}")),
        }
    }

    let installed = tracing_subscriber::registry().with(layers).try_init();
    if installed.is_ok() {
        install_panic_hook();
        for problem in &problems {
            tracing::warn!("{problem}");
        }
    }
    LogGuard { sink }
}

/// Routes panics through `tracing` as errors, keeping the previous hook's output.
fn install_panic_hook() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |information| {
        tracing::error!(panic = %information, "a thread panicked");
        previous(information);
    }));
}
