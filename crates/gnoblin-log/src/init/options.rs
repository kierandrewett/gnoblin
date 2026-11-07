//! What a process logs and where. The configuration supplies the settings; the environment
//! overrides them for one process.

use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};

use crate::journal::SOCKET_PATH;
use crate::store::{Retention, default_database_path};

/// The environment variable that holds the `tracing` filter, in `EnvFilter` syntax.
///
/// Event targets are component names, so `GNOBLIN_LOG=wayland=debug,warn` shows the debug lines
/// of the `wayland` component and warnings from everywhere else.
pub const FILTER_VARIABLE: &str = "GNOBLIN_LOG";

/// The environment variable that names the database file, or `off` to keep no database.
pub const DATABASE_VARIABLE: &str = "GNOBLIN_LOG_DB";

/// The environment variable that names the console output, one of the [`Console`] names.
pub const FORMAT_VARIABLE: &str = "GNOBLIN_LOG_FORMAT";

/// The variable systemd sets when a service's stderr goes to the journal.
const JOURNAL_STREAM_VARIABLE: &str = "JOURNAL_STREAM";

/// The filter used when neither `GNOBLIN_LOG` nor the configured level names one.
pub(crate) const DEFAULT_FILTER: &str = "info";

/// The console output a process asks for. [`Console::Auto`] is resolved to a
/// [`ConsoleOutput`] from the environment, in the order the variants of
/// [`ConsoleOutput`] describe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Console {
    /// Chosen from the environment: the native journal when systemd connects stderr to it and
    /// the journal socket exists, the `<N>` stream format when only stderr is the journal,
    /// pretty on a terminal, and plain otherwise.
    #[default]
    Auto,
    /// Nothing is printed. The database still records every event.
    Off,
    /// The native journal, with the console printing only if the journal cannot be reached.
    Journal,
    /// `<N>component: message` lines, which systemd reads the priority from.
    JournalStream,
    /// `LEVEL  role/component  message  key=value` lines, with the component in colour on a terminal.
    Pretty,
    /// One JSON object per line.
    Json,
    /// `LEVEL  role/component  message  key=value` lines without colour.
    Plain,
}

impl Console {
    /// The console named by a `GNOBLIN_LOG_FORMAT` value, or `None` for an unknown name.
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "auto" => Self::Auto,
            "off" => Self::Off,
            "journal" => Self::Journal,
            "journal-stream" => Self::JournalStream,
            "pretty" => Self::Pretty,
            "json" => Self::Json,
            "plain" => Self::Plain,
            _ => return None,
        })
    }
}

/// The console output a process uses, once [`Console::Auto`] has been resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleOutput {
    /// Nothing is printed.
    Off,
    /// The native journal, see [`Console::Journal`].
    Journal,
    /// The `<N>` stream format, see [`Console::JournalStream`].
    JournalStream,
    /// Coloured lines on a terminal, see [`Console::Pretty`].
    Pretty,
    /// JSON lines, see [`Console::Json`].
    Json,
    /// Plain lines, see [`Console::Plain`].
    Plain,
}

/// What the process can see of its environment, which [`Console::Auto`] depends on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Probe {
    /// Whether systemd connected stderr to the journal.
    pub journal_stream: bool,
    /// Whether the journal's native socket exists.
    pub journal_socket: bool,
    /// Whether stderr is a terminal.
    pub terminal: bool,
}

impl Probe {
    /// What the real process can see.
    pub fn current() -> Self {
        Self {
            journal_stream: std::env::var_os(JOURNAL_STREAM_VARIABLE).is_some_and(|value| !value.is_empty()),
            journal_socket: Path::new(SOCKET_PATH).exists(),
            terminal: std::io::stderr().is_terminal(),
        }
    }
}

/// The logging settings the configuration supplies. The default is what a process logs with no
/// configuration: `info` on the console and in the database at the default path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogSettings {
    /// The console output, see [`Console`].
    pub console: Console,
    /// The `tracing` filter, used when `GNOBLIN_LOG` is unset or empty. Empty means `info`.
    pub level: String,
    /// Whether events are kept in the database.
    pub store: bool,
    /// The database file, or `None` for the default location. An empty path is `None`.
    pub store_path: Option<PathBuf>,
    /// The limits on what the database keeps.
    pub retention: Retention,
}

impl Default for LogSettings {
    fn default() -> Self {
        Self {
            console: Console::Auto,
            level: DEFAULT_FILTER.to_owned(),
            store: true,
            store_path: None,
            retention: Retention::default(),
        }
    }
}

/// The resolved logging configuration of one process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogOptions {
    /// The `tracing` filter, which governs every console output and the database.
    pub filter: String,
    /// The console output, which is never [`Console::Auto`].
    pub output: ConsoleOutput,
    /// The database file, or `None` when the database is off.
    pub database: Option<PathBuf>,
    /// The limits on what the database keeps.
    pub retention: Retention,
    /// Environment values that were ignored, for the process to report once it can log.
    pub problems: Vec<String>,
}

impl LogOptions {
    /// Resolves the options from the process environment over `settings`.
    pub fn from_environment(settings: &LogSettings) -> Self {
        Self::resolve(|key| std::env::var(key).ok(), settings, Probe::current())
    }

    /// Resolves the options from `lookup`, which answers the value of one variable, over
    /// `settings`, with `probe` deciding [`Console::Auto`].
    ///
    /// An environment variable beats the setting it overrides. An empty variable is the same as
    /// an unset one. With no database path in the environment or the settings, the database goes
    /// to the default path, see [`default_database_path`]; it is off when `store` is false, or
    /// when there is no home to put it under.
    pub fn resolve(lookup: impl Fn(&str) -> Option<String>, settings: &LogSettings, probe: Probe) -> Self {
        let mut problems = Vec::new();
        let filter = non_empty(lookup(FILTER_VARIABLE)).unwrap_or_else(|| {
            if settings.level.is_empty() {
                DEFAULT_FILTER.to_owned()
            } else {
                settings.level.clone()
            }
        });
        let console = match non_empty(lookup(FORMAT_VARIABLE)) {
            Some(name) => Console::parse(&name).unwrap_or_else(|| {
                problems.push(format!(
                    "ignoring {FORMAT_VARIABLE}={name:?}: use auto, off, journal, journal-stream, pretty, json or plain"
                ));
                settings.console
            }),
            None => settings.console,
        };
        let database = match non_empty(lookup(DATABASE_VARIABLE)) {
            Some(value) if value == "off" => None,
            Some(path) => Some(PathBuf::from(path)),
            None if !settings.store => None,
            None => settings
                .store_path
                .clone()
                .filter(|path| !path.as_os_str().is_empty())
                .or_else(|| default_database_path(&lookup)),
        };
        Self {
            filter,
            output: output(console, probe),
            database,
            retention: settings.retention,
            problems,
        }
    }
}

/// The output a console names, with [`Console::Auto`] chosen from `probe`.
fn output(console: Console, probe: Probe) -> ConsoleOutput {
    match console {
        Console::Auto => match (probe.journal_stream, probe.journal_socket, probe.terminal) {
            (true, true, _) => ConsoleOutput::Journal,
            (true, false, _) => ConsoleOutput::JournalStream,
            (false, _, true) => ConsoleOutput::Pretty,
            (false, _, false) => ConsoleOutput::Plain,
        },
        Console::Off => ConsoleOutput::Off,
        Console::Journal => ConsoleOutput::Journal,
        Console::JournalStream => ConsoleOutput::JournalStream,
        Console::Pretty => ConsoleOutput::Pretty,
        Console::Json => ConsoleOutput::Json,
        Console::Plain => ConsoleOutput::Plain,
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn options(pairs: &[(&str, &str)]) -> LogOptions {
        options_with(pairs, &LogSettings::default())
    }

    fn options_with(pairs: &[(&str, &str)], settings: &LogSettings) -> LogOptions {
        options_probed(pairs, settings, Probe::default())
    }

    fn options_probed(pairs: &[(&str, &str)], settings: &LogSettings, probe: Probe) -> LogOptions {
        let environment: HashMap<&str, &str> = pairs.iter().copied().collect();
        LogOptions::resolve(
            |key| environment.get(key).map(|value| (*value).to_owned()),
            settings,
            probe,
        )
    }

    #[test]
    fn an_empty_environment_logs_info_to_the_state_home_of_the_user() {
        assert_eq!(
            options(&[("HOME", "/home/kieran")]),
            LogOptions {
                filter: "info".to_owned(),
                output: ConsoleOutput::Plain,
                database: Some(PathBuf::from("/home/kieran/.local/state/gnoblin/logs.sqlite3")),
                retention: Retention::default(),
                problems: Vec::new(),
            }
        );
    }

    #[test]
    fn the_xdg_state_home_replaces_the_default_state_directory() {
        let resolved = options(&[("HOME", "/home/kieran"), ("XDG_STATE_HOME", "/var/state")]);
        assert_eq!(
            resolved.database,
            Some(PathBuf::from("/var/state/gnoblin/logs.sqlite3"))
        );
    }

    #[test]
    fn with_no_home_and_no_state_home_the_database_is_off() {
        assert_eq!(options(&[]).database, None);
    }

    #[test]
    fn an_empty_variable_is_the_same_as_an_unset_one() {
        let resolved = options(&[("GNOBLIN_LOG", ""), ("GNOBLIN_LOG_DB", ""), ("HOME", "/home/kieran")]);
        assert_eq!(resolved.filter, "info");
        assert_eq!(
            resolved.database,
            Some(PathBuf::from("/home/kieran/.local/state/gnoblin/logs.sqlite3"))
        );
    }

    #[test]
    fn the_filter_and_the_database_path_are_taken_verbatim() {
        let resolved = options(&[
            ("GNOBLIN_LOG", "gnoblin=debug,warn"),
            ("GNOBLIN_LOG_DB", "/tmp/logs.db"),
        ]);
        assert_eq!(resolved.filter, "gnoblin=debug,warn");
        assert_eq!(resolved.database, Some(PathBuf::from("/tmp/logs.db")));
    }

    #[test]
    fn the_database_is_switched_off_by_name() {
        assert_eq!(
            options(&[("GNOBLIN_LOG_DB", "off"), ("HOME", "/home/kieran")]).database,
            None
        );
    }

    #[test]
    fn the_settings_supply_what_the_environment_does_not() {
        let settings = LogSettings {
            console: Console::Off,
            level: "warn".to_owned(),
            store: true,
            store_path: Some(PathBuf::from("/srv/logs.sqlite3")),
            retention: Retention {
                max_age_days: 7,
                max_bytes: Some(1024),
            },
        };
        assert_eq!(
            options_with(&[("HOME", "/home/kieran")], &settings),
            LogOptions {
                filter: "warn".to_owned(),
                output: ConsoleOutput::Off,
                database: Some(PathBuf::from("/srv/logs.sqlite3")),
                retention: settings.retention,
                problems: Vec::new(),
            }
        );
    }

    #[test]
    fn the_environment_beats_the_settings() {
        let settings = LogSettings {
            level: "warn".to_owned(),
            store_path: Some(PathBuf::from("/srv/logs.sqlite3")),
            ..LogSettings::default()
        };
        let resolved = options_with(
            &[("GNOBLIN_LOG", "debug"), ("GNOBLIN_LOG_DB", "/tmp/env.db")],
            &settings,
        );
        assert_eq!(resolved.filter, "debug");
        assert_eq!(resolved.database, Some(PathBuf::from("/tmp/env.db")));
    }

    #[test]
    fn store_false_keeps_no_database_unless_the_environment_names_one() {
        let settings = LogSettings {
            store: false,
            ..LogSettings::default()
        };
        assert_eq!(options_with(&[("HOME", "/home/kieran")], &settings).database, None);
        assert_eq!(
            options_with(&[("HOME", "/home/kieran"), ("GNOBLIN_LOG_DB", "/tmp/x.db")], &settings).database,
            Some(PathBuf::from("/tmp/x.db"))
        );
    }

    #[test]
    fn an_empty_store_path_means_the_default_location() {
        let settings = LogSettings {
            store_path: Some(PathBuf::new()),
            ..LogSettings::default()
        };
        assert_eq!(
            options_with(&[("HOME", "/home/kieran")], &settings).database,
            Some(PathBuf::from("/home/kieran/.local/state/gnoblin/logs.sqlite3"))
        );
    }

    #[test]
    fn a_systemd_service_with_the_journal_socket_logs_to_the_native_journal() {
        let probe = Probe {
            journal_stream: true,
            journal_socket: true,
            terminal: false,
        };
        assert_eq!(
            options_probed(&[], &LogSettings::default(), probe).output,
            ConsoleOutput::Journal
        );
    }

    #[test]
    fn a_systemd_service_without_the_socket_logs_in_the_stream_format() {
        let probe = Probe {
            journal_stream: true,
            journal_socket: false,
            terminal: true,
        };
        assert_eq!(
            options_probed(&[], &LogSettings::default(), probe).output,
            ConsoleOutput::JournalStream
        );
    }

    #[test]
    fn a_terminal_outside_systemd_gets_the_pretty_format() {
        let probe = Probe {
            terminal: true,
            ..Probe::default()
        };
        assert_eq!(
            options_probed(&[], &LogSettings::default(), probe).output,
            ConsoleOutput::Pretty
        );
    }

    #[test]
    fn an_explicit_format_beats_the_settings_and_the_probe() {
        let probe = Probe {
            journal_stream: true,
            journal_socket: true,
            terminal: false,
        };
        let settings = LogSettings {
            console: Console::Off,
            ..LogSettings::default()
        };
        assert_eq!(
            options_probed(&[("GNOBLIN_LOG_FORMAT", "json")], &settings, probe).output,
            ConsoleOutput::Json
        );
    }

    #[test]
    fn every_format_name_is_accepted() {
        for (name, output) in [
            ("off", ConsoleOutput::Off),
            ("journal", ConsoleOutput::Journal),
            ("journal-stream", ConsoleOutput::JournalStream),
            ("pretty", ConsoleOutput::Pretty),
            ("json", ConsoleOutput::Json),
            ("plain", ConsoleOutput::Plain),
        ] {
            assert_eq!(options(&[("GNOBLIN_LOG_FORMAT", name)]).output, output, "{name}");
        }
    }

    #[test]
    fn an_unknown_format_is_ignored_with_one_problem_and_the_settings_decide() {
        let settings = LogSettings {
            console: Console::Json,
            ..LogSettings::default()
        };
        let resolved = options_with(&[("GNOBLIN_LOG_FORMAT", "xml")], &settings);
        assert_eq!(resolved.output, ConsoleOutput::Json);
        assert_eq!(resolved.problems.len(), 1);
        assert!(resolved.problems[0].contains("GNOBLIN_LOG_FORMAT"));
    }
}
