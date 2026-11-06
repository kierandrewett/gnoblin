//! The console sink of the log: one line per event on stderr.
//!
//! Each line reads `LEVEL [role] component: message field=value`. The role is the process
//! that logged, and the component is the crate or sub-area declared by `gnoblin_log::component!`
//! (or the `component` argument of a macro). The component column is as wide as the longest
//! component printed so far, up to [`MAX_COMPONENT_WIDTH`]. When the writer takes ANSI escapes,
//! each component has a colour that is the same in every run, so a crate is easy to spot.

use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::__tracing::{Event, Subscriber};
use crate::event::Logged;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

/// The widest the component column grows. A longer component still prints in full, past the
/// column, so no name is ever cut short.
pub const MAX_COMPONENT_WIDTH: usize = 16;

/// The widest component printed so far, which the column keeps as its width.
static COMPONENT_WIDTH: AtomicUsize = AtomicUsize::new(0);

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
    fn format_event(&self, _context: &FmtContext<'_, S, N>, mut writer: Writer<'_>, event: &Event<'_>) -> fmt::Result {
        let logged = Logged::from_event(event);
        let width = widen(&logged.component);
        write!(writer, "{:<5} [{}] ", event.metadata().level().as_str(), self.role)?;
        if writer.has_ansi_escapes() {
            write!(
                writer,
                "\x1b[{}m{:<width$}\x1b[0m: ",
                colour(&logged.component),
                logged.component
            )?;
        } else {
            write!(writer, "{:<width$}: ", logged.component)?;
        }
        write!(writer, "{}", logged.message)?;
        let fields = logged.fields_line();
        if !fields.is_empty() {
            write!(writer, " {fields}")?;
        }
        writeln!(writer)
    }
}

/// Records `component` in the column width and returns the width to pad it to.
fn widen(component: &str) -> usize {
    let wanted = component.len().min(MAX_COMPONENT_WIDTH);
    let previous = COMPONENT_WIDTH.fetch_max(wanted, Ordering::Relaxed);
    previous.max(wanted)
}

/// The ANSI foreground colour of `component`: a hash of its name, so the colour is stable across
/// runs, chosen from the six colours that stand out on a dark background.
fn colour(component: &str) -> u8 {
    // FNV-1a, which is fixed by its definition, so the same name always gets the same colour.
    let hash = component.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    let index = u8::try_from(hash % 6).unwrap_or(0);
    31 + index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_colour_of_a_component_is_stable_and_one_of_six() {
        assert_eq!(colour("wayland"), colour("wayland"));
        for name in ["wayland", "backend", "render", "compositor", "lua", "ext"] {
            assert!((31..=36).contains(&colour(name)), "{name}");
        }
    }

    #[test]
    fn the_column_never_pads_past_the_cap() {
        let longest = "compat-displayconfig-extra-long-name";
        assert!(widen(longest) <= MAX_COMPONENT_WIDTH);
        assert!(widen("x") <= MAX_COMPONENT_WIDTH);
    }
}
