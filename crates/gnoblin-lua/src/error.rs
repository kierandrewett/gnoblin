//! What a script error is and how it is remembered.
//!
//! Two rules shape this module.
//!
//! *A handler error never ends the host.* Lua errors are values here, not
//! results: every entry into Lua that a handler triggers is wrapped, so a handler
//! that raises is reported and the next handler still runs.
//!
//! *One report per error site.* A handler that fails on every pump would fill the
//! log with the same line, so the host remembers the sites it has already
//! reported and stays quiet about a repeat. The site is the file, the line and
//! the first line of the message: a different failure on the same line is a
//! different site, and the same failure twice is one report.

use std::fmt;

use gnoblin_client::ClientError;
use gnoblin_config::LocateError;

/// One script error, with the file and line a person needs to fix it.
///
/// Lua reports a runtime error as `path:line: message` because the chunk was
/// loaded under its path. That text is kept whole in [`ScriptError::message`],
/// and [`ScriptError::source`] and [`ScriptError::line`] are parsed out of it so a
/// caller can group errors without re-parsing prose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptError {
    source: Option<String>,
    line: Option<u32>,
    message: String,
}

impl ScriptError {
    /// Build one from Lua's or the host's text, parsing a leading location.
    pub(super) fn from_text(text: impl Into<String>) -> ScriptError {
        let message = strip_prefix(text.into());
        let (source, line) = locate(&message);
        ScriptError { source, line, message }
    }

    /// Build one from an `mlua` error.
    pub(super) fn from_lua(error: &mlua::Error) -> ScriptError {
        ScriptError::from_text(error.to_string())
    }

    /// The file the error came from, when Lua named one.
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    /// The line inside that file, when Lua named one.
    pub fn line(&self) -> Option<u32> {
        self.line
    }

    /// The whole message a person reads, starting with the location.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The line a person reads when a file stops: `<file> stopped at line N: <message>`.
    ///
    /// The location is not repeated in the message, so it reads once. An error with no
    /// location (a refusal the host built) is its message as it is.
    pub fn stop_notice(&self) -> String {
        match (&self.source, self.line) {
            (Some(source), Some(line)) => {
                let location = format!("{source}:{line}: ");
                let rest = self.message.strip_prefix(&location).unwrap_or(&self.message);
                format!("{source} stopped at line {line}: {rest}")
            }
            _ => self.message.clone(),
        }
    }

    /// The identity a repeat is recognised by.
    ///
    /// The location plus the first line of the message: the rest is a Lua
    /// traceback, whose frame line numbers vary between two occurrences of the
    /// same failure and would defeat the deduplication.
    pub(super) fn site(&self) -> String {
        let first = self.message.lines().next().unwrap_or_default();
        match (&self.source, self.line) {
            (Some(source), Some(line)) => format!("{source}:{line}: {first}"),
            _ => first.to_owned(),
        }
    }
}

impl fmt::Display for ScriptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ScriptError {}

/// Why the host could not start, could not find the configuration, or lost the
/// connection.
#[derive(Debug)]
pub enum HostError {
    /// The initial subscription failed, so the host has no state to serve reads.
    Subscribe(ClientError),
    /// The Lua state could not be set up. This is a bug in the host, not in a
    /// script: the state is created before any script runs.
    Sandbox(mlua::Error),
    /// No configuration file can be named: neither `XDG_CONFIG_HOME` nor `HOME`
    /// is usable, or the selected file is not Lua.
    Locate(LocateError),
    /// The connection failed while the pump was draining it.
    Client(ClientError),
    /// The configuration directory or the root file could not be read.
    Read {
        /// The path that failed.
        path: std::path::PathBuf,
        /// Why it failed.
        source: std::io::Error,
    },
}

impl fmt::Display for HostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::Subscribe(error) => write!(formatter, "cannot subscribe to the graph: {error}"),
            HostError::Sandbox(error) => write!(formatter, "cannot set up the Lua state: {error}"),
            HostError::Locate(error) => write!(formatter, "{error}"),
            HostError::Client(error) => write!(formatter, "the connection failed: {error}"),
            HostError::Read { path, source } => write!(formatter, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for HostError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HostError::Subscribe(error) | HostError::Client(error) => Some(error),
            HostError::Sandbox(error) => Some(error),
            HostError::Locate(error) => Some(error),
            HostError::Read { source, .. } => Some(source),
        }
    }
}

/// Drop mlua's category prefix, which is noise in a log line the file and line
/// already explain.
fn strip_prefix(text: String) -> String {
    for prefix in ["runtime error: ", "syntax error: ", "memory error: "] {
        if let Some(rest) = text.strip_prefix(prefix) {
            return rest.to_owned();
        }
    }
    text
}

/// Pull `<source>:<line>:` out of the first line of a Lua message.
///
/// Lua's format is `file:line: message`. The scan takes the first `:<digits>:`
/// run, which is the line number and not some later colon in the prose. A message
/// with no such run (a refusal built by the host, for instance) has no location,
/// and the caller reports it without one rather than inventing a line.
fn locate(message: &str) -> (Option<String>, Option<u32>) {
    let first = message.lines().next().unwrap_or_default();
    let bytes = first.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b':' {
            let start = index + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end > start && end < bytes.len() && bytes[end] == b':' && index > 0 {
                let line = first[start..end].parse::<u32>().ok();
                return (Some(first[..index].to_owned()), line);
            }
        }
        index += 1;
    }
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lua_message_names_its_file_and_line() {
        let error = ScriptError::from_text("runtime error: /home/user/init.lua:12: attempt to index a nil value");
        assert_eq!(error.message(), "/home/user/init.lua:12: attempt to index a nil value");
        assert_eq!(error.source(), Some("/home/user/init.lua"));
        assert_eq!(error.line(), Some(12));
    }

    #[test]
    fn a_message_without_a_location_has_none() {
        let error = ScriptError::from_text("gnoblin.load: patterns may not be absolute");
        assert_eq!(error.source(), None);
        assert_eq!(error.line(), None);
    }

    #[test]
    fn the_site_is_the_location_and_the_first_line() {
        let one = ScriptError::from_text("/c/init.lua:4: boom\nstack traceback:\n\t[C]: in function 'x'");
        let two = ScriptError::from_text("/c/init.lua:4: boom\nstack traceback:\n\t[C]: in function 'y'");
        assert_eq!(one.site(), two.site());
        let other = ScriptError::from_text("/c/init.lua:5: boom");
        assert_ne!(one.site(), other.site());
    }
}
