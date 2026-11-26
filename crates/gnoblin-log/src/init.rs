//! Installs the gnoblin log for one process.
//!
//! [`init`] installs one `tracing` subscriber. Its console layer follows the resolved
//! [`ConsoleOutput`]: the native journal (the `journal` module), the `<N>` stream format, the pretty or
//! plain line format, JSON lines, or nothing. A sink, when the binary sets one in [`LogOptions`],
//! receives every event as a batch through the `pipeline` module, so a file sink (or any other consumer) can keep
//! the log after the fact without this crate depending on it. All layers share one filter, taken from
//! `GNOBLIN_LOG`; see [`LogOptions`].
//!
//! Only the binaries use this module: they enable the `init` feature. The crates that log use the
//! macros of the facade.

mod options;

use std::io::IsTerminal as _;
use std::panic;

use crate::ProcessRole;
use crate::console::{ConsoleFormat, JsonFormat, StreamFormat};
use crate::journal::{JournalLayer, SOCKET_PATH};
use crate::pipeline::{Pipeline, PipelineLayer};
use tracing_subscriber::Registry;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt::format::DefaultFields;
use tracing_subscriber::fmt::{self, FormatEvent};
use tracing_subscriber::layer::{Layer, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;

// The component this module's own warnings and panic reports log under.
crate::logger!(log = "log");

/// The options a process sets up logging with, and the environment variables that name them.
pub use options::{Console, ConsoleOutput, FILTER_VARIABLE, FORMAT_VARIABLE, LogOptions, LogSettings, Probe};

/// Keeps the sink's writer thread alive. Dropping it waits, for a bounded time, for pending
/// records to reach the sink, so a process should hold it until it is done logging.
#[must_use = "dropping the guard flushes the log sink immediately"]
pub struct LogGuard {
    pipeline: Option<Pipeline>,
}

impl Drop for LogGuard {
    fn drop(&mut self) {
        if let Some(pipeline) = &self.pipeline {
            pipeline.flush();
        }
    }
}

/// Installs the process-wide subscriber for `role`, with `options`.
///
/// Problems with the configuration do not stop the process: an invalid filter falls back to
/// `info`, an unknown format is ignored, a sink whose writer thread cannot start leaves the
/// console working, and a journal that cannot be reached writes to stderr instead. Each problem
/// is reported as a warning once the subscriber is installed. A panic is logged as an error
/// before the previous panic hook runs.
///
/// Calling this after another subscriber is installed leaves that subscriber in place.
pub fn init(role: ProcessRole, options: &LogOptions) -> LogGuard {
    let role = role.name();
    let mut problems = options.problems.clone();

    let filter = match resolve_filter(&options.filter) {
        Ok((filter, spec)) => {
            if let Some(spec) = spec {
                crate::filter::set_spec(spec);
            }
            filter
        }
        Err(problem) => {
            problems.push(problem);
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

    let mut pipeline = None;
    if let Some(handle) = &options.sink {
        match Pipeline::start(handle.shared(), role) {
            Ok(started) => {
                layers.push(PipelineLayer::new(started.clone()).with_filter(filter).boxed());
                pipeline = Some(started);
            }
            Err(error) => problems.push(format!("no log sink thread: {error}")),
        }
    }

    let installed = tracing_subscriber::registry().with(layers).try_init();
    if installed.is_ok() {
        install_panic_hook();
        for problem in &problems {
            log::warn!("{problem}");
        }
    }
    LogGuard { pipeline }
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
        log::error!(panic = %information, "a thread panicked");
        previous(information);
    }));
}

/// The console filter for `text`, and the component levels it sets.
///
/// `text` is the documented specification first (`wayland:debug,*:warn`, see [`crate::filter::Spec`]): it sets
/// the level of every component, which is what a `log::debug!` call site reads, and it becomes the matching
/// `EnvFilter` directives so the console and the sink agree with it. A text that is not that syntax is tried as
/// an `EnvFilter` (`smithay=debug,warn`) so a filter written for `tracing` keeps working; it sets no component
/// level. A text that is neither is refused with the documented syntax's error.
fn resolve_filter(text: &str) -> Result<(EnvFilter, Option<crate::filter::Spec>), String> {
    match crate::filter::Spec::parse(text) {
        Ok(spec) => {
            let filter = EnvFilter::try_new(spec.env_filter_directives())
                .map_err(|error| format!("ignoring {FILTER_VARIABLE}={text:?}: {error}"))?;
            Ok((filter, Some(spec)))
        }
        Err(error) => EnvFilter::try_new(text)
            .map(|filter| (filter, None))
            .map_err(|_| format!("ignoring {FILTER_VARIABLE}={text:?}: {error}")),
    }
}
