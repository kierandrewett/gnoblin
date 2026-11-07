//! The visual bell, from the `visual-bell` and `visual-bell-type` keys of the `compositor`
//! configuration section.
//!
//! A document with no `compositor` section, or one without these keys, keeps the built-in
//! default: no flash at all. The schema checks the values before they reach this module.

use std::path::Path;

use gnoblin_config::{SearchPaths, Store, VisualBellType};
use gnoblin_wayland::{VisualBellScope, VisualBellSettings};

use crate::SessionConfig;

use crate::log;

/// Sets `config.wayland.visual_bell` from the last accepted configuration in `store`, read
/// from `source`.
pub fn configure_from_store(config: &mut SessionConfig, store: &Store, source: &Path) {
    match store.load(source) {
        Ok(stored) => {
            let section = stored.config.compositor.clone().unwrap_or_default();
            let settings = visual_bell_settings(&section);
            log::debug!(area = "bell", enabled = settings.enabled, scope = ?settings.scope, "loaded from the last accepted configuration");
            config.wayland.visual_bell = settings;
        }
        Err(error) => log::warn!(area = "bell", %error, "configuration not readable; the visual bell stays off"),
    }
}

/// [`configure_from_store`] for the process's own state directory and configuration path.
///
/// With no configuration file there is no accepted document, so the visual bell stays off.
pub fn configure_from_environment(config: &mut SessionConfig) {
    let Some(store) = Store::from_env() else {
        log::debug!(area = "bell", "no state directory is set; the visual bell stays off");
        return;
    };
    match gnoblin_config::locate(&SearchPaths::from_env()) {
        Ok(path) => configure_from_store(config, &store, path.path()),
        Err(error) => log::debug!(area = "bell", %error, "no configuration file; the visual bell stays off"),
    }
}

/// The visual bell settings of an accepted `compositor` section.
fn visual_bell_settings(section: &gnoblin_config::Compositor) -> VisualBellSettings {
    let defaults = VisualBellSettings::default();
    VisualBellSettings {
        enabled: section.visual_bell.unwrap_or(defaults.enabled),
        scope: section
            .visual_bell_type
            .map(visual_bell_scope)
            .unwrap_or(defaults.scope),
    }
}

/// The session scope for a configured flash type.
fn visual_bell_scope(kind: VisualBellType) -> VisualBellScope {
    match kind {
        VisualBellType::FullscreenFlash => VisualBellScope::Fullscreen,
        VisualBellType::FrameFlash => VisualBellScope::Frame,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_section_keeps_the_bell_off() {
        let settings = visual_bell_settings(&gnoblin_config::Compositor::default());
        assert_eq!(settings, VisualBellSettings::default());
        assert!(!settings.enabled);
    }

    #[test]
    fn a_configured_flash_is_enabled_with_its_scope() {
        let section = gnoblin_config::Compositor {
            visual_bell: Some(true),
            visual_bell_type: Some(VisualBellType::FrameFlash),
            ..gnoblin_config::Compositor::default()
        };
        let settings = visual_bell_settings(&section);
        assert!(settings.enabled);
        assert_eq!(settings.scope, VisualBellScope::Frame);
    }

    #[test]
    fn a_scope_without_the_switch_stays_off() {
        let section = gnoblin_config::Compositor {
            visual_bell_type: Some(VisualBellType::FullscreenFlash),
            ..gnoblin_config::Compositor::default()
        };
        let settings = visual_bell_settings(&section);
        assert!(!settings.enabled);
        assert_eq!(settings.scope, VisualBellScope::Fullscreen);
    }
}
