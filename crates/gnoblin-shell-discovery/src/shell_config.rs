//! `$XDG_CONFIG_HOME/gnoblin/config.toml`, which holds the one `[shell]` table.
//!
//! ```toml
//! [shell]
//! default = "tiler"             # an installed shell, by name
//! # path = "~/dev/tiler/shell.py"  # or a file: absolute, `~/...`, or relative to the directory holding this file
//! # language = "python"            # only to override detection
//! # isolate = true                 # run the shell in a guardian-supervised child
//! ```
//!
//! `default` and `path` are alternatives. An unknown key is a warning, not an error, so a file written for a newer
//! Gnoblin still starts an older one.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::language::Language;

/// The `[shell]` table of `config.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShellConfig {
    /// Whether the file has a `[shell]` table at all.
    pub section: bool,
    /// `default`: the name of an installed shell.
    pub default: Option<String>,
    /// `path`: a file, as written (unexpanded).
    pub path: Option<String>,
    /// `language`: overrides detection for the shell this file selects.
    pub language: Option<Language>,
    /// `isolate`: run the shell in a guardian-supervised child. The only key read when the shell came from a
    /// higher tier.
    pub isolate: Option<bool>,
    /// One line for each key that is not known.
    pub warnings: Vec<String>,
}

/// `config.toml` exists and cannot be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    /// The file.
    pub path: PathBuf,
    /// What is wrong with it, one line.
    pub message: String,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.path.display(), self.message)
    }
}

impl std::error::Error for ConfigError {}

/// Read the `[shell]` table of the file at `path`.
///
/// `Ok(None)` when the file is not there: a missing file is the normal case, not an error.
///
/// # Errors
///
/// [`ConfigError`] when the file cannot be read, is not TOML, has a key of the wrong type, or sets both `default`
/// and `path`.
pub fn read_shell_config(path: &Path) -> Result<Option<ShellConfig>, ConfigError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => return Ok(None),
        Err(error) => {
            return Err(ConfigError {
                path: path.to_path_buf(),
                message: format!("cannot read it: {error}"),
            });
        }
    };
    parse(&text).map(Some).map_err(|message| ConfigError {
        path: path.to_path_buf(),
        message,
    })
}

/// Parse the text of a `config.toml`. The error is one line.
pub(crate) fn parse(text: &str) -> Result<ShellConfig, String> {
    let table: toml::Table = text.parse().map_err(|error: toml::de::Error| describe(text, &error))?;
    let mut config = ShellConfig::default();
    for (key, value) in &table {
        if key != "shell" {
            config.warnings.push(format!("unknown key `{key}`"));
            continue;
        }
        let toml::Value::Table(shell) = value else {
            return Err("`shell` must be a table: write `[shell]`".to_owned());
        };
        config.section = true;
        for (key, value) in shell {
            match key.as_str() {
                "default" => config.default = Some(non_empty(key, value)?),
                "path" => config.path = Some(non_empty(key, value)?),
                "language" => config.language = Some(Language::from_name(&non_empty(key, value)?)),
                "isolate" => {
                    let toml::Value::Boolean(isolate) = value else {
                        return Err("`[shell] isolate` must be true or false".to_owned());
                    };
                    config.isolate = Some(*isolate);
                }
                other => config.warnings.push(format!("unknown key `shell.{other}`")),
            }
        }
    }
    if config.default.is_some() && config.path.is_some() {
        return Err("`[shell]` sets both `default` and `path`; keep one".to_owned());
    }
    Ok(config)
}

/// The string value of a `[shell]` key, which must not be empty.
fn non_empty(key: &str, value: &toml::Value) -> Result<String, String> {
    match value {
        toml::Value::String(text) if !text.is_empty() => Ok(text.clone()),
        toml::Value::String(_) => Err(format!("`[shell] {key}` is empty")),
        _ => Err(format!("`[shell] {key}` must be a string")),
    }
}

/// A TOML syntax error as one line: the line number and the parser's message.
pub(crate) fn describe(text: &str, error: &toml::de::Error) -> String {
    match error.span() {
        Some(span) => {
            let start = span.start.min(text.len());
            let line = text.as_bytes()[..start].iter().filter(|byte| **byte == b'\n').count() + 1;
            format!("line {line}: {}", error.message())
        }
        None => error.message().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_key_names_a_shell() {
        let config = parse("[shell]\ndefault = \"tiler\"\n").expect("valid");
        assert!(config.section);
        assert_eq!(config.default.as_deref(), Some("tiler"));
        assert_eq!(config.path, None);
    }

    #[test]
    fn the_path_language_and_isolate_keys_are_read() {
        let config = parse("[shell]\npath = \"~/s.py\"\nlanguage = \"python\"\nisolate = true\n").expect("valid");
        assert_eq!(config.path.as_deref(), Some("~/s.py"));
        assert_eq!(config.language, Some(Language::Python));
        assert_eq!(config.isolate, Some(true));
    }

    #[test]
    fn a_file_without_a_shell_table_has_no_section() {
        let config = parse("# nothing here\n").expect("valid");
        assert!(!config.section);
        assert_eq!(config, ShellConfig::default());
    }

    #[test]
    fn unknown_keys_are_warnings() {
        let config = parse("theme = 1\n[shell]\ncolour = 2\ndefault = \"a\"\n").expect("valid");
        assert_eq!(
            config.warnings,
            vec![
                "unknown key `shell.colour`".to_owned(),
                "unknown key `theme`".to_owned()
            ]
        );
        assert_eq!(config.default.as_deref(), Some("a"));
    }

    #[test]
    fn both_default_and_path_is_an_error() {
        assert_eq!(
            parse("[shell]\ndefault = \"a\"\npath = \"/b\"\n"),
            Err("`[shell]` sets both `default` and `path`; keep one".to_owned())
        );
    }

    #[test]
    fn a_key_of_the_wrong_type_is_an_error() {
        assert_eq!(
            parse("[shell]\ndefault = 3\n"),
            Err("`[shell] default` must be a string".to_owned())
        );
        assert_eq!(
            parse("[shell]\nisolate = \"yes\"\n"),
            Err("`[shell] isolate` must be true or false".to_owned())
        );
        assert_eq!(
            parse("shell = 1\n"),
            Err("`shell` must be a table: write `[shell]`".to_owned())
        );
        assert_eq!(
            parse("[shell]\npath = \"\"\n"),
            Err("`[shell] path` is empty".to_owned())
        );
    }

    #[test]
    fn a_syntax_error_names_its_line() {
        let error = parse("[shell]\ndefault = \n").expect_err("not TOML");
        assert!(error.starts_with("line "), "{error}");
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let directory = tempfile::tempdir().expect("a directory");
        assert_eq!(read_shell_config(&directory.path().join("config.toml")), Ok(None));
    }

    #[test]
    fn an_unreadable_path_is_an_error() {
        let directory = tempfile::tempdir().expect("a directory");
        let error = read_shell_config(directory.path()).expect_err("a directory is not a file");
        assert!(error.message.starts_with("cannot read it: "), "{error}");
    }
}
