//! The one place that decides which file is the configuration.
//!
//! The C tree answers "where is the config?" in four places, and `gnoblinctl`
//! still accepts `gnoblin.toml` and `gnoblin.conf`, so the answer depends on
//! which program asked. Here there is one function, one order and one file name:
//!
//! 1. `GNOBLIN_CONFIG`, when it is set and not empty. A test and a nested session
//!    use it to point at a file without touching the user's config. Its extension
//!    names the language: `.ts` or `.mts`, `.js` or `.mjs`, `.lua`.
//! 2. `$XDG_CONFIG_HOME/gnoblin`, when `XDG_CONFIG_HOME` is set to an
//!    absolute path. The XDG base directory specification says a relative value
//!    is invalid, so it is ignored rather than resolved against the working
//!    directory, where a `sudo` or a desktop file would make it mean something
//!    else.
//! 3. `$HOME/.config/gnoblin`.
//!
//! Inside that directory the file is `init.ts`, else `init.js`, else `init.lua`: the
//! first one that is there wins and the others are listed by [`ConfigPath::ignored`],
//! so the caller can warn that they are not read. A directory with none of them
//! names `init.ts`, the file a new user is told to create (`gnoblinctl config init`).
//! [`locate_lua`] looks for `init.lua` alone, for the Lua shell, which cannot read
//! the others.
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

/// The language a configuration file is written in, which decides the shell that reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    /// `init.ts`: TypeScript, run by the JavaScript shell.
    TypeScript,
    /// `init.js`: JavaScript, run by the JavaScript shell.
    JavaScript,
    /// `init.lua`: Lua, run by the Lua shell.
    Lua,
}

impl Language {
    /// The name a log line uses.
    pub fn as_str(self) -> &'static str {
        match self {
            Language::TypeScript => "typescript",
            Language::JavaScript => "javascript",
            Language::Lua => "lua",
        }
    }

    /// Whether the JavaScript shell reads files of this language.
    pub fn is_script(self) -> bool {
        matches!(self, Language::TypeScript | Language::JavaScript)
    }

    /// The language a file extension names, or `None` for an extension no shell reads.
    fn of_extension(extension: &str) -> Option<Language> {
        match extension {
            "ts" | "mts" => Some(Language::TypeScript),
            "js" | "mjs" => Some(Language::JavaScript),
            "lua" => Some(Language::Lua),
            _ => None,
        }
    }

    /// The language of the file at `path`, from its extension.
    fn of_path(path: &Path) -> Option<Language> {
        path.extension()
            .and_then(|extension| extension.to_str())
            .and_then(Language::of_extension)
    }
}

/// The selected configuration file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigPath {
    path: PathBuf,
    origin: Origin,
    exists: bool,
    language: Language,
    ignored: Vec<PathBuf>,
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

    /// The language of the file, from its extension. For a file that is not there it is the language of the
    /// name that would be created.
    pub fn language(&self) -> Language {
        self.language
    }

    /// The configuration files of the same directory that are there and lost to [`path`](Self::path) on the
    /// order `init.ts`, `init.js`, `init.lua`. Nothing reads them, so a caller warns that they are ignored.
    /// Empty for `GNOBLIN_CONFIG`, which names one file.
    pub fn ignored(&self) -> &[PathBuf] {
        &self.ignored
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
    /// The selected file is not one Gnoblin reads. Configuration is TypeScript,
    /// JavaScript or Lua, and guessing at TOML or CONF would half-support a format the
    /// compositor does not read.
    UnsupportedExtension { path: PathBuf },
    /// The selected file is not Lua, and the caller can only read Lua.
    NotLua { path: PathBuf },
}

impl std::fmt::Display for LocateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LocateError::NoHome => {
                formatter.write_str("neither XDG_CONFIG_HOME nor HOME is set, so there is no config path")
            }
            LocateError::UnsupportedExtension { path } => write!(
                formatter,
                "{}: Gnoblin configuration is a .ts, .js or .lua file; convert this file and give it one of those extensions",
                path.display()
            ),
            LocateError::NotLua { path } => write!(
                formatter,
                "{}: the Lua shell reads only .lua configuration; use the JavaScript shell for this file",
                path.display()
            ),
        }
    }
}

impl std::error::Error for LocateError {}

/// The file names inside the config directory, in the order the first one that is there wins.
const CONFIG_FILE_NAMES: [(&str, Language); 3] = [
    ("init.ts", Language::TypeScript),
    ("init.js", Language::JavaScript),
    ("init.lua", Language::Lua),
];

/// The directory Gnoblin owns under the config home.
const CONFIG_DIRECTORY_NAME: &str = "gnoblin";

/// The directory the config files live in, or the override file itself.
fn base(paths: &SearchPaths) -> Result<(PathBuf, Origin), LocateError> {
    if let Some(override_path) = &paths.override_path {
        return Ok((override_path.clone(), Origin::Override));
    }
    if let Some(config_home) = paths
        .config_home
        .as_ref()
        .filter(|config_home| config_home.is_absolute())
    {
        return Ok((config_home.join(CONFIG_DIRECTORY_NAME), Origin::ConfigHome));
    }
    let home = paths
        .home
        .as_ref()
        .filter(|home| home.is_absolute())
        .ok_or(LocateError::NoHome)?;
    Ok((home.join(".config").join(CONFIG_DIRECTORY_NAME), Origin::Home))
}

/// Select the configuration file.
///
/// The only filesystem access in the crate outside the last-good store: the
/// answer says which files are there.
///
/// # Errors
///
/// [`LocateError`] when there is no home, or `GNOBLIN_CONFIG` names a file with an extension no shell reads.
pub fn locate(paths: &SearchPaths) -> Result<ConfigPath, LocateError> {
    let (path, origin) = base(paths)?;
    if origin == Origin::Override {
        let language =
            Language::of_path(&path).ok_or_else(|| LocateError::UnsupportedExtension { path: path.clone() })?;
        let exists = path.is_file();
        return Ok(ConfigPath {
            path,
            origin,
            exists,
            language,
            ignored: Vec::new(),
        });
    }
    let mut present = CONFIG_FILE_NAMES
        .iter()
        .map(|(name, language)| (path.join(name), *language))
        .filter(|(file, _)| file.is_file());
    match present.next() {
        Some((chosen, language)) => Ok(ConfigPath {
            path: chosen,
            origin,
            exists: true,
            language,
            ignored: present.map(|(file, _)| file).collect(),
        }),
        None => Ok(ConfigPath {
            path: path.join(CONFIG_FILE_NAMES[0].0),
            origin,
            exists: false,
            language: CONFIG_FILE_NAMES[0].1,
            ignored: Vec::new(),
        }),
    }
}

/// Select the configuration file for the Lua shell: `init.lua` in the directory [`locate`] would use, or the
/// `GNOBLIN_CONFIG` file when it is Lua. A `init.ts` beside it is not this shell's and is not reported.
///
/// # Errors
///
/// [`LocateError`] when there is no home, or `GNOBLIN_CONFIG` names a file that is not Lua.
pub fn locate_lua(paths: &SearchPaths) -> Result<ConfigPath, LocateError> {
    let (path, origin) = base(paths)?;
    let path = match origin {
        Origin::Override => {
            if Language::of_path(&path) != Some(Language::Lua) {
                return Err(match Language::of_path(&path) {
                    Some(_) => LocateError::NotLua { path },
                    None => LocateError::UnsupportedExtension { path },
                });
            }
            path
        }
        _ => path.join("init.lua"),
    };
    let exists = path.is_file();
    Ok(ConfigPath {
        path,
        origin,
        exists,
        language: Language::Lua,
        ignored: Vec::new(),
    })
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
        assert_eq!(found.path(), Path::new("/home/user/.config/gnoblin/init.ts"));
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

    #[test]
    fn an_override_names_its_language_by_extension() {
        for (name, language) in [
            ("a.ts", Language::TypeScript),
            ("a.mts", Language::TypeScript),
            ("a.js", Language::JavaScript),
            ("a.mjs", Language::JavaScript),
            ("a.lua", Language::Lua),
        ] {
            let paths = SearchPaths {
                override_path: Some(PathBuf::from("/nowhere").join(name)),
                ..SearchPaths::default()
            };
            assert_eq!(
                locate(&paths).expect("a supported extension").language(),
                language,
                "{name}"
            );
        }
    }
}
