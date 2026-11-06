//! The one place that decides which file is the configuration.
//!
//! The C tree answers "where is the config?" in four places, and `gnoblinctl`
//! still accepts `gnoblin.toml` and `gnoblin.conf`, so the answer depends on
//! which program asked. Here there is one function, one order and one file name:
//!
//! 1. `GNOBLIN_CONFIG`, when it is set and not empty. A test and a nested session
//!    use it to point at a file without touching the user's config.
//! 2. `$XDG_CONFIG_HOME/gnoblin/init.lua`, when `XDG_CONFIG_HOME` is set to an
//!    absolute path. The XDG base directory specification says a relative value
//!    is invalid, so it is ignored rather than resolved against the working
//!    directory, where a `sudo` or a desktop file would make it mean something
//!    else.
//! 3. `$HOME/.config/gnoblin/init.lua`.
//!
//! The environment is passed in rather than read from the process: `set_var` is
//! unsafe in edition 2024 and this workspace forbids unsafe, so a test supplies
//! a [`SearchPaths`] instead of mutating the test runner's environment.
//!
//! This is also the only module that looks at the filesystem before a document
//! exists: the answer includes whether the file is there, which is what tells
//! the session to start from the built-in configuration on first login.

use std::path::{Path, PathBuf};

/// The environment a lookup reads, captured so it can be supplied by a test.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchPaths {
    /// `$HOME`, which the fallback path is built from.
    pub home: Option<PathBuf>,
    /// `$XDG_CONFIG_HOME`, used only when it is an absolute path.
    pub config_home: Option<PathBuf>,
    /// `$GNOBLIN_CONFIG`, used only when it is set and not empty.
    pub override_path: Option<PathBuf>,
}

impl SearchPaths {
    /// Read the three variables from the process environment.
    pub fn from_env() -> SearchPaths {
        let absolute = |name: &str| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|value| !value.as_os_str().is_empty())
        };
        SearchPaths {
            home: absolute("HOME"),
            config_home: absolute("XDG_CONFIG_HOME"),
            override_path: absolute("GNOBLIN_CONFIG"),
        }
    }
}

/// Where the config path came from, which the status and the log name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// `GNOBLIN_CONFIG`.
    Override,
    /// `$XDG_CONFIG_HOME/gnoblin`.
    ConfigHome,
    /// `$HOME/.config/gnoblin`.
    Home,
}

impl Origin {
    /// The name the object model publishes for this origin.
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Override => "override",
            Origin::ConfigHome => "xdg-config-home",
            Origin::Home => "home",
        }
    }
}

/// The selected configuration file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigPath {
    path: PathBuf,
    origin: Origin,
    exists: bool,
}

impl ConfigPath {
    /// The file to load, whether or not it exists yet.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The variable or directory the path was built from.
    pub fn origin(&self) -> Origin {
        self.origin
    }

    /// Whether the file is there now.
    ///
    /// A false answer means first login: the session starts from the built-in
    /// configuration and `gnoblinctl init` can write a copy at this path.
    pub fn exists(&self) -> bool {
        self.exists
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
    /// The selected file is not a Lua file. Configuration is Lua only, and
    /// guessing at TOML or CONF would half-support a format the compositor does
    /// not read.
    UnsupportedExtension { path: PathBuf },
}

impl std::fmt::Display for LocateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LocateError::NoHome => {
                formatter.write_str("neither XDG_CONFIG_HOME nor HOME is set, so there is no config path")
            }
            LocateError::UnsupportedExtension { path } => write!(
                formatter,
                "{}: Gnoblin configuration uses Lua only; convert this file to Lua and give it a .lua filename",
                path.display()
            ),
        }
    }
}

impl std::error::Error for LocateError {}

/// The Lua file name inside the config directory.
const CONFIG_FILE_NAME: &str = "init.lua";

/// The directory Gnoblin owns under the config home.
const CONFIG_DIRECTORY_NAME: &str = "gnoblin";

/// Select the configuration file.
///
/// The only filesystem access in the crate outside the last-good store: the
/// answer says whether the file is there.
pub fn locate(paths: &SearchPaths) -> Result<ConfigPath, LocateError> {
    let (path, origin) = match &paths.override_path {
        Some(override_path) => (override_path.clone(), Origin::Override),
        None => {
            let config_home = match &paths.config_home {
                Some(config_home) if config_home.is_absolute() => Some((config_home.clone(), Origin::ConfigHome)),
                _ => None,
            };
            match config_home {
                Some(found) => found,
                None => {
                    let Some(home) = paths.home.as_ref().filter(|home| home.is_absolute()) else {
                        return Err(LocateError::NoHome);
                    };
                    (home.join(".config"), Origin::Home)
                }
            }
        }
    };
    let path = match origin {
        Origin::Override => path,
        _ => path.join(CONFIG_DIRECTORY_NAME).join(CONFIG_FILE_NAME),
    };
    if !has_lua_extension(&path) {
        return Err(LocateError::UnsupportedExtension { path });
    }
    let exists = path.is_file();
    Ok(ConfigPath { path, origin, exists })
}

/// Whether the path ends in `.lua`.
fn has_lua_extension(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "lua")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_config_home_is_ignored() {
        let paths = SearchPaths {
            home: Some(PathBuf::from("/home/user")),
            config_home: Some(PathBuf::from("relative/config")),
            override_path: None,
        };
        let found = locate(&paths).expect("home fallback");
        assert_eq!(found.path(), Path::new("/home/user/.config/gnoblin/init.lua"));
        assert_eq!(found.origin(), Origin::Home);
    }

    #[test]
    fn without_a_home_or_a_config_home_there_is_no_path() {
        assert_eq!(locate(&SearchPaths::default()), Err(LocateError::NoHome));
    }

    #[test]
    fn a_relative_home_is_not_a_config_location() {
        let paths = SearchPaths {
            home: Some(PathBuf::from("home/user")),
            ..SearchPaths::default()
        };
        assert_eq!(locate(&paths), Err(LocateError::NoHome));
    }
}
