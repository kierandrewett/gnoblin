//! The shared look of Gnoblin's command-line tools: colour and glyph
//! decisions (`theme`), text measuring (`text`), the human-output building
//! blocks (`components`) and timestamps (`time`).
//!
//! The design rules are in `design/cli-design-system.md`. Components return
//! strings and never print, so callers decide where output goes.

pub mod components;
pub mod text;
pub mod theme;
pub mod time;

pub use components::{
    ErrorReport, LineOutcome, badge, clear_waiting, definition_list, error_block, event_line, identifier, kv_list,
    section, status_line, table, waiting_line,
};
pub use text::{strip_ansi, width};
pub use theme::{ColorChoice, ColourToken, Glyph, OutputEnvironment, OutputTheme};
pub use time::{clock_utc, now_utc};
