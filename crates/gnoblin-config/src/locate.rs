//! The one place that decides which file is the configuration.
//!
//! The C tree answers "where is the config?" in four places, and `gnoblinctl`
//! still accepts `gnoblin.toml` and `gnoblin.conf`, so the answer depends on
//! which program asked. Here there is one function, one order and one file name:
//!
//! 1. `GNOBLIN_CONFIG`, when it is set and not empty. A test and a nested session
//!    use it to point at a file without touching the user's config. Its extension
//!    names the language: `.ts` or `.mts`, `.js` or `.mjs`. A `.lua` file is refused
//!    with the command that converts it.
//! 2. `$XDG_CONFIG_HOME/gnoblin`, when `XDG_CONFIG_HOME` is set to an
//!    absolute path. The XDG base directory specification says a relative value
//!    is invalid, so it is ignored rather than resolved against the working
//!    directory, where a `sudo` or a desktop file would make it mean something
//!    else.
//! 3. `$HOME/.config/gnoblin`.
//!
//! Inside that directory the file is `init.ts`, else `init.js`: the first one that is
//! there wins and the others are listed by [`ConfigPath::ignored`], so the caller can
//! warn that they are not read. A directory with neither names `init.ts`, the file a
//! new user is told to create (`gnoblinctl config init`). An `init.lua` is never read;
//! [`ConfigPath::lua`] says when it is the only configuration there, so the caller can tell
//! the user to convert it.
//!
//! The environment is passed in rather than read from the process: `set_var` is
//! unsafe in edition 2024 and this workspace forbids unsafe, so a test supplies a
//! [`SearchPaths`] instead of mutating the test runner's environment.
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

/// The language a configuration file is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    /// `init.ts`: TypeScript.
    TypeScript,
    /// `init.js`: JavaScript.
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

    /// The language a file extension names, or `None` for an extension no shell reads.
    fn of_extension(extension: &str) -> Option<Language> {
        match extension {
            "ts" | "mts" => Some(Language::TypeScript),
            "js" | "mjs" => Some(Language::JavaScript),
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
    lua: Option<PathBuf>,
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
    /// order `init.ts`, `init.js`, and then a Lua `init.lua`, which nothing reads. A caller warns that they are ignored.
    /// Empty for `GNOBLIN_CONFIG`, which names one file.
    pub fn ignored(&self) -> &[PathBuf] {
        &self.ignored
    }

    /// The `init.lua` of the directory when it is the only configuration there: there is no `init.ts` or `init.js`
    /// beside it, so the session runs the stock configuration and the caller tells the user to convert the Lua file.
    pub fn lua(&self) -> Option<&Path> {
        self.lua.as_deref()
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
    /// The selected file is not one Gnoblin reads. Configuration is TypeScript or
    /// JavaScript, and guessing at TOML or CONF would half-support a format the
    /// compositor does not read.
    UnsupportedExtension { path: PathBuf },
    /// The selected file is Lua, which nothing reads any more. It is converted with `gnoblinctl config migrate`.
    LuaFile { path: PathBuf },
}

impl std::fmt::Display for LocateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LocateError::NoHome => {
                formatter.write_str("neither XDG_CONFIG_HOME nor HOME is set, so there is no config path")
            }
            LocateError::UnsupportedExtension { path } => write!(
                formatter,
                "{}: Gnoblin configuration is a .ts or .js file; convert this file and give it one of those extensions",
                path.display()
            ),
            LocateError::LuaFile { path } => write!(
                formatter,
                "{}: Gnoblin no longer runs Lua; convert it with `gnoblinctl config migrate` (see docs/migrating-from-lua.md)",
                path.display()
            ),
        }
    }
}

/// The file names inside the config directory, in the order the first one that is there wins.
const CONFIG_FILE_NAMES: [(&str, Language); 2] = [("init.ts", Language::TypeScript), ("init.js", Language::JavaScript)];

/// The file name a Lua configuration had. Nothing reads it; the lookup only reports that it is there, so the caller can
/// tell the user how to convert it.
const LUA_FILE_NAME: &str = "init.lua";

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
        if path.extension().is_some_and(|extension| extension == "lua") {
            return Err(LocateError::LuaFile { path });
        }
        let language =
            Language::of_path(&path).ok_or_else(|| LocateError::UnsupportedExtension { path: path.clone() })?;
        let exists = path.is_file();
        return Ok(ConfigPath {
            path,
            origin,
            exists,
            language,
            ignored: Vec::new(),
            lua: None,
        });
    }
    let lua = Some(path.join(LUA_FILE_NAME)).filter(|file| file.is_file());
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
            ignored: present.map(|(file, _)| file).chain(lua).collect(),
            lua: None,
        }),
        None => Ok(ConfigPath {
            path: path.join(CONFIG_FILE_NAMES[0].0),
            origin,
            exists: false,
            language: CONFIG_FILE_NAMES[0].1,
            ignored: Vec::new(),
            lua,
        }),
    }
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
