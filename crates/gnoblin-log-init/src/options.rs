//! What a process logs and where. The configuration supplies the settings; the environment
//! overrides them for one process.

use std::path::PathBuf;

use gnoblin_log_store::{Retention, default_database_path};

/// The environment variable that holds the `tracing` filter, in `EnvFilter` syntax.
///
/// Event targets are component names, so `GNOBLIN_LOG=wayland=debug,warn` shows the debug lines
/// of the `wayland` component and warnings from everywhere else.
pub const FILTER_VARIABLE: &str = "GNOBLIN_LOG";

/// The environment variable that names the database file, or `off` to keep no database.
pub const DATABASE_VARIABLE: &str = "GNOBLIN_LOG_DB";

/// The filter used when neither `GNOBLIN_LOG` nor the configured level names one.
pub(crate) const DEFAULT_FILTER: &str = "info";

/// The logging settings the configuration supplies. The default is what a process logs with no
/// configuration: `info` on the console and in the database at the default path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogSettings {
    /// Whether each event is printed on stderr.
    pub console: bool,
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
            console: true,
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
    /// The `tracing` filter, which governs both the console and the database.
    pub filter: String,
    /// Whether the console sink prints each event on stderr.
    pub console: bool,
    /// The database file, or `None` when the database is off.
    pub database: Option<PathBuf>,
    /// The limits on what the database keeps.
    pub retention: Retention,
}

impl LogOptions {
    /// Resolves the options from the process environment over `settings`.
    pub fn from_environment(settings: &LogSettings) -> Self {
        Self::resolve(|key| std::env::var(key).ok(), settings)
    }

    /// Resolves the options from `lookup`, which answers the value of one variable, over
    /// `settings`.
    ///
    /// An environment variable beats the setting it overrides. An empty variable is the same as
    /// an unset one. With no database path in the environment or the settings, the database goes
    /// to the default path, see [`default_database_path`]; it is off when `store` is false, or
    /// when there is no home to put it under.
    pub fn resolve(lookup: impl Fn(&str) -> Option<String>, settings: &LogSettings) -> Self {
        let filter = non_empty(lookup(FILTER_VARIABLE)).unwrap_or_else(|| {
            if settings.level.is_empty() {
                DEFAULT_FILTER.to_owned()
            } else {
                settings.level.clone()
            }
        });
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
            console: settings.console,
            database,
            retention: settings.retention,
        }
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
        let environment: HashMap<&str, &str> = pairs.iter().copied().collect();
        LogOptions::resolve(|key| environment.get(key).map(|value| (*value).to_owned()), settings)
    }

    #[test]
    fn an_empty_environment_logs_info_to_the_state_home_of_the_user() {
        assert_eq!(
            options(&[("HOME", "/home/kieran")]),
            LogOptions {
                filter: "info".to_owned(),
                console: true,
                database: Some(PathBuf::from("/home/kieran/.local/state/gnoblin/logs.sqlite3")),
                retention: Retention::default(),
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
            console: false,
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
                console: false,
                database: Some(PathBuf::from("/srv/logs.sqlite3")),
                retention: settings.retention,
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
}
