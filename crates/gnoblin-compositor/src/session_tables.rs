//! The session-wide tables the Wayland host reads at startup: the advertised protocols,
//! from the `protocols` configuration section.
//!
//! A document with no such section keeps the built-in defaults. The values are checked by
//! the schema before they reach this module, so an unknown protocol name or a non-boolean
//! toggle is refused there and never arrives here.

use std::path::Path;

use gnoblin_config::{SearchPaths, Store};
use tracing::{debug, warn};

use crate::SessionConfig;

/// Sets the session tables of `config.wayland` from the last accepted configuration in
/// `store`, read from `source`.
pub fn configure_from_store(config: &mut SessionConfig, store: &Store, source: &Path) {
    match store.load(source) {
        Ok(stored) => {
            config.wayland.protocols = stored.config.protocols.unwrap_or_default();
            debug!("[protocols] loaded from the last accepted configuration");
        }
        Err(error) => {
            warn!(%error, "[protocols] configuration not readable; the defaults are used");
        }
    }
}

/// [`configure_from_store`] for the process's own state directory and configuration path.
///
/// With no configuration file there is no accepted document, so the defaults are used.
pub fn configure_from_environment(config: &mut SessionConfig) {
    let Some(store) = Store::from_env() else {
        debug!("[protocols] no state directory is set; the defaults are used");
        return;
    };
    match gnoblin_config::locate(&SearchPaths::from_env()) {
        Ok(path) => configure_from_store(config, &store, path.path()),
        Err(error) => debug!(%error, "[protocols] no configuration file; the defaults are used"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_config::{Config, Protocol, Value};

    /// A document with one `protocols` table holding `entries`.
    fn document(entries: Vec<(&str, Value)>) -> Value {
        let protocols = entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect();
        Value::table([("protocols", Value::Map(protocols))])
    }

    /// Maps an accepted document into a session configuration, as the store path does.
    fn session_from(document: Value) -> SessionConfig {
        let config = Config::from_value(document).expect("the document is valid");
        let mut session = SessionConfig::default();
        session.wayland.protocols = config.protocols.unwrap_or_default();
        session
    }

    #[test]
    fn an_absent_section_keeps_the_defaults() {
        let session = session_from(Value::table([]));
        assert_eq!(session.wayland.protocols, gnoblin_wayland::Config::default().protocols);
        assert!(session.wayland.protocols.toggles.is_empty());
    }

    #[test]
    fn every_leaf_maps_into_the_compositor_configuration() {
        let names = [
            "wlr-layer-shell",
            "ext-foreign-toplevel-list",
            "wlr-foreign-toplevel-management",
            "xdg-decoration",
            "kde-server-decoration",
            "window-frame-renderer",
            "wlr-screencopy",
            "ext-image-copy-capture",
            "ext-background-effect-v1",
            "blur-fade",
            "ext-data-control",
            "ext-idle-notify",
            "wlr-gamma-control",
            "wlr-output-power-management",
            "wlr-output-management",
            "ext-session-lock",
        ];
        let entries = names.iter().map(|name| (*name, Value::Bool(false))).collect();
        let session = session_from(document(entries));

        assert_eq!(session.wayland.protocols.toggles.len(), names.len());
        assert!(session.wayland.protocols.toggles.values().all(|enabled| !enabled));
        assert_eq!(
            session.wayland.protocols.toggles.get(&Protocol::WlrScreencopy),
            Some(&false)
        );
        assert_eq!(
            session.wayland.protocols.toggles.get(&Protocol::ExtSessionLock),
            Some(&false)
        );
    }

    #[test]
    fn a_non_boolean_toggle_is_refused_naming_the_leaf() {
        let report = Config::from_value(document(vec![("wlr-screencopy", Value::string("no"))]))
            .expect_err("a string is not a protocol toggle");
        let paths: Vec<String> = report
            .problems()
            .iter()
            .map(|problem| problem.path.to_string())
            .collect();
        assert!(paths.iter().any(|path| path.contains("wlr-screencopy")), "{paths:?}");
    }

    #[test]
    fn an_unknown_protocol_is_refused_naming_the_leaf() {
        let report = Config::from_value(document(vec![("not-a-protocol", Value::Bool(false))]))
            .expect_err("an unknown protocol is not in the schema");
        let paths: Vec<String> = report
            .problems()
            .iter()
            .map(|problem| problem.path.to_string())
            .collect();
        assert!(paths.iter().any(|path| path.contains("not-a-protocol")), "{paths:?}");
    }
}
