//! The X11 session, from the `xwayland` configuration section.
//!
//! X11 starts when an `Xwayland` binary is on `PATH`. A document with no `xwayland` table
//! starts it with the documented defaults. A host with no binary runs a working session
//! without X11: that is logged once and no server is started. A value the section states
//! that cannot be run is refused by naming its leaf, and the session starts without X11
//! rather than with a different policy from the one written.

use std::ffi::OsStr;
use std::path::Path;

use gnoblin_config::{SearchPaths, Store, Xwayland};
use gnoblin_xwayland::{Settings, probe};
use tracing::{debug, info, warn};

use crate::SessionConfig;

/// Sets `config.xwayland` from the last accepted configuration in `store`, read from `source`.
pub fn configure_from_store(config: &mut SessionConfig, store: &Store, source: &Path) {
    let path = std::env::var_os("PATH");
    match store.load(source) {
        Ok(stored) => apply(config, stored.config.xwayland.as_ref(), path.as_deref()),
        Err(error) => {
            warn!(%error, "[xwayland] configuration not readable; the defaults are used");
            apply(config, None, path.as_deref());
        }
    }
}

/// [`configure_from_store`] for the process's own state directory and configuration path.
///
/// With no configuration file there is no accepted document, so the defaults are used.
pub fn configure_from_environment(config: &mut SessionConfig) {
    let Some(store) = Store::from_env() else {
        debug!("[xwayland] no state directory is set; the defaults are used");
        apply(config, None, std::env::var_os("PATH").as_deref());
        return;
    };
    match gnoblin_config::locate(&SearchPaths::from_env()) {
        Ok(path) => configure_from_store(config, &store, path.path()),
        Err(error) => {
            debug!(%error, "[xwayland] no configuration file; the defaults are used");
            apply(config, None, std::env::var_os("PATH").as_deref());
        }
    }
}

/// Sets `config.xwayland` to what `section` states, looking for the binary on `path`.
fn apply(config: &mut SessionConfig, section: Option<&Xwayland>, path: Option<&OsStr>) {
    match settings_for(section, path) {
        Ok(Some(settings)) => config.xwayland = Some(settings),
        Ok(None) => {
            info!("[xwayland] no Xwayland binary on PATH; the session runs without X11");
            config.xwayland = None;
        }
        Err(reason) => {
            warn!("[xwayland] {reason}; the session runs without X11");
            config.xwayland = None;
        }
    }
}

/// The settings `section` states, or [`None`] when there is no binary on `path`.
///
/// # Errors
///
/// The leaf and the reason, when a value cannot be run. The schema already checked the
/// type and the list members; the scale is re-checked here because a value between zero
/// and `0.5` passes the schema and would round to a scale of zero.
fn settings_for(section: Option<&Xwayland>, path: Option<&OsStr>) -> Result<Option<Settings>, String> {
    if let Some(factor) = section.and_then(|section| section.scaling_factor) {
        Settings::scale_from_factor(factor).map_err(|error| format!("xwayland.scaling-factor: {error}"))?;
    }
    if probe(path).is_err() {
        return Ok(None);
    }
    Ok(Some(section.map_or_else(Settings::default, Settings::from_config)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_config::{Config, Value, XwaylandExtension};

    /// A throwaway `PATH` directory, with an executable `Xwayland` in it when `present`.
    fn search_path(present: bool) -> tempfile::TempDir {
        let directory = tempfile::tempdir().expect("a temporary directory");
        if present {
            let binary = directory.path().join("Xwayland");
            std::fs::write(&binary, "").expect("the binary");
            std::fs::set_permissions(&binary, std::os::unix::fs::PermissionsExt::from_mode(0o755)).expect("executable");
        }
        directory
    }

    /// The `xwayland` section of a document holding `entries`, as the store would hand it over.
    fn section<const N: usize>(entries: [(&str, Value); N]) -> Xwayland {
        Config::from_value(Value::table([("xwayland", Value::table(entries))]))
            .expect("the document is valid")
            .xwayland
            .expect("the section is present")
    }

    #[test]
    fn an_absent_section_starts_with_the_defaults_when_the_binary_is_present() {
        let directory = search_path(true);
        let mut config = SessionConfig::default();
        apply(&mut config, None, Some(directory.path().as_os_str()));
        assert_eq!(config.xwayland, Some(Settings::default()));
    }

    #[test]
    fn every_leaf_maps_into_the_settings() {
        let directory = search_path(true);
        let section = section([
            ("allow-grabs", Value::Bool(true)),
            ("allow-byte-swapped-clients", Value::Bool(true)),
            (
                "grab-access-rules",
                Value::List(vec![Value::string("legacy-app"), Value::string("!blocked-app")]),
            ),
            (
                "disable-extensions",
                Value::List(vec![Value::string("security"), Value::string("xtest")]),
            ),
            ("scaling-factor", Value::Float(1.5)),
        ]);
        let mut config = SessionConfig::default();
        apply(&mut config, Some(&section), Some(directory.path().as_os_str()));

        let settings = config.xwayland.expect("X11 starts");
        assert!(settings.allow_grabs);
        assert!(settings.allow_byte_swapped_clients);
        assert_eq!(settings.grab_access_rules, ["legacy-app", "!blocked-app"]);
        assert_eq!(
            settings.disable_extensions,
            [XwaylandExtension::Security, XwaylandExtension::Xtest]
        );
        assert_eq!(
            settings.scale,
            Settings::scale_from_factor(1.5).expect("1.5 is a scale")
        );
        assert_eq!(settings.runtime_dir, None);
    }

    #[test]
    fn an_empty_table_keeps_the_documented_defaults() {
        let directory = search_path(true);
        let section = section([]);
        assert_eq!(
            settings_for(Some(&section), Some(directory.path().as_os_str())),
            Ok(Some(Settings::default()))
        );
    }

    #[test]
    fn automatic_scaling_maps_to_no_fixed_scale() {
        let directory = search_path(true);
        let section = section([("scaling-factor", Value::Float(0.0))]);
        let settings = settings_for(Some(&section), Some(directory.path().as_os_str()))
            .expect("zero is automatic")
            .expect("X11 starts");
        assert_eq!(settings.scale, None);
    }

    #[test]
    fn a_scale_that_rounds_to_zero_is_refused_naming_the_leaf() {
        // The store refuses the value before the mapper sees it, so the refusal is the
        // store's and must name the leaf.
        let document = Value::table([("xwayland", Value::table([("scaling-factor", Value::Float(0.3))]))]);
        let report = match Config::from_value(document) {
            Ok(_) => panic!("0.3 is refused"),
            Err(report) => format!("{report:?}"),
        };
        assert!(report.contains("scaling-factor"), "{report}");
    }

    #[test]
    fn a_missing_binary_leaves_x11_off() {
        let directory = search_path(false);
        let section = section([("allow-grabs", Value::Bool(true))]);
        let mut config = SessionConfig::default();
        apply(&mut config, Some(&section), Some(directory.path().as_os_str()));
        assert_eq!(config.xwayland, None);

        let mut config = SessionConfig::default();
        apply(&mut config, None, None);
        assert_eq!(config.xwayland, None);
    }
}
