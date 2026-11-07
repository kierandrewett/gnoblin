//! The console sink of the log: one line per event on stderr, in one of three formats.
//!
//! [`ConsoleFormat`] is the human format, `LEVEL  role/component  message  key=value`. The
//! component column is as wide as the longest component printed so far, up to
//! [`MAX_COMPONENT_WIDTH`]. With colour, each component has the same colour in every run, so a
//! crate is easy to spot.
//!
//! [`StreamFormat`] is for stderr that systemd connects to the journal. Each line starts with a
//! `<N>` priority prefix, which systemd reads and removes. It has no timestamp or colour, because
//! the journal adds its own.
//!
//! [`JsonFormat`] writes one JSON object per line, for a machine reader.

use std::fmt::{self, Write as _};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::__tracing::{Event, Level, Subscriber};
use crate::event::Logged;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

/// The widest the component column grows. A longer component still prints in full, past the
/// column, so no name is ever cut short.
pub const MAX_COMPONENT_WIDTH: usize = 16;

/// The widest component printed so far, which the column keeps as its width.
static COMPONENT_WIDTH: AtomicUsize = AtomicUsize::new(0);

/// Formats `LEVEL  role/component  message  key=value`, with the component in colour when
/// `colour` is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsoleFormat {
    /// The name of the process role, as the log records it.
    pub role: &'static str,
    /// Whether the component is printed with an ANSI colour.
    pub colour: bool,
}

impl<S, N> FormatEvent<S, N> for ConsoleFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(&self, _context: &FmtContext<'_, S, N>, mut writer: Writer<'_>, event: &Event<'_>) -> fmt::Result {
        let logged = Logged::from_event(event);
        let width = widen(&logged.component);
        writeln!(
            writer,
            "{}",
            pretty_line(&logged, *event.metadata().level(), self.role, self.colour, width)
        )
    }
}

/// Formats one human line: `LEVEL  role/component  message  key=value`. The level is padded to
/// five columns and the component to `width`. With `coloured` set, the role and the fields are
/// dim and the component takes its stable colour. The line never has square brackets, so a
/// component name is not repeated in a tag before the message.
pub fn pretty_line(logged: &Logged, level: Level, role: &str, coloured: bool, width: usize) -> String {
    let mut line = format!("{:<5}  ", level.as_str());
    if coloured {
        let _ = write!(
            line,
            "\x1b[2m{role}\x1b[0m/\x1b[{}m{:<width$}\x1b[0m  ",
            colour(&logged.component),
            logged.component
        );
    } else {
        let _ = write!(line, "{role}/{:<width$}  ", logged.component);
    }
    line.push_str(&logged.message);
    let fields = logged.fields_line();
    if !fields.is_empty() {
        line.push_str("  ");
        if coloured {
            let _ = write!(line, "\x1b[2m{fields}\x1b[0m");
        } else {
            line.push_str(&fields);
        }
    }
    line
}

/// Formats one line for stderr that systemd reads a priority from: `<N>component: message
/// field=value`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StreamFormat;

impl<S, N> FormatEvent<S, N> for StreamFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(&self, _context: &FmtContext<'_, S, N>, mut writer: Writer<'_>, event: &Event<'_>) -> fmt::Result {
        let logged = Logged::from_event(event);
        writeln!(writer, "{}", logged.stream_line(*event.metadata().level()))
    }
}

/// Formats one event as a JSON object on one line, for a machine reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonFormat {
    /// The name of the process role, as the log records it.
    pub role: &'static str,
}

impl<S, N> FormatEvent<S, N> for JsonFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(&self, _context: &FmtContext<'_, S, N>, mut writer: Writer<'_>, event: &Event<'_>) -> fmt::Result {
        let logged = Logged::from_event(event);
        writeln!(writer, "{}", json_line(&logged, *event.metadata().level(), self.role))
    }
}

/// One event as a JSON object with the keys `level`, `role`, `component`, `target`, `message`
/// and `fields`. Every value is a string. `fields` is an object with one key per field, in the
/// order the event recorded them. A field name recorded twice appears twice, and a JSON reader
/// keeps the last value.
pub fn json_line(logged: &Logged, level: Level, role: &str) -> String {
    let mut line = String::from("{");
    json_pair(&mut line, "level", level.as_str());
    line.push(',');
    json_pair(&mut line, "role", role);
    line.push(',');
    json_pair(&mut line, "component", &logged.component);
    line.push(',');
    json_pair(&mut line, "target", &logged.target);
    line.push(',');
    json_pair(&mut line, "message", &logged.message);
    line.push_str(",\"fields\":{");
    for (index, (name, value)) in logged.fields.iter().enumerate() {
        if index > 0 {
            line.push(',');
        }
        json_string(&mut line, name);
        line.push(':');
        json_string(&mut line, value);
    }
    line.push_str("}}");
    line
}

/// Writes `"key":"value"` with both sides escaped.
fn json_pair(out: &mut String, key: &str, value: &str) {
    json_string(out, key);
    out.push(':');
    json_string(out, value);
}

/// Writes `text` as a JSON string, escaping quotes, backslashes and control characters.
fn json_string(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if u32::from(control) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(control));
            }
            other => out.push(other),
        }
    }
    out.push('"');
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

    fn sample(message: &str) -> Logged {
        Logged {
            component: "wayland".to_owned(),
            message: message.to_owned(),
            target: "gnoblin_wayland".to_owned(),
            fields: vec![("output".to_owned(), "\"DP-1\"".to_owned())],
        }
    }

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

    #[test]
    fn a_pretty_line_has_no_brackets_and_pads_level_and_component() {
        // "wayland" pads to the width 8, then two spaces precede the message.
        assert_eq!(
            pretty_line(&sample("ready"), Level::INFO, "compositor", false, 8),
            "INFO   compositor/wayland   ready  output=\"DP-1\""
        );
        let coloured = pretty_line(&sample("ready"), Level::INFO, "compositor", true, 8);
        assert!(coloured.starts_with("INFO   \x1b[2mcompositor\x1b[0m/\x1b["));
        assert!(coloured.contains("\x1b[0m  ready  \x1b[2moutput=\"DP-1\"\x1b[0m"));
        // Escape codes contain '[', so strip them before checking that no bracket is printed.
        assert!(!"INFO   compositor/wayland   ready".contains(['[', ']']));
        assert!(!coloured.replace("\x1b[", "").contains(['[', ']']));
    }

    #[test]
    fn a_stream_line_starts_with_the_priority_and_the_component() {
        assert_eq!(
            sample("ready").stream_line(Level::INFO),
            "<6>wayland: ready output=\"DP-1\""
        );
        assert!(sample("bad").stream_line(Level::ERROR).starts_with("<3>wayland: "));
    }

    #[test]
    fn a_json_line_is_one_object_and_escapes_its_strings() {
        let line = json_line(&sample("a \"quoted\"\nline"), Level::WARN, "compositor");
        assert!(!line.contains('\n'));
        assert_eq!(
            line,
            "{\"level\":\"WARN\",\"role\":\"compositor\",\"component\":\"wayland\",\
             \"target\":\"gnoblin_wayland\",\"message\":\"a \\\"quoted\\\"\\nline\",\
             \"fields\":{\"output\":\"\\\"DP-1\\\"\"}}"
        );
    }
}
