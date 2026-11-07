//! The session-wide tables the Wayland host reads at startup: the advertised protocols,
//! from the `protocols` configuration section, the keyboard repeat and XKB options, from
//! `input.keyboard`, and the switchable layouts, from `input-sources`.
//!
//! A document with no such section keeps the built-in defaults. The values are checked by
//! the schema before they reach this module, so an unknown protocol name or a non-boolean
//! toggle is refused there and never arrives here.

use std::path::Path;

use gnoblin_config::{SearchPaths, Store};

use crate::SessionConfig;

use crate::log;

/// Sets the session tables of `config.wayland` from the last accepted configuration in
/// `store`, read from `source`.
pub fn configure_from_store(config: &mut SessionConfig, store: &Store, source: &Path) {
    match store.load(source) {
        Ok(stored) => {
            config.wayland.protocols = stored.config.protocols.unwrap_or_default();
            config.wayland.keyboard = stored.config.input.and_then(|input| input.keyboard);
            config.wayland.input_sources = stored.config.input_sources;
            log::debug!("[session tables] loaded from the last accepted configuration");
        }
        Err(error) => {
            log::warn!(%error, "[session tables] configuration not readable; the defaults are used");
        }
    }
}

/// [`configure_from_store`] for the process's own state directory and configuration path.
///
/// With no configuration file there is no accepted document, so the defaults are used.
pub fn configure_from_environment(config: &mut SessionConfig) {
    let Some(store) = Store::from_env() else {
        log::debug!("[session tables] no state directory is set; the defaults are used");
        return;
    };
    match gnoblin_config::locate(&SearchPaths::from_env()) {
        Ok(path) => configure_from_store(config, &store, path.path()),
        Err(error) => {
            log::debug!(%error, "[session tables] no configuration file; the defaults are used");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_config::{Config, Inherit, InputSourceType, Protocol, Value};

    /// A document with one `protocols` table holding `entries`.
    fn document(entries: Vec<(&str, Value)>) -> Value {
        let protocols = entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect();
        Value::table([("protocols", Value::Map(protocols))])
    }

    /// A document with one `input.keyboard` table holding `entries`.
    fn keyboard_document(entries: Vec<(&str, Value)>) -> Value {
        let keyboard = entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect();
        Value::table([("input", Value::table([("keyboard", Value::Map(keyboard))]))])
    }

    /// A document with one `input-sources` table holding `entries`.
    fn sources_document(entries: Vec<(&str, Value)>) -> Value {
        let sources = entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect();
        Value::table([("input-sources", Value::Map(sources))])
    }

    /// One `input-sources` entry with a type and an id.
    fn source(kind: &str, id: &str) -> Value {
        Value::table([("type", Value::string(kind)), ("id", Value::string(id))])
    }

    /// Maps an accepted document into a session configuration, as the store path does.
    fn session_from(document: Value) -> SessionConfig {
        let config = Config::from_value(document).expect("the document is valid");
        let mut session = SessionConfig::default();
        session.wayland.protocols = config.protocols.unwrap_or_default();
        session.wayland.keyboard = config.input.and_then(|input| input.keyboard);
        session.wayland.input_sources = config.input_sources;
        session
    }

    /// The paths of every problem in a refused document.
    fn refused_paths(document: Value) -> Vec<String> {
        Config::from_value(document)
            .expect_err("the document is refused")
            .problems()
            .iter()
            .map(|problem| problem.path.to_string())
            .collect()
    }

    #[test]
    fn an_absent_section_keeps_the_defaults() {
        let session = session_from(Value::table([]));
        assert_eq!(session.wayland.protocols, gnoblin_wayland::Config::default().protocols);
        assert!(session.wayland.protocols.toggles.is_empty());
        assert_eq!(session.wayland.keyboard, None);
        assert_eq!(session.wayland.input_sources, None);
    }

    #[test]
    fn every_protocol_leaf_maps_into_the_compositor_configuration() {
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
        let paths = refused_paths(document(vec![("wlr-screencopy", Value::string("no"))]));
        assert!(paths.iter().any(|path| path.contains("wlr-screencopy")), "{paths:?}");
    }

    #[test]
    fn an_unknown_protocol_is_refused_naming_the_leaf() {
        let paths = refused_paths(document(vec![("not-a-protocol", Value::Bool(false))]));
        assert!(paths.iter().any(|path| path.contains("not-a-protocol")), "{paths:?}");
    }

    #[test]
    fn an_absent_keyboard_table_keeps_the_host_defaults() {
        let session = session_from(Value::table([("input", Value::table([]))]));
        assert_eq!(session.wayland.keyboard, None);
    }

    #[test]
    fn every_keyboard_leaf_the_host_reads_maps_into_the_compositor_configuration() {
        let session = session_from(keyboard_document(vec![
            ("repeat", Value::Bool(false)),
            ("delay", Value::Integer(500)),
            ("repeat-interval", Value::Integer(30)),
            (
                "xkb-options",
                Value::List(vec![Value::string("ctrl:nocaps"), Value::string("compose:ralt")]),
            ),
        ]));
        let keyboard = session.wayland.keyboard.expect("the keyboard table is present");

        assert_eq!(keyboard.repeat, Some(Inherit::Value(false)));
        assert_eq!(keyboard.delay, Some(Inherit::Value(500)));
        assert_eq!(keyboard.repeat_interval, Some(Inherit::Value(30)));
        assert_eq!(
            keyboard.xkb_options,
            Some(Inherit::Value(vec![
                "ctrl:nocaps".to_owned(),
                "compose:ralt".to_owned(),
            ]))
        );
    }

    #[test]
    fn a_non_integer_repeat_interval_is_refused_naming_the_leaf() {
        let paths = refused_paths(keyboard_document(vec![("repeat-interval", Value::string("fast"))]));
        assert!(paths.iter().any(|path| path.contains("repeat-interval")), "{paths:?}");
    }

    #[test]
    fn every_input_sources_leaf_maps_into_the_compositor_configuration() {
        let session = session_from(sources_document(vec![
            (
                "sources",
                Value::List(vec![source("xkb", "us"), source("ibus", "anthy")]),
            ),
            ("per-window", Value::Bool(true)),
        ]));
        let sources = session
            .wayland
            .input_sources
            .expect("the input-sources table is present");

        assert_eq!(sources.per_window, Some(true));
        let entries = sources.sources.expect("the sources list is present");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].r#type, Some(InputSourceType::Xkb));
        assert_eq!(entries[0].id.as_deref(), Some("us"));
        assert_eq!(entries[1].r#type, Some(InputSourceType::Ibus));
        assert_eq!(entries[1].id.as_deref(), Some("anthy"));
    }

    #[test]
    fn a_non_boolean_per_window_is_refused_naming_the_leaf() {
        let paths = refused_paths(sources_document(vec![("per-window", Value::string("yes"))]));
        assert!(paths.iter().any(|path| path.contains("per-window")), "{paths:?}");
    }

    #[test]
    fn an_unknown_input_source_type_is_refused_naming_the_leaf() {
        let paths = refused_paths(sources_document(vec![(
            "sources",
            Value::List(vec![source("x11", "us")]),
        )]));
        assert!(paths.iter().any(|path| path.contains("type")), "{paths:?}");
    }
}
