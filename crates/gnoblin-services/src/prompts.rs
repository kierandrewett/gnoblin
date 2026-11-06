//! Whether the services role serves passphrase prompts, from `prompts.enabled`.
//!
//! `docs/config/configure/prompts.md` says the prompter names stay unowned while the setting is
//! `false`, which is the default. The services role reads it here, next to the location policy,
//! so the session binary does not need its own dependency on the configuration crate.

use std::path::Path;

use gnoblin_config::{SearchPaths, Store};
use tracing::{debug, warn};

/// Whether the configuration turns the prompter on: `prompts.enabled`, which defaults to `false`.
/// With no state directory, no file or an unreadable file the prompter stays off.
pub fn enabled_from_environment() -> bool {
    let Some(store) = Store::from_env() else {
        debug!("[services] no state directory is set; prompts are off");
        return false;
    };
    match gnoblin_config::locate(&SearchPaths::from_env()) {
        Ok(path) => enabled_from_store(&store, path.path()),
        Err(error) => {
            debug!(%error, "[services] no configuration file; prompts are off");
            false
        }
    }
}

/// [`enabled_from_environment`] for the document at `source`, read through `store`.
pub fn enabled_from_store(store: &Store, source: &Path) -> bool {
    match store.load(source) {
        Ok(stored) => stored
            .config
            .prompts
            .as_ref()
            .and_then(|section| section.enabled)
            .unwrap_or(false),
        Err(error) => {
            warn!(%error, "[services] configuration not readable; prompts are off");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use gnoblin_config::{Config, Value};

    fn enabled_in(document: Value) -> bool {
        Config::from_value(document)
            .expect("the document is accepted")
            .prompts
            .as_ref()
            .and_then(|section| section.enabled)
            .unwrap_or(false)
    }

    #[test]
    fn prompts_are_off_when_the_section_is_absent() {
        assert!(!enabled_in(Value::table([])));
    }

    #[test]
    fn prompts_are_on_only_when_enabled_is_true() {
        let on = Value::table([("prompts", Value::table([("enabled", Value::Bool(true))]))]);
        let off = Value::table([("prompts", Value::table([("enabled", Value::Bool(false))]))]);
        assert!(enabled_in(on));
        assert!(!enabled_in(off));
    }
}
