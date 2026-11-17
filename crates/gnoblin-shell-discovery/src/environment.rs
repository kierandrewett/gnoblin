//! The environment a lookup reads, and the XDG rules that turn it into directories.
//!
//! The process environment is captured once into an [`Environment`] and passed in. `set_var` is unsafe in edition
//! 2024 and this workspace forbids unsafe code, so a test supplies an `Environment` instead of changing the test
//! runner's environment, and a launcher decides which of its own facts (the library directory, the install prefix)
//! to add.
//!
//! The XDG base directory specification says a relative `XDG_*_HOME` is invalid, so it is ignored rather than
//! resolved against the working directory, where a `sudo` or a desktop file would make it mean something else. A
//! relative entry of `XDG_DATA_DIRS` is ignored for the same reason.

use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

/// The directory Gnoblin owns under every XDG base directory.
const DIRECTORY_NAME: &str = "gnoblin";

/// The file name of the shell settings under the config directory.
pub const CONFIG_FILE_NAME: &str = "config.toml";

/// The directory of installed shells under a `gnoblin` data directory.
pub const SHELLS_DIRECTORY: &str = "shells";

/// The directory of the packaged default shell under the install prefix's `share/gnoblin`.
pub const DEFAULT_SHELL_DIRECTORY: &str = "default-shell";

/// The file stem of a conventional user shell and of the shell inside a shell directory: `shell.ts`.
pub const SHELL_STEM: &str = "shell";

/// The file a Lua configuration was written in. Nothing reads it; discovery only notices it, so the user is told to convert it.
pub const LUA_CONFIG_FILE: &str = "init.lua";

/// What the XDG specification says `XDG_DATA_DIRS` is when it is unset or empty.
const DEFAULT_DATA_DIRS: &str = "/usr/local/share:/usr/share";

/// The facts a lookup reads, captured so a test can supply them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Environment {
    /// `$HOME`, which the XDG fallbacks are built from.
    pub home: Option<PathBuf>,
    /// `$XDG_CONFIG_HOME`, used only when it is an absolute path.
    pub xdg_config_home: Option<PathBuf>,
    /// `$XDG_DATA_HOME`, used only when it is an absolute path.
    pub xdg_data_home: Option<PathBuf>,
    /// `$XDG_DATA_DIRS`, a colon separated list. Unset or empty means `/usr/local/share:/usr/share`.
    pub xdg_data_dirs: Option<OsString>,
    /// `$GNOBLIN_SHELL`. Unset or empty means the tier is skipped.
    pub gnoblin_shell: Option<OsString>,
    /// The working directory a relative path from the command line or `GNOBLIN_SHELL` is relative to. When it is
    /// `None` such a path is returned as it was given and the process's own working directory applies.
    pub working_dir: Option<PathBuf>,
    /// The directory of the packaged default shell, `<prefix>/share/gnoblin/default-shell`, where `<prefix>` comes
    /// from the executable's location. It is not looked up through `XDG_DATA_DIRS`, so a relocated or `/opt` prefix
    /// still finds it. `None` means there is none to look in.
    pub packaged_default_dir: Option<PathBuf>,
    /// The directory a launcher file's `module` key is relative to, `<core libdir>/gnoblin/shells`. Only the code
    /// that knows where the core library was loaded from can say. `None` makes a launcher with a `module` key
    /// unusable, and says so.
    pub module_dir: Option<PathBuf>,
}

impl Environment {
    /// Read the variables, the working directory and the executable's location from the running process.
    ///
    /// [`module_dir`](Self::module_dir) stays `None`: the launcher sets it from where the core library is mapped.
    pub fn from_env() -> Environment {
        let variable = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
        Environment {
            home: variable("HOME").map(PathBuf::from),
            xdg_config_home: variable("XDG_CONFIG_HOME").map(PathBuf::from),
            xdg_data_home: variable("XDG_DATA_HOME").map(PathBuf::from),
            xdg_data_dirs: variable("XDG_DATA_DIRS"),
            gnoblin_shell: variable("GNOBLIN_SHELL"),
            working_dir: std::env::current_dir().ok(),
            packaged_default_dir: std::env::current_exe()
                .ok()
                .and_then(|executable| packaged_default_dir_of(&executable)),
            module_dir: None,
        }
    }

    /// `$XDG_CONFIG_HOME/gnoblin`, else `$HOME/.config/gnoblin`; `None` when neither is usable.
    pub fn config_dir(&self) -> Option<PathBuf> {
        base(&self.xdg_config_home, &self.home, ".config").map(|dir| dir.join(DIRECTORY_NAME))
    }

    /// The `config.toml` that holds the `[shell]` section.
    pub fn config_toml(&self) -> Option<PathBuf> {
        self.config_dir().map(|dir| dir.join(CONFIG_FILE_NAME))
    }

    /// The conventional user shell of one extension: `<config dir>/shell.<extension>`. `gnoblinctl config init`
    /// creates `shell.ts` here.
    pub fn user_shell_file(&self, extension: &str) -> Option<PathBuf> {
        self.config_dir()
            .map(|dir| dir.join(format!("{SHELL_STEM}.{extension}")))
    }

    /// `$XDG_DATA_HOME/gnoblin/shells`, else `$HOME/.local/share/gnoblin/shells`.
    pub fn user_shells_dir(&self) -> Option<PathBuf> {
        base(&self.xdg_data_home, &self.home, ".local/share").map(|dir| dir.join(DIRECTORY_NAME).join(SHELLS_DIRECTORY))
    }

    /// `<entry>/gnoblin/shells` for each absolute entry of `XDG_DATA_DIRS` in order, with repeats dropped.
    pub fn system_shells_dirs(&self) -> Vec<PathBuf> {
        let list = match &self.xdg_data_dirs {
            Some(list) if !list.is_empty() => list.as_os_str(),
            _ => OsStr::new(DEFAULT_DATA_DIRS),
        };
        let mut directories: Vec<PathBuf> = Vec::new();
        for entry in std::env::split_paths(list) {
            if !entry.is_absolute() {
                continue;
            }
            let directory = entry.join(DIRECTORY_NAME).join(SHELLS_DIRECTORY);
            if !directories.contains(&directory) {
                directories.push(directory);
            }
        }
        directories
    }

    /// The path made absolute against [`working_dir`](Self::working_dir) with `.` components removed. A relative
    /// path with no working directory to anchor it is returned as it was.
    pub fn absolute(&self, path: &Path) -> PathBuf {
        let joined = match (&self.working_dir, path.is_absolute()) {
            (Some(working_dir), false) => working_dir.join(path),
            _ => path.to_path_buf(),
        };
        clean(&joined)
    }

    /// `~` and `~/rest` made absolute against `$HOME`. `None` when the text starts with `~` and there is no home.
    pub(crate) fn expand_home(&self, text: &str) -> Option<PathBuf> {
        let home = || self.home.as_ref().filter(|home| home.is_absolute());
        if text == "~" {
            return home().cloned();
        }
        match text.strip_prefix("~/") {
            Some(rest) => home().map(|home| home.join(rest)),
            None => Some(PathBuf::from(text)),
        }
    }
}

/// `<prefix>/share/gnoblin/default-shell` for an executable at `<prefix>/bin/<name>`.
fn packaged_default_dir_of(executable: &Path) -> Option<PathBuf> {
    let prefix = executable.parent()?.parent()?;
    Some(prefix.join("share").join(DIRECTORY_NAME).join(DEFAULT_SHELL_DIRECTORY))
}

/// The XDG base directory: the `*_HOME` variable when it is absolute, else `$HOME/<fallback>` when `$HOME` is.
fn base(xdg: &Option<PathBuf>, home: &Option<PathBuf>, fallback: &str) -> Option<PathBuf> {
    if let Some(xdg) = xdg.as_ref().filter(|xdg| xdg.is_absolute()) {
        return Some(xdg.clone());
    }
    home.as_ref()
        .filter(|home| home.is_absolute())
        .map(|home| home.join(fallback))
}

/// The path without its `.` components. `..` is kept: removing it would change what a symlink means.
fn clean(path: &Path) -> PathBuf {
    path.components()
        .filter(|component| !matches!(component, Component::CurDir))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment() -> Environment {
        Environment {
            home: Some(PathBuf::from("/home/k")),
            ..Environment::default()
        }
    }

    #[test]
    fn the_config_dir_falls_back_to_dot_config() {
        assert_eq!(
            environment().config_dir(),
            Some(PathBuf::from("/home/k/.config/gnoblin"))
        );
    }

    #[test]
    fn an_absolute_config_home_wins_over_home() {
        let environment = Environment {
            xdg_config_home: Some(PathBuf::from("/cfg")),
            ..environment()
        };
        assert_eq!(environment.config_dir(), Some(PathBuf::from("/cfg/gnoblin")));
    }

    #[test]
    fn a_relative_config_home_is_ignored() {
        let environment = Environment {
            xdg_config_home: Some(PathBuf::from("relative/cfg")),
            ..environment()
        };
        assert_eq!(environment.config_dir(), Some(PathBuf::from("/home/k/.config/gnoblin")));
    }

    #[test]
    fn without_a_usable_home_there_is_no_user_directory() {
        let relative_home = Environment {
            home: Some(PathBuf::from("home/k")),
            ..Environment::default()
        };
        assert_eq!(relative_home.config_dir(), None);
        assert_eq!(relative_home.user_shells_dir(), None);
        assert_eq!(Environment::default().config_toml(), None);
    }

    #[test]
    fn the_user_shells_dir_follows_the_data_home() {
        assert_eq!(
            environment().user_shells_dir(),
            Some(PathBuf::from("/home/k/.local/share/gnoblin/shells"))
        );
        let environment = Environment {
            xdg_data_home: Some(PathBuf::from("/data")),
            ..environment()
        };
        assert_eq!(
            environment.user_shells_dir(),
            Some(PathBuf::from("/data/gnoblin/shells"))
        );
    }

    #[test]
    fn data_dirs_default_when_unset_or_empty() {
        let expected = vec![
            PathBuf::from("/usr/local/share/gnoblin/shells"),
            PathBuf::from("/usr/share/gnoblin/shells"),
        ];
        assert_eq!(Environment::default().system_shells_dirs(), expected);
        let empty = Environment {
            xdg_data_dirs: Some(OsString::new()),
            ..Environment::default()
        };
        assert_eq!(empty.system_shells_dirs(), expected);
    }

    #[test]
    fn data_dirs_keep_order_and_drop_relative_empty_and_repeated_entries() {
        let environment = Environment {
            xdg_data_dirs: Some(OsString::from("/b:relative:/a::/b:/c/")),
            ..Environment::default()
        };
        assert_eq!(
            environment.system_shells_dirs(),
            vec![
                PathBuf::from("/b/gnoblin/shells"),
                PathBuf::from("/a/gnoblin/shells"),
                PathBuf::from("/c/gnoblin/shells"),
            ]
        );
    }

    #[test]
    fn a_data_dirs_of_only_relative_entries_names_no_directory() {
        let environment = Environment {
            xdg_data_dirs: Some(OsString::from("a:b")),
            ..Environment::default()
        };
        assert!(environment.system_shells_dirs().is_empty());
    }

    #[test]
    fn absolute_joins_the_working_dir_and_drops_dot_components() {
        let environment = Environment {
            working_dir: Some(PathBuf::from("/work")),
            ..Environment::default()
        };
        assert_eq!(
            environment.absolute(Path::new("./a/./b.so")),
            PathBuf::from("/work/a/b.so")
        );
        assert_eq!(environment.absolute(Path::new("/x/./y")), PathBuf::from("/x/y"));
        assert_eq!(
            Environment::default().absolute(Path::new("./a.so")),
            PathBuf::from("a.so")
        );
    }

    #[test]
    fn the_packaged_default_follows_the_executable() {
        assert_eq!(
            packaged_default_dir_of(Path::new("/opt/gnoblin/bin/gnoblin")),
            Some(PathBuf::from("/opt/gnoblin/share/gnoblin/default-shell"))
        );
        assert_eq!(packaged_default_dir_of(Path::new("gnoblin")), None);
    }

    #[test]
    fn home_expansion_needs_an_absolute_home() {
        let environment = environment();
        assert_eq!(environment.expand_home("~/a/b"), Some(PathBuf::from("/home/k/a/b")));
        assert_eq!(environment.expand_home("~"), Some(PathBuf::from("/home/k")));
        assert_eq!(environment.expand_home("a/b"), Some(PathBuf::from("a/b")));
        assert_eq!(Environment::default().expand_home("~/a"), None);
    }
}
