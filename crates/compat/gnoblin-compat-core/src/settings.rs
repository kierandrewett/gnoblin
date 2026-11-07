//! The settings a publisher sends to other toolkits (D15).
//!
//! D15 publishes the desktop's settings **one way**: Gnoblin's configuration is the
//! source, a per-toolkit switch in `gnoblin.configure.compat` decides whether the
//! target hears about it at all, and nothing a toolkit writes is read back. The
//! publishers are three adapters — [`gnoblin-compat-file-gtk-dconf-settings`],
//! [`gnoblin-compat-file-qt-portal-settings`] and [`gnoblin-compat-x11-xsettings`] — and
//! the vocabulary they publish lives here because two of them name the same values
//! and the plan forbids a compat crate depending on another.
//!
//! Two decisions are worth stating rather than implying:
//!
//! - **The strings are already in the desktop's own spelling.** A font reads
//!   `Cantarell 11`, the way `org.gnome.desktop.interface font-name` and
//!   `Gtk/FontName` both write it. Each publisher formats for its target; none of
//!   them parses a font, so a font name a user typed survives the round trip byte
//!   for byte.
//! - **A resolution is a DPI, not a scale.** XSETTINGS and GTK both speak
//!   `Xft/DPI` in thousand-and-twenty-fourths of a dot per inch, so the published
//!   value is the DPI the compositor scale works out to (96 at scale 1). A scale
//!   that does not land on a whole DPI is rounded, which is what every other
//!   desktop does and what a toolkit can use.
//!
//! # Failure modes first
//!
//! 1. **A publisher invents a value the desktop never declared.** Every field of
//!    [`DesktopSettings::default`] is GNOME's own default, so a session that
//!    configures nothing publishes the values a stock GNOME session would:
//!    `gnoblin-compat-file-gtk-dconf-settings`'s `the_defaults_are_gnomes_own`.
//! 2. **A target that is switched off still hears something.** [`Switch::Off`] is
//!    what every publisher's applier checks first, and its test asserts that the
//!    target received no action at all rather than a neutral one.
//! 3. **A colour-scheme spelling reaches a toolkit that does not know it.** The
//!    three values are the ones GTK, the portal and `gsettings` all accept, and
//!    [`ColourScheme::parse`] refuses anything else instead of guessing.

use std::fmt;

use gnoblin_config::appearance::Accent;

use crate::log;

/// The colour scheme a toolkit should prefer.
///
/// The spellings are the ones `org.gnome.desktop.interface color-scheme` and the
/// portal's `color-scheme` key use, so a publisher writes one and a toolkit reads
/// the same string. There is no `prefer-light` in older GNOME releases, which is
/// why it is not the default.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum ColourScheme {
    /// Follow the toolkit's own default.
    #[default]
    NoPreference,
    /// Prefer a dark appearance.
    PreferDark,
    /// Prefer a light appearance.
    PreferLight,
}

impl ColourScheme {
    /// The configuration and `gsettings` spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoPreference => "default",
            Self::PreferDark => "prefer-dark",
            Self::PreferLight => "prefer-light",
        }
    }

    /// Parse the configured spelling.
    ///
    /// `None` means the value is not one of the three; a caller reports it against
    /// the key that held it instead of publishing a scheme the toolkit would
    /// ignore.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "default" => Some(Self::NoPreference),
            "prefer-dark" => Some(Self::PreferDark),
            "prefer-light" => Some(Self::PreferLight),
            _ => None,
        }
    }
}

impl fmt::Display for ColourScheme {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Whether one target in `gnoblin.configure.compat` is on.
///
/// D15 gives every target its own switch, and the default is on: a session that
/// configures nothing publishes, which is what makes GTK, Qt and X11 applications
/// look like the desktop on a fresh install. Off is a complete statement rather
/// than a partial one — a target that is off receives no action, so the
/// application keeps whatever it had.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum Switch {
    /// Publish to this target.
    #[default]
    On,
    /// Publish nothing; the application keeps its own settings.
    Off,
}

impl Switch {
    /// Whether the target hears anything.
    #[must_use]
    pub const fn is_on(self) -> bool {
        matches!(self, Self::On)
    }

    /// The configuration spelling, which is the boolean the key holds.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
        }
    }

    /// The switch a configured boolean means.
    #[must_use]
    pub const fn from_enabled(enabled: bool) -> Self {
        if enabled { Self::On } else { Self::Off }
    }
}

impl fmt::Display for Switch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The settings D15 publishes to other toolkits.
///
/// Every field is a value a toolkit reads: GTK's `gsettings` keys, XSETTINGS
/// properties and the portal's own keys are three copies of the same handful of
/// facts, and this is the one place they are stated. Adding a field is adding a
/// fact the desktop declares, not adding a toolkit.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DesktopSettings {
    /// The GTK theme name, such as `Adwaita`.
    pub gtk_theme: String,
    /// The icon theme name, such as `Adwaita`.
    pub icon_theme: String,
    /// The interface font, in the desktop's own spelling: `Cantarell 11`.
    pub font: String,
    /// The monospace font, in the same spelling.
    pub monospace_font: String,
    /// The window-title font, in the same spelling.
    pub titlebar_font: String,
    /// The cursor theme name, such as `Adwaita`.
    pub cursor_theme: String,
    /// The cursor size in logical pixels.
    pub cursor_size: u32,
    /// The titlebar button layout, as `gtk-decoration-layout` writes it:
    /// `appmenu:minimize,maximize,close`.
    pub button_layout: String,
    /// The colour scheme to prefer.
    pub colour_scheme: ColourScheme,
    /// The dots per inch the interface scale works out to: 96 at scale 1.
    pub dpi: u32,
    /// The accent colour, when the configuration names one. `None` leaves the toolkit's own accent.
    pub accent: Option<Accent>,
}

impl DesktopSettings {
    /// The DPI a compositor scale works out to.
    ///
    /// A scale of 1 is 96 dots per inch, which is what every toolkit assumes when
    /// nothing tells it otherwise, so a session that reports 96 is indistinguishable
    /// from one that reports nothing.
    #[must_use]
    pub fn dpi_for_scale(scale: f64) -> u32 {
        let dpi = (96.0 * scale).round();
        // A scale is never zero or negative in the workspace's own `Scale` type, and
        // a value that is still nonsense here would produce a DPI no toolkit can use,
        // so the floor and the ceiling are the ones a screen can actually have.
        dpi.clamp(48.0, 768.0) as u32
    }
}

impl Default for DesktopSettings {
    /// A stock GNOME session's values, which is what a session that configures
    /// nothing declares.
    fn default() -> Self {
        Self {
            gtk_theme: "Adwaita".to_owned(),
            icon_theme: "Adwaita".to_owned(),
            font: "Cantarell 11".to_owned(),
            monospace_font: "Source Code Pro 10".to_owned(),
            titlebar_font: "Cantarell Bold 11".to_owned(),
            cursor_theme: "Adwaita".to_owned(),
            cursor_size: 24,
            button_layout: "appmenu:minimize,maximize,close".to_owned(),
            colour_scheme: ColourScheme::NoPreference,
            dpi: 96,
            accent: None,
        }
    }
}

/// The desktop settings the configuration declares, over [`DesktopSettings::default`].
///
/// The `cursor` section is the one table that names a value a publisher sends on: its theme and
/// size. Every other field keeps the stock value until a section declares it.
#[must_use]
pub fn settings_from_config(config: &gnoblin_config::Config) -> DesktopSettings {
    let mut settings = DesktopSettings::default();
    if let Some(cursor) = &config.cursor {
        if let Some(theme) = cursor.theme.as_ref().filter(|theme| !theme.trim().is_empty()) {
            settings.cursor_theme = theme.clone();
        }
        if let Some(size) = cursor.size.and_then(|size| u32::try_from(size).ok()) {
            settings.cursor_size = size;
        }
    }
    if let Some(appearance) = &config.appearance {
        settings.colour_scheme = appearance
            .color_scheme
            .map_or(ColourScheme::NoPreference, |scheme| match scheme {
                gnoblin_config::ColorScheme::Default => ColourScheme::NoPreference,
                gnoblin_config::ColorScheme::PreferDark => ColourScheme::PreferDark,
                gnoblin_config::ColorScheme::PreferLight => ColourScheme::PreferLight,
            });
        settings.accent = appearance
            .accent
            .as_deref()
            .and_then(|accent| Accent::parse(accent).ok());
    }
    settings
}

/// The desktop settings the session's configuration names, read from the environment.
///
/// A missing or unreadable configuration is the stock defaults, so a session that cannot read its
/// configuration still publishes what GNOME publishes. The reason is logged.
#[must_use]
pub fn settings_from_environment() -> DesktopSettings {
    let Some(store) = gnoblin_config::Store::from_env() else {
        return DesktopSettings::default();
    };
    match gnoblin_config::locate(&gnoblin_config::SearchPaths::from_env()) {
        Ok(path) => match store.load(path.path()) {
            Ok(stored) => settings_from_config(&stored.config),
            Err(error) => {
                log::warn!(area = "settings", %error, "configuration not readable; publishing the defaults");
                DesktopSettings::default()
            }
        },
        Err(error) => {
            log::debug!(area = "settings", %error, "no configuration file; publishing the defaults");
            DesktopSettings::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scale becomes the DPI a toolkit can use, and a nonsense scale does not
    /// become a nonsense DPI.
    #[test]
    fn a_scale_becomes_a_dpi() {
        assert_eq!(DesktopSettings::dpi_for_scale(1.0), 96);
        assert_eq!(DesktopSettings::dpi_for_scale(1.5), 144);
        assert_eq!(DesktopSettings::dpi_for_scale(2.0), 192);
        assert_eq!(DesktopSettings::dpi_for_scale(0.0), 48);
        assert_eq!(DesktopSettings::dpi_for_scale(1000.0), 768);
    }

    /// The spellings a toolkit uses and the ones this crate renders are the same
    /// bytes, so nothing has to be translated at a seam.
    #[test]
    fn the_spellings_are_the_toolkits_own() {
        assert_eq!(ColourScheme::PreferDark.to_string(), "prefer-dark");
        assert_eq!(ColourScheme::parse("prefer-light"), Some(ColourScheme::PreferLight));
        assert_eq!(ColourScheme::parse("dark"), None);
        assert_eq!(Switch::from_enabled(false), Switch::Off);
        assert!(!Switch::Off.is_on());
        assert!(Switch::default().is_on());
    }

    /// A configuration's `cursor` section is the cursor a publisher sends, and a section that names
    /// nothing leaves the stock cursor in place.
    #[test]
    fn the_cursor_section_is_the_published_cursor() {
        let config = gnoblin_config::Config::from_value(
            gnoblin_config::Value::from_json(&serde_json::json!({"cursor": {"theme": "Breeze", "size": 32}}))
                .expect("an object"),
        )
        .expect("a valid configuration");
        let settings = settings_from_config(&config);
        assert_eq!(settings.cursor_theme, "Breeze");
        assert_eq!(settings.cursor_size, 32);
        assert_eq!(settings.font, DesktopSettings::default().font);
    }

    #[test]
    fn a_configuration_without_a_cursor_keeps_the_stock_cursor() {
        let config = gnoblin_config::Config::from_value(
            gnoblin_config::Value::from_json(&serde_json::json!({})).expect("an object"),
        )
        .expect("a valid configuration");
        assert_eq!(settings_from_config(&config), DesktopSettings::default());
    }

    /// The appearance section is the colour scheme and accent a publisher sends; an absent section
    /// sends neither, so the toolkit keeps its own.
    #[test]
    fn the_appearance_section_is_the_published_scheme_and_accent() {
        let config = gnoblin_config::Config::from_value(
            gnoblin_config::Value::from_json(
                &serde_json::json!({"appearance": {"color-scheme": "prefer-dark", "accent": "#3584e4"}}),
            )
            .expect("an object"),
        )
        .expect("a valid configuration");
        let settings = settings_from_config(&config);
        assert_eq!(settings.colour_scheme, ColourScheme::PreferDark);
        assert_eq!(settings.accent.map(Accent::name), Some("blue"));

        let bare = gnoblin_config::Config::from_value(
            gnoblin_config::Value::from_json(&serde_json::json!({})).expect("an object"),
        )
        .expect("a valid configuration");
        let bare = settings_from_config(&bare);
        assert_eq!(bare.colour_scheme, ColourScheme::NoPreference);
        assert_eq!(bare.accent, None);
    }
}
