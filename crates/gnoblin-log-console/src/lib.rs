//! The console sink of the log: one line per event on stderr.
//!
//! Each line reads `LEVEL [role] component: message field=value`. The role is the process
//! that logged, and the component is the crate or sub-area that logged the event.

use std::fmt;

use gnoblin_log::__tracing::{Event, Subscriber};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

/// Formats `LEVEL [role] component: message field=value`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsoleFormat {
    /// The name of the process role, as the log records it.
    pub role: &'static str,
}

impl<S, N> FormatEvent<S, N> for ConsoleFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(&self, context: &FmtContext<'_, S, N>, mut writer: Writer<'_>, event: &Event<'_>) -> fmt::Result {
        let metadata = event.metadata();
        write!(
            writer,
            "{:<5} [{}] {}: ",
            metadata.level().as_str(),
            self.role,
            metadata.target()
        )?;
        context.format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}
