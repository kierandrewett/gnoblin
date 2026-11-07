//! The Secret Service the services role serves, when `keyring.provider` names Gnoblin.
//!
//! `design/keyring.md` ("Coexistence"): Gnoblin takes `org.freedesktop.secrets` only when it is the
//! configured provider. Any other name leaves the name to that program, and the role does not
//! request it. The default is `gnoblin`, so an absent section keeps Gnoblin as the provider.

use std::path::{Path, PathBuf};

use gnoblin_config::{Config, SearchPaths, Store};

use crate::link::{Host, LinkedAgent, RequestId, RequestOutcome, Tick};
use crate::log;

/// The provider name that selects Gnoblin's own Secret Service.
pub const GNOBLIN_PROVIDER: &str = "gnoblin";

/// Whether `keyring.provider` names Gnoblin. An absent section or field leaves Gnoblin as the
/// provider, as the schema default does.
#[must_use]
pub fn provider_is_gnoblin(config: &Config) -> bool {
    config
        .keyring
        .as_ref()
        .and_then(|section| section.provider.as_deref())
        .is_none_or(|provider| provider == GNOBLIN_PROVIDER)
}

/// Whether the environment's configuration names Gnoblin as the keyring provider. With no state
/// directory, no file or an unreadable file, Gnoblin stays the provider, as the default is.
pub fn enabled_from_environment() -> bool {
    let Some(store) = Store::from_env() else {
        log::debug!("no state directory is set; keyring provider is the default");
        return true;
    };
    match gnoblin_config::locate(&SearchPaths::from_env()) {
        Ok(path) => enabled_from_store(&store, path.path()),
        Err(error) => {
            log::debug!(%error, "no configuration file; keyring provider is the default");
            true
        }
    }
}

/// [`enabled_from_environment`] for the document at `source`, read through `store`.
pub fn enabled_from_store(store: &Store, source: &Path) -> bool {
    match store.load(source) {
        Ok(stored) => provider_is_gnoblin(&stored.config),
        Err(error) => {
            log::warn!(%error, "configuration not readable; keyring provider is the default");
            true
        }
    }
}

/// The store the Secret Service keeps its items in: `$XDG_DATA_HOME/gnoblin/keyring.db`, or under
/// `$HOME/.local/share` when `XDG_DATA_HOME` is unset. `None` when neither is an absolute path.
#[must_use]
pub fn store_path_from_environment() -> Option<PathBuf> {
    let absolute = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let data_home = absolute("XDG_DATA_HOME").or_else(|| absolute("HOME").map(|home| home.join(".local/share")))?;
    Some(data_home.join("gnoblin").join("keyring.db"))
}

/// The hand-off socket the unlock path listens on. It is `$XDG_RUNTIME_DIR` when that is absolute,
/// and `/run/user/<uid>` otherwise, which is where the PAM module looks as well.
#[must_use]
pub fn socket_path_from_environment() -> PathBuf {
    let runtime_dir = match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
        Some(dir) if dir.is_absolute() => dir,
        _ => PathBuf::from(format!("/run/user/{}", rustix::process::getuid().as_raw())),
    };
    gnoblin_keyring_handoff::socket_path(&runtime_dir)
}

/// The Secret Service agent. It serves `org.freedesktop.secrets` from the store and socket it was
/// given, or logs why it cannot and does not start.
pub struct KeyringAgent {
    store: PathBuf,
    socket: PathBuf,
    /// Held so the bus name stays ours for as long as the role runs.
    connection: Option<zbus::Connection>,
}

impl KeyringAgent {
    /// An agent for the store at `store` and the hand-off socket at `socket`. It has not started.
    #[must_use]
    pub fn new(store: PathBuf, socket: PathBuf) -> KeyringAgent {
        KeyringAgent {
            store,
            socket,
            connection: None,
        }
    }
}

impl LinkedAgent for KeyringAgent {
    fn start(&mut self, _host: &Host, _deadline: Tick) -> bool {
        match futures_lite::future::block_on(gnoblin_service_keyring::serve(&self.store, &self.socket)) {
            Ok(connection) => {
                self.connection = Some(connection);
                true
            }
            Err(error) => {
                // Another program owning the name lands here too: that is the coexistence case,
                // and the role serves what is left.
                log::info!(area = "services", %error, "the Secret Service is not served here");
                false
            }
        }
    }

    fn answer(&self, _request: RequestId, _outcome: &RequestOutcome) {}
}

#[cfg(test)]
mod tests {
    use super::provider_is_gnoblin;
    use gnoblin_config::{Config, Value};

    fn config_with(document: Value) -> Config {
        Config::from_value(document).expect("the document is accepted")
    }

    #[test]
    fn gnoblin_is_the_provider_when_the_section_is_absent() {
        assert!(provider_is_gnoblin(&config_with(Value::table([]))));
    }

    #[test]
    fn gnoblin_is_the_provider_only_when_it_is_named() {
        let named = Value::table([("keyring", Value::table([("provider", Value::string("gnoblin"))]))]);
        let other = Value::table([("keyring", Value::table([("provider", Value::string("gnome-keyring"))]))]);
        assert!(provider_is_gnoblin(&config_with(named)));
        assert!(!provider_is_gnoblin(&config_with(other)));
    }
}
