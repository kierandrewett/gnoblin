//! The visual accessibility provider: the `magnifier`, `magnifier-zoom` and `colour-filter` keys of the `accessibility`
//! configuration section, turned into the composer's `a11y.zoom` and `a11y.filter` settings.
//!
//! The mechanism is core's (the frame's zoom and colour passes, drawn from the settings the composer holds); what
//! a document means by "invert" or "2x" is this provider's. A document with no `accessibility` section, or one without
//! these keys, keeps the built-in default: no magnifier and no colour filter. The schema checks the values before they
//! reach this module.

use std::path::Path;

use gnoblin_a11y::ColourFilter;
use gnoblin_config::{ColourFilter as ConfigColourFilter, Environment, Store, VisualAccessibility};
use gnoblin_render_frame::frame::{MAX_ZOOM, MIN_ZOOM};
use gnoblin_wayland::Setting;

use crate::SessionConfig;

use super::set_settings;
use crate::log;

/// The zoom used when a document turns the magnifier on without naming a factor. It is the
/// schema's own default, so the two never disagree.
const DEFAULT_ZOOM: f64 = 2.0;

/// Sets the magnifier and colour filter in `config.wayland.settings` from the last accepted configuration in `store`,
/// read from `source`.
pub fn configure_from_store(config: &mut SessionConfig, store: &Store, source: &Path) {
    match store.load(source) {
        Ok(stored) => {
            let section = stored.config.accessibility.clone().unwrap_or_default();
            let settings = visual_accessibility_settings(&section);
            log::debug!(
                area = "accessibility",
                magnifier = matches!(settings[0], Setting::Zoom(Some(_))),
                colour = matches!(settings[1], Setting::ColourFilter(Some(_))),
                "loaded from the last accepted configuration"
            );
            set_settings(&mut config.wayland.settings, settings);
        }
        Err(error) if error.is_absent() => {
            log::debug!(area = "accessibility", %error, "no configuration file; the visual accessibility stays off")
        }
        Err(error) => {
            log::warn!(area = "accessibility", %error, "configuration not readable; the visual accessibility stays off")
        }
    }
}

/// [`configure_from_store`] for the process's own state directory and configuration path.
///
/// With no configuration file there is no accepted document, so the visual accessibility stays
/// off.
pub fn configure_from_environment(config: &mut SessionConfig) {
    let Some(store) = Store::from_env() else {
        log::debug!(
            area = "accessibility",
            "no state directory is set; the visual accessibility stays off"
        );
        return;
    };
    match gnoblin_config::locate(&Environment::from_env()) {
        Ok(path) => configure_from_store(config, &store, path.path()),
        Err(error) => {
            log::debug!(area = "accessibility", %error, "no configuration file; the visual accessibility stays off")
        }
    }
}

/// The `a11y.zoom` and `a11y.filter` settings of an accepted `accessibility` section, in that order.
///
/// The magnifier is on only when `magnifier` is set and the zoom is above the identity; a zoom
/// above the frame's ceiling is clamped to it, so a value the schema accepts is never dropped.
/// The colour filter is applied only when it names a transform; `none` leaves colours alone.
pub(crate) fn visual_accessibility_settings(section: &VisualAccessibility) -> [Setting; 2] {
    let magnifier_zoom = if section.magnifier.unwrap_or(false) {
        let zoom = section
            .magnifier_zoom
            .unwrap_or(DEFAULT_ZOOM)
            .clamp(f64::from(MIN_ZOOM), f64::from(MAX_ZOOM)) as f32;
        (zoom > MIN_ZOOM).then_some(zoom)
    } else {
        None
    };
    let colour_matrix = section.colour_filter.and_then(colour_matrix);
    [Setting::Zoom(magnifier_zoom), Setting::ColourFilter(colour_matrix)]
}

/// The colour transform rows of a configured filter, or [`None`] when it leaves colours alone.
fn colour_matrix(filter: ConfigColourFilter) -> Option<[[f32; 5]; 4]> {
    let filter = match filter {
        ConfigColourFilter::None => return None,
        ConfigColourFilter::Invert => ColourFilter::Invert,
        ConfigColourFilter::Greyscale => ColourFilter::Greyscale,
        ConfigColourFilter::Protanopia => ColourFilter::Protanopia,
        ConfigColourFilter::Deuteranopia => ColourFilter::Deuteranopia,
        ConfigColourFilter::Tritanopia => ColourFilter::Tritanopia,
    };
    Some(filter.matrix())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_section_keeps_both_off() {
        let settings = visual_accessibility_settings(&VisualAccessibility::default());
        assert_eq!(settings, [Setting::Zoom(None), Setting::ColourFilter(None)]);
    }

    #[test]
    fn a_magnifier_without_a_zoom_uses_the_schema_default() {
        let section = VisualAccessibility {
            magnifier: Some(true),
            ..VisualAccessibility::default()
        };
        assert_eq!(visual_accessibility_settings(&section)[0], Setting::Zoom(Some(2.0)));
    }

    #[test]
    fn a_zoom_above_the_frame_ceiling_is_clamped_not_dropped() {
        let section = VisualAccessibility {
            magnifier: Some(true),
            magnifier_zoom: Some(32.0),
            ..VisualAccessibility::default()
        };
        assert_eq!(
            visual_accessibility_settings(&section)[0],
            Setting::Zoom(Some(MAX_ZOOM))
        );
    }

    #[test]
    fn a_zoom_of_one_turns_the_magnifier_off() {
        let section = VisualAccessibility {
            magnifier: Some(true),
            magnifier_zoom: Some(1.0),
            ..VisualAccessibility::default()
        };
        assert_eq!(visual_accessibility_settings(&section)[0], Setting::Zoom(None));
    }

    #[test]
    fn a_zoom_without_the_magnifier_switched_on_does_nothing() {
        let section = VisualAccessibility {
            magnifier: Some(false),
            magnifier_zoom: Some(4.0),
            ..VisualAccessibility::default()
        };
        assert_eq!(visual_accessibility_settings(&section)[0], Setting::Zoom(None));
    }

    #[test]
    fn no_filter_is_no_matrix() {
        assert!(colour_matrix(ConfigColourFilter::None).is_none());
    }

    #[test]
    fn each_named_filter_maps_to_its_own_matrix() {
        assert_eq!(
            colour_matrix(ConfigColourFilter::Invert),
            Some(ColourFilter::Invert.matrix())
        );
        assert_eq!(
            colour_matrix(ConfigColourFilter::Greyscale),
            Some(ColourFilter::Greyscale.matrix())
        );
        assert_eq!(
            colour_matrix(ConfigColourFilter::Protanopia),
            Some(ColourFilter::Protanopia.matrix())
        );
        assert_eq!(
            colour_matrix(ConfigColourFilter::Deuteranopia),
            Some(ColourFilter::Deuteranopia.matrix())
        );
        assert_eq!(
            colour_matrix(ConfigColourFilter::Tritanopia),
            Some(ColourFilter::Tritanopia.matrix())
        );
    }
}
