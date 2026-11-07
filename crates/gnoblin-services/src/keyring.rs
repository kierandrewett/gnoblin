//! The Secret Service the services role serves, when `keyring.enabled` is true.
//!
//! `design/keyring.md` ("Coexistence"): with `keyring.enabled` on, which is the default, Gnoblin
//! takes `org.freedesktop.secrets` and serves its own store. With it off, the role does not request
//! the name, creates no store file or directory, and runs no keyring code; another program keeps the
//! name.

use std::path::PathBuf;

use gnoblin_model::CapabilitySlot;
use gnoblin_service_keyring::Secrets;

use crate::link::{Host, LinkedAgent, RequestId, RequestOutcome, Tick};
use crate::log;

/// Whether the environment's configuration enables Gnoblin's built-in keyring. The rule lives in
/// `gnoblin-config`, shared with the portal relay. An unreadable configuration keeps the default.
pub fn enabled_from_environment() -> bool {
    gnoblin_config::keyring_enabled_from_environment().unwrap_or_else(|problem| {
        log::warn!(%problem, "keyring.enabled is the default");
        true
    })
}

/// The Secret Service agent for the store and socket, when the built-in keyring is enabled. When it
/// is off, this is `None` and the store is not touched.
#[must_use]
pub fn agent_for(served: bool, store: Option<PathBuf>, socket: PathBuf) -> Option<KeyringAgent> {
    if !served {
        return None;
    }
    let Some(store) = store else {
        log::warn!("no keyring store path: neither XDG_DATA_HOME nor HOME is absolute");
        return None;
    };
    Some(KeyringAgent::new(store, socket))
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
    /// Reads the app secrets behind `secret.derive`. Absent until the store serves.
    secrets: Option<Secrets>,
}

impl KeyringAgent {
    /// An agent for the store at `store` and the hand-off socket at `socket`. It has not started.
    #[must_use]
    pub fn new(store: PathBuf, socket: PathBuf) -> KeyringAgent {
        KeyringAgent {
            store,
            socket,
            connection: None,
            secrets: None,
        }
    }
}

impl LinkedAgent for KeyringAgent {
    fn start(&mut self, _host: &Host, _deadline: Tick) -> bool {
        match futures_lite::future::block_on(gnoblin_service_keyring::serve(&self.store, &self.socket)) {
            Ok((connection, secrets)) => {
                self.connection = Some(connection);
                self.secrets = Some(secrets);
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

    /// Derive the app secret for `secret.derive`. The subject is the app id, which the portal
    /// sends as the offer's subject. A locked store or a missing app id declines, because the
    /// caller is left with nothing it can use and must not wait for the deadline.
    fn offer(&self, host: &Host, request: RequestId, slot: &str, subject: &[u8]) -> bool {
        if slot != CapabilitySlot::SecretDerive.name() {
            return false;
        }
        let secret = self
            .secrets
            .as_ref()
            .ok_or_else(|| "the store is not serving".to_owned())
            .and_then(|secrets| {
                let app_id = std::str::from_utf8(subject).map_err(|error| error.to_string())?;
                secrets.app_secret(app_id).map_err(|error| format!("{error:?}"))
            });
        match secret {
            Ok(secret) => host.answer(request, &secret[..]),
            Err(reason) => {
                log::info!(area = "services", request = request.get(), %reason, "declining a secret offer");
                host.decline(request);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::agent_for;

    fn store() -> PathBuf {
        PathBuf::from("/nonexistent-gnoblin-keyring-test/keyring.db")
    }

    fn socket() -> PathBuf {
        PathBuf::from("/nonexistent-gnoblin-keyring-test/keyring.sock")
    }

    #[test]
    fn no_agent_when_the_built_in_keyring_is_disabled() {
        assert!(agent_for(false, Some(store()), socket()).is_none());
    }

    #[test]
    fn no_agent_and_no_store_when_the_built_in_keyring_is_disabled() {
        assert!(agent_for(false, None, socket()).is_none());
        assert!(!store().exists());
    }

    #[test]
    fn agent_when_gnoblin_serves_the_name() {
        assert!(agent_for(true, Some(store()), socket()).is_some());
    }

    #[test]
    fn no_agent_when_gnoblin_serves_the_name_but_has_no_store_path() {
        assert!(agent_for(true, None, socket()).is_none());
    }
}
