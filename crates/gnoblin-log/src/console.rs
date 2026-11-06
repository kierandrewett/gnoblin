//! The console half of the log: one line per event on stderr, tagged with its component.

use std::fmt;

use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

/// Formats `LEVEL [component] target: message field=value`.
pub(crate) struct ConsoleFormat {
    pub(crate) component: &'static str,
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
            self.component,
            metadata.target()
        )?;
        context.format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}
