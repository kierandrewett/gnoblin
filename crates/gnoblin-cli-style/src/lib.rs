//! The shared look of Gnoblin's command-line tools: colour and glyph
//! decisions (`theme`), text measuring (`text`), the human-output building
//! blocks (`components`) and timestamps (`time`).
//!
//! The design rules are in `design/cli-design-system.md`. Components return
//! strings and never print, so callers decide where output goes.

/// Human-output building blocks: each returns a `String` for the caller to print.
pub mod components;
/// Measuring and stripping terminal text, so columns line up with ANSI styling present.
pub mod text;
/// Colour and glyph decisions: the output environment and the palette tokens.
pub mod theme;
/// UTC timestamps for output, formatted from the clock with no time-zone lookup.
pub mod time;

/// The building blocks a command composes its output from, re-exported at the root.
pub use components::{
    ErrorReport, LineOutcome, badge, clear_waiting, definition_list, error_block, event_line, identifier, kv_list,
    section, status_line, table, waiting_line,
};
/// Text helpers: the visible width of a string and its ANSI-stripped form.
pub use text::{strip_ansi, width};
/// Output decisions: colour choice, the colour token, glyphs and the environment they follow.
pub use theme::{ColorChoice, ColourToken, Glyph, OutputEnvironment, OutputTheme};
/// Timestamp helpers: format epoch milliseconds, or the current time, as UTC `HH:MM:SS.mmm`.
pub use time::{clock_utc, now_utc};
