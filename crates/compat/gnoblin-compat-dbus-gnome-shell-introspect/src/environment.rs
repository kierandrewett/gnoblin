//! What the adapter needs from its process: the compositor socket and the `compat` gate.
//!
//! The adapter owns these reads so the process that links it names no client, protocol or
//! configuration crate of its own. The socket follows the session's environment; the gate is
//! the `[compat] capture` key, where a missing key or an unreadable file is the default.

use gnoblin_client::{Client, UnixTransport, socket_path};
use gnoblin_compat_core::Mode;
use gnoblin_config::{SearchPaths, Store};

use gnoblin_proto::Version;

use crate::{IntrospectError, IntrospectLink};

use crate::log;

/// Connects to the compositor socket named by the environment and subscribes to its graph.
pub fn link_from_environment() -> Result<IntrospectLink<UnixTransport>, IntrospectError> {
    let link = |reason: String| IntrospectError::Link { reason };
    let path = socket_path(|name| std::env::var(name).ok()).map_err(|error| link(error.to_string()))?;
    let transport = UnixTransport::connect(&path).map_err(|error| link(error.to_string()))?;
    let client: Client<UnixTransport> =
        Client::connect(transport, Version::new(0, 2, 0)).map_err(|error| link(error.to_string()))?;
    IntrospectLink::connect(client)
}

/// The compat mode the configuration names, or the default `portal` when none is readable.
pub fn mode_from_environment() -> Mode {
    let Some(store) = Store::from_env() else {
        return Mode::default();
    };
    match gnoblin_config::locate(&SearchPaths::from_env()) {
        Ok(path) => match store.load(path.path()) {
            Ok(stored) => mode(stored.config.compat.as_ref()),
            Err(error) => {
                log::warn!(area = "services", %error, "configuration not readable; compat stays at its default");
                Mode::default()
            }
        },
        Err(error) => {
            log::debug!(area = "services", %error, "no configuration file; compat stays at its default");
            Mode::default()
        }
    }
}

/// The mode a `compat` section names. A missing section or key is the default.
pub fn mode(compat: Option<&gnoblin_config::Compat>) -> Mode {
    match compat.and_then(|compat| compat.capture) {
        Some(gnoblin_config::CompatCapture::Off) => Mode::Off,
        Some(gnoblin_config::CompatCapture::Unmediated) => Mode::Unmediated,
        Some(gnoblin_config::CompatCapture::Portal) | None => Mode::Portal,
    }
}

#[cfg(test)]
mod tests {
    use super::mode;
    use gnoblin_compat_core::Mode;
    use gnoblin_config::{Compat, CompatCapture};

    #[test]
    fn a_missing_compat_section_is_the_portal_default() {
        assert_eq!(mode(None), Mode::Portal);
    }

    #[test]
    fn the_configured_capture_names_the_mode() {
        let off = Compat {
            capture: Some(CompatCapture::Off),
        };
        let unmediated = Compat {
            capture: Some(CompatCapture::Unmediated),
        };
        assert_eq!(mode(Some(&off)), Mode::Off);
        assert_eq!(mode(Some(&unmediated)), Mode::Unmediated);
    }
}
