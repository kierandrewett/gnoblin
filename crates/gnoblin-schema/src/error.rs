//! Why a schema file or the catalog was rejected.

use std::fmt;
use std::io;
use std::path::PathBuf;

/// A schema that could not be read, decoded or validated.
///
/// Validation keeps every problem in one error rather than stopping at the
/// first, because a schema author edits several members per change and CI should
/// name all of them in one run.
#[derive(Debug)]
pub enum SchemaError {
    /// A file or directory could not be read.
    Read { path: PathBuf, source: io::Error },
    /// The file is TOML but not in the schema shape, so no rule can be checked.
    Toml { source: String, message: String },
    /// One or more rules failed. Each problem names the member it is about.
    Invalid {
        /// The file, directory or catalog the problems came from.
        source: String,
        problems: Vec<String>,
    },
}

impl fmt::Display for SchemaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchemaError::Read { path, source } => write!(formatter, "cannot read {}: {source}", path.display()),
            SchemaError::Toml { source, message } => write!(formatter, "{source}: {message}"),
            SchemaError::Invalid { source, problems } => {
                writeln!(formatter, "{source}:")?;
                for problem in problems {
                    writeln!(formatter, "  - {problem}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for SchemaError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SchemaError::Read { source, .. } => Some(source),
            SchemaError::Toml { .. } | SchemaError::Invalid { .. } => None,
        }
    }
}
