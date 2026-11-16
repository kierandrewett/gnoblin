//! The visual bell provider: the `visual-bell` and `visual-bell-type` keys of the `compositor` configuration section,
//! turned into the composer's `bell.flash` setting.
//!
//! The flash is core's (the node effect that fades out over a window or every output, run when a client rings the
//! bell); whether to flash and where is this provider's. A document with no `compositor` section, or one without these
//! keys, keeps the built-in default: no flash at all. The schema checks the values before they reach this module.

use std::path::Path;

use gnoblin_config::{Environment, Store, VisualBellType};
use gnoblin_wayland::{BellFlash, Setting};

use crate::SessionConfig;

use super::set_settings;
use crate::log;

/// Sets the `bell.flash` setting in `config.wayland.settings` from the last accepted configuration in `store`, read
/// from `source`.
pub fn configure_from_store(config: &mut SessionConfig, store: &Store, source: &Path) {
    match store.load(source) {
        Ok(stored) => {
            let section = stored.config.compositor.clone().unwrap_or_default();
            let flash = visual_bell_setting(&section);
            log::debug!(area = "bell", flash = ?flash, "loaded from the last accepted configuration");
            set_settings(&mut config.wayland.settings, [flash]);
        }
        Err(error) if error.is_absent() => {
            log::debug!(area = "bell", %error, "no configuration file; the visual bell stays off")
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
    match gnoblin_config::locate(&Environment::from_env()) {
        Ok(path) => configure_from_store(config, &store, path.path()),
        Err(error) => log::debug!(area = "bell", %error, "no configuration file; the visual bell stays off"),
    }
}

/// The `bell.flash` setting of an accepted `compositor` section: off unless `visual-bell` is set, and the scope
/// `visual-bell-type` names (the whole screen when it names none).
pub(crate) fn visual_bell_setting(section: &gnoblin_config::Compositor) -> Setting {
    if !section.visual_bell.unwrap_or(false) {
        return Setting::BellFlash(BellFlash::Off);
    }
    Setting::BellFlash(match section.visual_bell_type {
        Some(VisualBellType::FrameFlash) => BellFlash::Frame,
        Some(VisualBellType::FullscreenFlash) | None => BellFlash::Fullscreen,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_section_keeps_the_bell_off() {
        let setting = visual_bell_setting(&gnoblin_config::Compositor::default());
        assert_eq!(setting, Setting::BellFlash(BellFlash::Off));
    }

    #[test]
    fn a_configured_flash_is_enabled_with_its_scope() {
        let section = gnoblin_config::Compositor {
            visual_bell: Some(true),
            visual_bell_type: Some(VisualBellType::FrameFlash),
            ..gnoblin_config::Compositor::default()
        };
        assert_eq!(visual_bell_setting(&section), Setting::BellFlash(BellFlash::Frame));
    }

    #[test]
    fn a_scope_without_the_switch_stays_off() {
        let section = gnoblin_config::Compositor {
            visual_bell_type: Some(VisualBellType::FullscreenFlash),
            ..gnoblin_config::Compositor::default()
        };
        assert_eq!(visual_bell_setting(&section), Setting::BellFlash(BellFlash::Off));
    }
}
