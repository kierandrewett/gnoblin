//! The `keyring` section: whether Gnoblin runs its built-in keyring.
//!
//! `design/keyring.md` ("Coexistence"): Gnoblin serves the Secret Service name and the Secret
//! portal only when `keyring.enabled` is true, which is the default. Gnoblin does not need to know
//! which other program holds the name when it is off. The services role and the portal relay read
//! the same rule from here, so they cannot disagree about it.

use std::path::Path;

use crate::{Config, SearchPaths, Store, locate};

impl Config {
    /// Whether `keyring.enabled` is true. An absent section or field leaves the built-in keyring
    /// on, as the schema default does.
    #[must_use]
    pub fn keyring_enabled(&self) -> bool {
        self.keyring
            .as_ref()
            .and_then(|section| section.enabled)
            .unwrap_or(true)
    }
}

/// Whether the last accepted configuration enables Gnoblin's built-in keyring.
///
/// With no state directory or no configuration file, the keyring stays on, as the default is. A
/// configuration that exists but cannot be read is an error: the caller keeps the default and
/// says why.
///
/// # Errors
///
/// A human-readable description when the last accepted configuration cannot be read.
pub fn keyring_enabled_from_environment() -> Result<bool, String> {
    let Some(store) = Store::from_env() else {
        return Ok(true);
    };
    let Ok(path) = locate(&SearchPaths::from_env()) else {
        return Ok(true);
    };
    keyring_enabled_from_store(&store, path.path())
}

/// [`keyring_enabled_from_environment`] for the document at `source`, read through `store`.
///
/// # Errors
///
/// A human-readable description when the document exists but cannot be read.
pub fn keyring_enabled_from_store(store: &Store, source: &Path) -> Result<bool, String> {
    match store.load(source) {
        Ok(stored) => Ok(stored.config.keyring_enabled()),
        Err(error) if error.is_absent() => Ok(true),
        Err(error) => Err(format!("configuration not readable: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use crate::{Config, Value};

    fn config_with(document: Value) -> Config {
        Config::from_value(document).expect("the document is accepted")
    }

    fn enabled(flag: bool) -> Value {
        Value::table([("keyring", Value::table([("enabled", Value::Bool(flag))]))])
    }

    #[test]
    fn the_built_in_keyring_is_on_when_the_section_is_absent() {
        assert!(config_with(Value::table([])).keyring_enabled());
    }

    #[test]
    fn the_built_in_keyring_is_on_by_default_when_the_section_has_no_field() {
        let document = Value::table([("keyring", Value::table([]))]);
        assert!(config_with(document).keyring_enabled());
    }

    #[test]
    fn the_built_in_keyring_follows_the_enabled_flag() {
        assert!(config_with(enabled(true)).keyring_enabled());
        assert!(!config_with(enabled(false)).keyring_enabled());
    }
}
