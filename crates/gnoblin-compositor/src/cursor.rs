//! The pointer cursor theme, size and providers, from the `cursor` configuration section.
//!
//! A document with no `cursor` section keeps the built-in defaults. A section with a
//! field missing keeps that field's default. The values are checked by the schema before
//! they reach this module, so a size outside the accepted range is ignored rather than
//! clamped.

use std::path::Path;

use gnoblin_backend::render::cursor::{DEFAULT_CURSOR_SIZE, ProviderKind};
use gnoblin_config::{CursorProvider, SearchPaths, Store};
use gnoblin_log::{debug_in, warn_in};
use gnoblin_wayland::CursorSettings;

use crate::SessionConfig;

/// Sets `config.wayland.cursor` from the last accepted configuration in `store`, read from
/// `source`.
pub fn configure_from_store(config: &mut SessionConfig, store: &Store, source: &Path) {
    match store.load(source) {
        Ok(stored) => {
            let section = stored.config.cursor.clone().unwrap_or_default();
            let settings = cursor_settings(section);
            debug_in!("compositor", theme = %settings.theme, size = settings.size, "[cursor] loaded from the last accepted configuration");
            config.wayland.cursor = settings;
        }
        Err(error) => warn_in!("compositor", %error, "[cursor] configuration not readable; the default cursor is used"),
    }
}

/// [`configure_from_store`] for the process's own state directory and configuration path.
///
/// With no configuration file there is no accepted document, so the default cursor is used.
pub fn configure_from_environment(config: &mut SessionConfig) {
    let Some(store) = Store::from_env() else {
        debug_in!(
            "compositor",
            "[cursor] no state directory is set; the default cursor is used"
        );
        return;
    };
    match gnoblin_config::locate(&SearchPaths::from_env()) {
        Ok(path) => configure_from_store(config, &store, path.path()),
        Err(error) => debug_in!("compositor", %error, "[cursor] no configuration file; the default cursor is used"),
    }
}

/// The cursor settings of an accepted `cursor` section.
fn cursor_settings(section: gnoblin_config::Cursor) -> CursorSettings {
    let defaults = CursorSettings::default();
    CursorSettings {
        theme: section
            .theme
            .filter(|theme| !theme.trim().is_empty())
            .unwrap_or(defaults.theme),
        size: section
            .size
            .and_then(|size| u16::try_from(size).ok())
            .unwrap_or(DEFAULT_CURSOR_SIZE),
        providers: section
            .providers
            .map(|providers| providers.into_iter().map(provider_kind).collect())
            .unwrap_or(defaults.providers),
    }
}

/// The render-side provider for a configured one.
fn provider_kind(provider: CursorProvider) -> ProviderKind {
    match provider {
        CursorProvider::Hyprcursor => ProviderKind::Hyprcursor,
        CursorProvider::KdeGnomeSvg => ProviderKind::KdeGnomeSvg,
        CursorProvider::Xcursor => ProviderKind::Xcursor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_backend::render::cursor::DEFAULT_CURSOR_THEME;

    #[test]
    fn an_absent_section_keeps_the_defaults() {
        let settings = cursor_settings(gnoblin_config::Cursor::default());
        assert_eq!(settings, CursorSettings::default());
        assert_eq!(settings.theme, DEFAULT_CURSOR_THEME);
    }

    #[test]
    fn configured_values_replace_the_defaults() {
        let section = gnoblin_config::Cursor {
            theme: Some("Adwaita".to_owned()),
            size: Some(32),
            providers: Some(vec![CursorProvider::Xcursor]),
        };
        let settings = cursor_settings(section);
        assert_eq!(settings.theme, "Adwaita");
        assert_eq!(settings.size, 32);
        assert_eq!(settings.providers, vec![ProviderKind::Xcursor]);
    }

    #[test]
    fn an_out_of_range_size_keeps_the_default() {
        let section = gnoblin_config::Cursor {
            size: Some(-4),
            ..gnoblin_config::Cursor::default()
        };
        assert_eq!(cursor_settings(section).size, DEFAULT_CURSOR_SIZE);
    }
}
