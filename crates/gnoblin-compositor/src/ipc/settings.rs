//! The `settings` object's values: the stock desktop, overridden by the `appearance` and
//! `cursor` tables the configuration declares.
//!
//! The graph creates an object only with every declared property, so the object starts
//! with the stock values from [`defaults`]. The compositor does not depend on the compat
//! crates, so those values are copied, not imported.
//!
//! The accent's unset state is the Settings portal's own sentinel: an `accent-color`
//! component outside [0, 1] means no accent, so the object carries -1.0 until the
//! configuration names one. See
//! <https://flatpak.github.io/xdg-desktop-portal/docs/org.freedesktop.impl.portal.Settings.html>.

use gnoblin_model::{Fields, Value};

/// The schema's property names for the `settings` object.
pub mod member {
    /// The `settings` object's color_scheme property, the light or dark preference.
    pub const COLOR_SCHEME: &str = "color_scheme";
    /// The accent colour's red component, in [0, 1] or unset.
    pub const ACCENT_RED: &str = "accent_red";
    /// The accent colour's green component, in [0, 1] or unset.
    pub const ACCENT_GREEN: &str = "accent_green";
    /// The accent colour's blue component, in [0, 1] or unset.
    pub const ACCENT_BLUE: &str = "accent_blue";
    /// The contrast preference.
    pub const CONTRAST: &str = "contrast";
    /// The reduced-motion preference.
    pub const REDUCED_MOTION: &str = "reduced_motion";
    /// The GTK theme name.
    pub const GTK_THEME: &str = "gtk_theme";
    /// The icon theme name.
    pub const ICON_THEME: &str = "icon_theme";
    /// The cursor theme name.
    pub const CURSOR_THEME: &str = "cursor_theme";
    /// The cursor size, in pixels.
    pub const CURSOR_SIZE: &str = "cursor_size";
    /// The interface font name.
    pub const FONT_NAME: &str = "font_name";
    /// The document font name.
    pub const DOCUMENT_FONT_NAME: &str = "document_font_name";
    /// The monospace font name.
    pub const MONOSPACE_FONT_NAME: &str = "monospace_font_name";
    /// The text scaling factor.
    pub const TEXT_SCALING_FACTOR: &str = "text_scaling_factor";
    /// The window button layout string.
    pub const BUTTON_LAYOUT: &str = "button_layout";
    /// Whether animations are enabled.
    pub const ENABLE_ANIMATIONS: &str = "enable_animations";
}

/// The accent's unset sentinel: any component outside [0, 1].
const UNSET_ACCENT: f64 = -1.0;

/// The values the `settings` object is created with, before any configuration.
///
/// The colour scheme, gtk, icon, cursor, font, monospace and titlebar values are copied
/// from `DesktopSettings::default` in `crates/compat/gnoblin-compat-core/src/settings.rs`.
/// `contrast`, `reduced_motion` and `text_scaling_factor` take the neutral values the
/// schema comments name. `document_font_name` follows `font_name` and `enable_animations`
/// is `true`; both are GNOME stock, not copied from compat, so they are assumptions.
#[must_use]
pub fn defaults() -> Fields {
    Fields::from([
        (member::COLOR_SCHEME.to_owned(), Value::Uint(0)),
        (member::ACCENT_RED.to_owned(), Value::Double(UNSET_ACCENT)),
        (member::ACCENT_GREEN.to_owned(), Value::Double(UNSET_ACCENT)),
        (member::ACCENT_BLUE.to_owned(), Value::Double(UNSET_ACCENT)),
        (member::CONTRAST.to_owned(), Value::Uint(0)),
        (member::REDUCED_MOTION.to_owned(), Value::Uint(0)),
        (member::GTK_THEME.to_owned(), Value::Text("Adwaita".to_owned())),
        (member::ICON_THEME.to_owned(), Value::Text("Adwaita".to_owned())),
        (member::CURSOR_THEME.to_owned(), Value::Text("Adwaita".to_owned())),
        (member::CURSOR_SIZE.to_owned(), Value::Uint(24)),
        (member::FONT_NAME.to_owned(), Value::Text("Cantarell 11".to_owned())),
        (
            member::DOCUMENT_FONT_NAME.to_owned(),
            Value::Text("Cantarell 11".to_owned()),
        ),
        (
            member::MONOSPACE_FONT_NAME.to_owned(),
            Value::Text("Source Code Pro 10".to_owned()),
        ),
        (member::TEXT_SCALING_FACTOR.to_owned(), Value::Double(1.0)),
        (
            member::BUTTON_LAYOUT.to_owned(),
            Value::Text("appmenu:minimize,maximize,close".to_owned()),
        ),
        (member::ENABLE_ANIMATIONS.to_owned(), Value::Bool(true)),
    ])
}

/// The `settings` properties a configuration names; absent ones keep their stock value.
#[must_use]
pub fn from_config(config: &gnoblin_config::Config) -> Fields {
    let mut fields = Fields::new();
    if let Some(appearance) = &config.appearance {
        let scheme = match appearance.color_scheme {
            Some(gnoblin_config::ColorScheme::PreferDark) => 1,
            Some(gnoblin_config::ColorScheme::PreferLight) => 2,
            Some(gnoblin_config::ColorScheme::Default) | None => 0,
        };
        fields.insert(member::COLOR_SCHEME.to_owned(), Value::Uint(scheme));
        let (red, green, blue) = match appearance
            .accent
            .as_deref()
            .and_then(|accent| gnoblin_config::appearance::Accent::parse(accent).ok())
        {
            Some(accent) => accent.rgb().unit(),
            None => (UNSET_ACCENT, UNSET_ACCENT, UNSET_ACCENT),
        };
        fields.insert(member::ACCENT_RED.to_owned(), Value::Double(red));
        fields.insert(member::ACCENT_GREEN.to_owned(), Value::Double(green));
        fields.insert(member::ACCENT_BLUE.to_owned(), Value::Double(blue));
        // The accessibility preferences. `reduced-motion` also turns animations off, because the
        // `enable_animations` property answers the same question for GTK 3 that `reduced_motion`
        // answers for GTK 4 and the portal. `high-contrast` is the portal's `contrast` value 1.
        let reduced_motion = appearance.reduced_motion.unwrap_or(false);
        let high_contrast = appearance.high_contrast.unwrap_or(false);
        fields.insert(
            member::REDUCED_MOTION.to_owned(),
            Value::Uint(u64::from(reduced_motion)),
        );
        fields.insert(member::CONTRAST.to_owned(), Value::Uint(u64::from(high_contrast)));
        fields.insert(member::ENABLE_ANIMATIONS.to_owned(), Value::Bool(!reduced_motion));
    }
    if let Some(cursor) = &config.cursor {
        if let Some(theme) = cursor.theme.as_ref().filter(|theme| !theme.trim().is_empty()) {
            fields.insert(member::CURSOR_THEME.to_owned(), Value::Text(theme.clone()));
        }
        if let Some(size) = cursor.size.and_then(|size| u64::try_from(size).ok()) {
            fields.insert(member::CURSOR_SIZE.to_owned(), Value::Uint(size));
        }
    }
    fields
}

#[cfg(test)]
mod tests {
    use gnoblin_model::Value;

    use super::{defaults, from_config, member};

    /// The values `publish_settings` writes: the stock values, with the configuration's on top.
    fn published(json: &str) -> gnoblin_model::Fields {
        let config = gnoblin_config::Config::from_json_bytes(json.as_bytes()).expect("the document is valid");
        let mut fields = defaults();
        fields.extend(from_config(&config));
        fields
    }

    /// `reduced-motion = true` publishes the portal preference and turns animations off.
    #[test]
    fn reduced_motion_is_published_and_stops_animations() {
        let fields = published(r#"{"appearance":{"reduced-motion":true}}"#);
        assert_eq!(fields.get(member::REDUCED_MOTION), Some(&Value::Uint(1)));
        assert_eq!(fields.get(member::ENABLE_ANIMATIONS), Some(&Value::Bool(false)));
    }

    /// `high-contrast = true` publishes the portal's `contrast` value 1 and leaves animations on.
    #[test]
    fn high_contrast_is_published_as_contrast_one() {
        let fields = published(r#"{"appearance":{"high-contrast":true}}"#);
        assert_eq!(fields.get(member::CONTRAST), Some(&Value::Uint(1)));
        assert_eq!(fields.get(member::REDUCED_MOTION), Some(&Value::Uint(0)));
        assert_eq!(fields.get(member::ENABLE_ANIMATIONS), Some(&Value::Bool(true)));
    }

    /// A configuration that names neither preference, or no appearance at all, publishes the
    /// stock values, so turning a preference off on reload restores them.
    #[test]
    fn absent_preferences_publish_the_stock_values() {
        for json in [r#"{}"#, r#"{"appearance":{"color-scheme":"prefer-dark"}}"#] {
            let fields = published(json);
            assert_eq!(fields.get(member::CONTRAST), Some(&Value::Uint(0)), "{json}");
            assert_eq!(fields.get(member::REDUCED_MOTION), Some(&Value::Uint(0)), "{json}");
            assert_eq!(
                fields.get(member::ENABLE_ANIMATIONS),
                Some(&Value::Bool(true)),
                "{json}"
            );
        }
    }
}
