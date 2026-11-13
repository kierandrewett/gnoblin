//! The errors the JavaScript runtime reports to its host.
//!
//! Every failure a generation can hit is one [`JsError`]: a [`JsErrorKind`] the host can branch on
//! (a budget trip is a different recovery from a script exception), the message, the source location
//! in the author's `.ts` or `.js` coordinates, and the stack with its frames remapped the same way.

use std::fmt;
use std::path::PathBuf;

use thiserror::Error;

/// A TypeScript file that does not parse or transform.
///
/// `line` and `column` are 1-based and point into the `.ts` source the author wrote, not into the
/// stripped JavaScript, because the parser reports against the original text.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{}:{line}:{column}: {message}", path.display())]
pub struct JsTsError {
    /// The file that failed to parse.
    pub path: PathBuf,
    /// The 1-based line of the first error.
    pub line: u32,
    /// The 1-based column of the first error, counted in characters.
    pub column: u32,
    /// The parser's description of the first error.
    pub message: String,
}

/// What kind of failure a [`JsError`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsErrorKind {
    /// The wall-clock budget ran out: the script was interrupted.
    Budget,
    /// The generation's memory limit was reached.
    Memory,
    /// A script threw, or a promise it returned rejected.
    Exception,
    /// A TypeScript file could not be parsed or stripped.
    Ts,
    /// A module could not be found, read or confined to the root.
    Load,
    /// The engine itself failed, for example while creating a context.
    Engine,
}

/// A position in an author's source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsLocation {
    /// The file the position is in.
    pub file: PathBuf,
    /// The 1-based line.
    pub line: u32,
    /// The 1-based column.
    pub column: u32,
}

/// A failure in a JavaScript generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsError {
    /// The kind of failure, so the host can pick a recovery.
    pub kind: JsErrorKind,
    /// The message the script or the engine produced.
    pub message: String,
    /// Where the failure happened, when a frame could be mapped back to an author's file.
    pub location: Option<JsLocation>,
    /// The stack with each mapped frame rewritten to the author's coordinates.
    pub stack: Option<String>,
}

impl JsError {
    /// A failure with no location or stack.
    pub(crate) fn new(kind: JsErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            location: None,
            stack: None,
        }
    }
}

/// A module that cannot be resolved, found or read.
///
/// Each variant names the specifier or the path, so a script author sees which import failed without
/// the runtime's internal module names.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JsLoadError {
    /// The specifier is neither relative, absolute for the host's entry, nor an embedded module name.
    #[error(
        "cannot resolve `{name}`: only `./` and `../` paths and embedded modules such as `gnoblin` can be imported"
    )]
    Unresolvable {
        /// The specifier as written.
        name: String,
    },
    /// An embedded module name that the host did not register.
    #[error("no embedded module named `{name}`")]
    NoEmbedded {
        /// The specifier as written.
        name: String,
    },
    /// A relative import from a module that is not a file, such as an embedded module.
    #[error("cannot import `{name}` relative to `{base}`, which is not a file in the config root")]
    NoFileBase {
        /// The specifier as written.
        name: String,
        /// The importing module.
        base: String,
    },
    /// The path leaves the config root, lexically or through a symlink.
    #[error("`{name}` resolves to {}, outside the config root {}", path.display(), root.display())]
    Escape {
        /// The specifier as written.
        name: String,
        /// Where it resolved to.
        path: PathBuf,
        /// The config root every module must stay under.
        root: PathBuf,
    },
    /// No candidate file exists.
    #[error("`{name}` not found under the config root (tried {tried})")]
    NotFound {
        /// The specifier as written.
        name: String,
        /// The candidate paths, comma separated.
        tried: String,
    },
    /// The path is not a regular file.
    #[error("{} is not a file", path.display())]
    NotAFile {
        /// The path that was found.
        path: PathBuf,
    },
    /// The file extension is not one the loader runs.
    #[error("{} is not a module: only .ts, .mts, .js and .mjs files can be imported", path.display())]
    Extension {
        /// The path that was found.
        path: PathBuf,
    },
    /// The path cannot be a QuickJS module name.
    #[error("{} is not valid UTF-8", path.display())]
    NotUtf8 {
        /// The offending path.
        path: PathBuf,
    },
    /// The file could not be read.
    #[error("cannot read {}: {reason}", path.display())]
    Read {
        /// The file that failed.
        path: PathBuf,
        /// The operating system's description.
        reason: String,
    },
}

impl From<JsLoadError> for JsError {
    fn from(error: JsLoadError) -> Self {
        Self::new(JsErrorKind::Load, error.to_string())
    }
}

impl From<JsTsError> for JsError {
    fn from(error: JsTsError) -> Self {
        Self {
            kind: JsErrorKind::Ts,
            location: Some(JsLocation {
                file: error.path.clone(),
                line: error.line,
                column: error.column,
            }),
            message: error.message,
            stack: None,
        }
    }
}

impl fmt::Display for JsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.location {
            Some(at) => write!(f, "{}:{}:{}: {}", at.file.display(), at.line, at.column, self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for JsError {}
