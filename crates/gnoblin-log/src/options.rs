//! What a process logs and where, read once from its environment.

use std::path::PathBuf;

/// The environment variable that holds the `tracing` filter, in `EnvFilter` syntax.
pub const FILTER_VARIABLE: &str = "GNOBLIN_LOG";

/// The environment variable that names the database file, or `off` to keep no database.
pub const DATABASE_VARIABLE: &str = "GNOBLIN_LOG_DB";

/// The filter used when `GNOBLIN_LOG` is unset or empty.
pub(crate) const DEFAULT_FILTER: &str = "info";

/// The subdirectory of the state home that holds the database.
const STATE_DIRECTORY: &str = "gnoblin";

/// The database file name inside [`STATE_DIRECTORY`].
const DATABASE_FILE: &str = "logs.sqlite3";

/// The resolved logging configuration of one process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogOptions {
    /// The `tracing` filter, which governs both the console and the database.
    pub filter: String,
    /// The database file, or `None` when the database is off.
    pub database: Option<PathBuf>,
}

impl LogOptions {
    /// Reads the options from the process environment.
    pub fn from_env() -> Self {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Resolves the options from `lookup`, which answers the value of one variable.
    ///
    /// An empty variable is the same as an unset one. The database defaults to
    /// `$XDG_STATE_HOME/gnoblin/logs.sqlite3`, falling back to `$HOME/.local/state`, and
    /// is off when neither home is set.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let filter = non_empty(lookup(FILTER_VARIABLE)).unwrap_or_else(|| DEFAULT_FILTER.to_owned());
        let database = match non_empty(lookup(DATABASE_VARIABLE)) {
            Some(value) if value == "off" => None,
            Some(path) => Some(PathBuf::from(path)),
            None => default_database(&lookup),
        };
        Self { filter, database }
    }
}

/// The default database path, or `None` when there is no home to put it under.
fn default_database(lookup: &impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let state_home = match non_empty(lookup("XDG_STATE_HOME")) {
        Some(state_home) => PathBuf::from(state_home),
        None => PathBuf::from(non_empty(lookup("HOME"))?).join(".local").join("state"),
    };
    Some(state_home.join(STATE_DIRECTORY).join(DATABASE_FILE))
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn options(pairs: &[(&str, &str)]) -> LogOptions {
        let environment: HashMap<&str, &str> = pairs.iter().copied().collect();
        LogOptions::from_lookup(|key| environment.get(key).map(|value| (*value).to_owned()))
    }

    #[test]
    fn an_empty_environment_logs_info_to_the_state_home_of_the_user() {
        assert_eq!(
            options(&[("HOME", "/home/kieran")]),
            LogOptions {
                filter: "info".to_owned(),
                database: Some(PathBuf::from("/home/kieran/.local/state/gnoblin/logs.sqlite3")),
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
}
