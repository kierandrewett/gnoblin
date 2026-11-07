//! Where a module's own output and the host's reports go.
//!
//! A module has no file descriptor and no standard output: it runs inside the
//! compositor's process with no WASI implementation linked, so `stdout` and
//! `stderr` do not exist for it (design/rust-compositor.md, D6). What it has is the
//! `log` host function, which writes one line to this sink, and the sink is the
//! seam the compositor already needs for everything else it reports: the default
//! writes into the log, and a supervisor, a test or the console overlay
//! supplies its own.
//!
//! The sink is shared behind a lock rather than held by value because every
//! module's store needs it and a store owns its data. One lock around one line is
//! not a contention point: the host writes a line when a module is denied, traps or
//! asks for its own line.

use std::fmt;
use std::sync::{Arc, Mutex};

use crate::host_log;

/// How important a line is.
///
/// The four levels are the module ABI's words: a module passes one and the host
/// does not filter, because filtering belongs to whoever installed the sink.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// The module could not do what it was asked.
    Error,
    /// Something the module did was refused or ignored.
    Warn,
    /// Ordinary progress.
    Info,
    /// Detail for a developer.
    Debug,
}

impl Level {
    /// The level a module's word names, defaulting to [`Level::Info`].
    pub fn from_word(word: u32) -> Level {
        match word {
            0 => Level::Error,
            1 => Level::Warn,
            2 => Level::Info,
            _ => Level::Debug,
        }
    }

    /// The level's name, as a line starts.
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Error => "error",
            Level::Warn => "warn",
            Level::Info => "info",
            Level::Debug => "debug",
        }
    }
}

/// One line: who wrote it, how important it is, and what it says.
#[derive(Clone, Copy, Debug)]
pub struct LogLine<'a> {
    /// The module the line is about.
    pub module: &'a str,
    /// How important it is.
    pub level: Level,
    /// The text. A module's own line is opaque; the host never interprets it.
    pub message: &'a str,
}

impl fmt::Display for LogLine<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "wasm[{}] {}: {}",
            self.module,
            self.level.as_str(),
            self.message
        )
    }
}

/// Where lines go.
///
/// `Send` because a store may move to another thread, and a sink that could not
/// follow would make the host single-threaded for a reason nothing else shares.
pub trait Sink: Send {
    /// Accept one line.
    fn write(&mut self, record: &LogLine<'_>);
}

/// The default: one line into the log, at the line's own level.
///
/// The module's name travels as a field, so the console and the database can filter by it.
#[derive(Debug, Default)]
pub struct LogSink;

impl Sink for LogSink {
    fn write(&mut self, record: &LogLine<'_>) {
        let module = record.module;
        let message = record.message;
        match record.level {
            Level::Error => host_log::error!(module = module, "{message}"),
            Level::Warn => host_log::warn!(module = module, "{message}"),
            Level::Info => host_log::info!(module = module, "{message}"),
            Level::Debug => host_log::debug!(module = module, "{message}"),
        }
    }
}

/// The sink every store shares with the host.
pub(crate) type Shared = Arc<Mutex<Box<dyn Sink>>>;

/// Write one line, ignoring a poisoned lock.
///
/// A poisoned lock means another thread panicked while holding it. Rethrowing
/// here would turn a module's log line into a panic in the compositor, which is
/// the opposite of what a sandbox is for, so the line is written through the
/// poisoned guard and the panic is reported by whoever caused it.
pub(crate) fn write(sink: &Shared, record: &LogLine<'_>) {
    match sink.lock() {
        Ok(mut sink) => sink.write(record),
        Err(poisoned) => poisoned.into_inner().write(record),
    }
}

#[cfg(test)]
mod tests {
    use super::{Level, LogLine};

    #[test]
    fn prints_a_line() {
        let record = LogLine {
            module: "placement",
            level: Level::Warn,
            message: "refused",
        };
        assert_eq!(record.to_string(), "wasm[placement] warn: refused");
    }

    #[test]
    fn maps_level_words() {
        assert_eq!(Level::from_word(0), Level::Error);
        assert_eq!(Level::from_word(1), Level::Warn);
        assert_eq!(Level::from_word(2), Level::Info);
        assert_eq!(Level::from_word(9), Level::Debug);
    }
}
