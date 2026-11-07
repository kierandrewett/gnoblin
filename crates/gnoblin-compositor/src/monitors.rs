//! The `monitors` configuration section: the privacy-screen request and the global integer
//! scale, read into the Wayland host's session configuration.
//!
//! A document with no `monitors` section keeps the defaults. A leaf set to `"inherit"` or
//! left out is `None`, so the backend keeps its own state. A global scale of 0 follows the
//! system setting, which is also `None`. The schema checks the values before they reach
//! this module.
//!
//! The privacy-screen request is stored on the host only. The output schema has no
//! privacy member, so nothing applies it to hardware yet.

use std::path::Path;

use gnoblin_config::{Inherit, Monitors, SearchPaths, Store};

use gnoblin_wayland::MonitorSettings;

use crate::SessionConfig;

use crate::log;

/// Sets the monitor settings of `config.wayland` from the last accepted configuration in
/// `store`, read from `source`.
pub fn configure_from_store(config: &mut SessionConfig, store: &Store, source: &Path) {
    match store.load(source) {
        Ok(stored) => {
            let section = stored.config.monitors.clone().unwrap_or_default();
            config.wayland.monitors = settings(&section);
            log::debug!(area = "monitors", "loaded from the last accepted configuration");
        }
        Err(error) if error.is_absent() => {
            log::debug!(area = "monitors", %error, "no configuration file; the defaults are used")
        }
        Err(error) => log::warn!(area = "monitors", %error, "configuration not readable; the defaults are used"),
    }
}

/// [`configure_from_store`] for the process's own state directory and configuration path.
///
/// With no configuration file there is no accepted document, so the defaults are used.
pub fn configure_from_environment(config: &mut SessionConfig) {
    let Some(store) = Store::from_env() else {
        log::debug!(area = "monitors", "no state directory is set; the defaults are used");
        return;
    };
    match gnoblin_config::locate(&SearchPaths::from_env()) {
        Ok(path) => configure_from_store(config, &store, path.path()),
        Err(error) => log::debug!(area = "monitors", %error, "no configuration file; the defaults are used"),
    }
}

/// The host settings a `monitors` section asks for. An inherited or absent leaf is `None`,
/// and a global scale of 0 is `None` because 0 follows the system setting.
fn settings(section: &Monitors) -> MonitorSettings {
    MonitorSettings {
        privacy_screen: section.privacy_screen.as_ref().and_then(Inherit::value).copied(),
        global_scale: section
            .global_scale
            .as_ref()
            .and_then(Inherit::value)
            .and_then(|scale| u32::try_from(*scale).ok())
            .filter(|scale| *scale > 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_section_keeps_the_defaults() {
        assert_eq!(settings(&Monitors::default()), MonitorSettings::default());
    }

    #[test]
    fn explicit_leaves_map_to_their_values() {
        let section = Monitors {
            privacy_screen: Some(Inherit::Value(true)),
            global_scale: Some(Inherit::Value(2)),
        };
        assert_eq!(
            settings(&section),
            MonitorSettings {
                privacy_screen: Some(true),
                global_scale: Some(2),
            }
        );
    }

    #[test]
    fn inherit_clears_each_leaf() {
        let section = Monitors {
            privacy_screen: Some(Inherit::Inherit),
            global_scale: Some(Inherit::Inherit),
        };
        assert_eq!(settings(&section), MonitorSettings::default());
    }

    #[test]
    fn a_global_scale_of_zero_follows_the_system() {
        let section = Monitors {
            global_scale: Some(Inherit::Value(0)),
            ..Monitors::default()
        };
        assert_eq!(settings(&section).global_scale, None);
    }
}
