//! The one place that decides which file is the configuration.
//!
//! The configuration document is what the TypeScript or JavaScript shell commits, so the file that holds it is the
//! shell that discovery selects (`gnoblin-shell-discovery`, which owns the tiers and the error text):
//!
//! 1. `GNOBLIN_SHELL`, a path (when it contains `/`) or the name of an installed shell. A test and a nested session
//!    use it to point at a shell without touching the user's own.
//! 2. `[shell] default` or `[shell] path` in `$XDG_CONFIG_HOME/gnoblin/config.toml`.
//! 3. `$XDG_CONFIG_HOME/gnoblin/shell.ts`, `shell.js`, `shell.mts`, `shell.mjs` (and the native and Python
//!    names, which this runtime does not read), then the installed shells and the packaged default.
//!
//! This module adds the two things that are about the configuration and not about shells. A selected shell that is
//! not TypeScript or JavaScript is not a configuration source here. When discovery finds no shell at all (a first login before the packaged default is
//! installed), the answer is the file a new user is told to create, `$XDG_CONFIG_HOME/gnoblin/shell.ts`, and
//! [`ConfigPath::exists`] is false: the session starts from the built-in configuration and `gnoblinctl config init`
//! can write a copy there.
//!
//! The environment is passed in rather than read from the process: `set_var` is unsafe in edition 2024 and this
//! workspace forbids unsafe, so a test supplies an [`Environment`] instead of mutating the test runner's environment.
//!
//! This is also the only module that looks at the filesystem before a document exists: the answer includes whether
//! the file is there, which is what tells the session to start from the built-in configuration on first login.

use std::path::{Path, PathBuf};

use gnoblin_shell_discovery::{DiscoveryError, Language as ShellLanguage, Request, discover};
/// The environment a lookup reads, and the tier of the lookup that chose the file.
pub use gnoblin_shell_discovery::{Environment, Tier};

/// The file a new user is told to create.
const NEW_FILE_EXTENSION: &str = "ts";

/// The language a configuration file is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    /// `shell.ts`: TypeScript.
    TypeScript,
    /// `shell.js`: JavaScript.
    JavaScript,
}

impl Language {
    /// The name a log line uses.
    pub fn as_str(self) -> &'static str {
        match self {
            Language::TypeScript => "typescript",
            Language::JavaScript => "javascript",
        }
    }
}

/// The selected configuration file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigPath {
    path: PathBuf,
    tier: Tier,
    exists: bool,
    language: Language,
    warnings: Vec<String>,
}

impl ConfigPath {
    /// The file to load, whether or not it exists yet.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The tier of the lookup that chose the file; for a first login it is [`Tier::UserConfig`], the tier the new
    /// file would be found in.
    pub fn tier(&self) -> Tier {
        self.tier
    }

    /// Whether the file is there now.
    ///
    /// A false answer means first login: the session starts from the built-in
    /// configuration and `gnoblinctl config init` can write a copy at this path.
    pub fn exists(&self) -> bool {
        self.exists
    }

    /// The language of the file, from its extension. For a file that is not there it is the language of the
    /// name that would be created.
    pub fn language(&self) -> Language {
        self.language
    }

    /// What did not stop the lookup but is worth a log line: a `config.toml` key that is not known, a
    /// `config.toml` that was set aside because a higher tier chose the shell.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The path as text, for a log line or a diagnostic.
    pub fn display(&self) -> String {
        self.path.display().to_string()
    }
}

/// Why no configuration file could be named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocateError {
    /// Neither `XDG_CONFIG_HOME` nor `HOME` is usable, so there is no default
    /// location to fall back to.
    NoHome,
    /// Discovery refused: an explicit request that cannot be met, several equally preferred shells, a bad
    /// `config.toml`.
    Discovery(DiscoveryError),
    /// The selected shell is in a language this runtime does not read.
    UnsupportedLanguage { path: PathBuf, language: String },
}

impl std::fmt::Display for LocateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LocateError::NoHome => {
                formatter.write_str("neither XDG_CONFIG_HOME nor HOME is set, so there is no config path")
            }
            LocateError::Discovery(error) => write!(formatter, "{error}"),
            LocateError::UnsupportedLanguage { path, language } => write!(
                formatter,
                "{}: this is a {language} shell, and the configuration runtime reads only TypeScript and JavaScript shells",
                path.display()
            ),
        }
    }
}

impl std::error::Error for LocateError {}

/// Select the configuration file.
///
/// The only filesystem access in the crate outside the last-good store: the
/// answer says which files are there.
///
/// # Errors
///
/// [`LocateError`] when discovery refuses, when there is no home to build the first-login path from, or when the
/// selected shell is not TypeScript or JavaScript.
pub fn locate(environment: &Environment) -> Result<ConfigPath, LocateError> {
    match discover(environment, &Request::Discover) {
        Ok(selection) => {
            let language = match &selection.language {
                ShellLanguage::TypeScript => Language::TypeScript,
                ShellLanguage::JavaScript => Language::JavaScript,
                other => {
                    return Err(LocateError::UnsupportedLanguage {
                        path: selection.path,
                        language: other.to_string(),
                    });
                }
            };
            Ok(ConfigPath {
                tier: selection.chosen_by,
                path: selection.path,
                exists: true,
                language,
                warnings: selection.warnings.iter().map(ToString::to_string).collect(),
            })
        }
        Err(DiscoveryError::NoShell { .. }) => {
            let path = environment
                .user_shell_file(NEW_FILE_EXTENSION)
                .ok_or(LocateError::NoHome)?;
            Ok(ConfigPath {
                path,
                tier: Tier::UserConfig,
                exists: false,
                language: Language::TypeScript,
                warnings: Vec::new(),
            })
        }
        Err(error) => Err(LocateError::Discovery(error)),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    /// An environment with these homes and data directories that hold nothing, so the host's own `/usr/share` is
    /// not consulted.
    fn environment(home: Option<&str>, config_home: Option<&str>) -> Environment {
        Environment {
            home: home.map(PathBuf::from),
            xdg_config_home: config_home.map(PathBuf::from),
            xdg_data_dirs: Some(OsString::from("/nonexistent/share")),
            ..Environment::default()
        }
    }

    #[test]
    fn a_relative_config_home_is_ignored() {
        let found = locate(&environment(Some("/home/user"), Some("relative/config"))).expect("home fallback");
        assert_eq!(found.path(), Path::new("/home/user/.config/gnoblin/shell.ts"));
        assert_eq!(found.tier(), Tier::UserConfig);
        assert!(!found.exists());
    }

    #[test]
    fn without_a_home_or_a_config_home_there_is_no_path() {
        assert_eq!(locate(&environment(None, None)), Err(LocateError::NoHome));
    }

    #[test]
    fn a_relative_home_is_not_a_config_location() {
        assert_eq!(locate(&environment(Some("home/user"), None)), Err(LocateError::NoHome));
    }
}
