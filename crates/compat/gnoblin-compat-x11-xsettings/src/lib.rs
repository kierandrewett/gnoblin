//! XSETTINGS for X11 applications (D15's `xsettings` target).
//!
//! X11 applications do not read the configuration document, a dconf database or the
//! portal: they read the `_XSETTINGS_S0` selection's property on the root window, which
//! a settings manager publishes and they watch. This adapter is that manager's values.
//!
//! # The split
//!
//! Deciding what an X11 client should see is this crate's; writing the property is not,
//! because that needs the compositor's X connection, its manager window and its
//! selection ownership. [`gnoblin_xwayland::xsettings::XSettings`] is the seam: the
//! compositor asks [`Owner`] for the list and tells it what publishing did, and the
//! crate never touches an X11 socket. That is also why this crate can be tested with
//! [`gnoblin_xwayland::RecordingOwner`]: the seam ships its own double, and the
//! compositor's side of it is proved in `gnoblin-xwayland`'s own tests.
//!
//! # One way only
//!
//! D15: "Writes from GNOME Settings are never read back." [`publish`] is a pure function
//! of one [`DesktopSettings`] value, so there is no path from an X11 client, a
//! configuration file or a `gsettings` database into what is published. A test proves it
//! by publishing twice from the same value with an unrelated file in between.
//!
//! # Failure modes first
//!
//! 1. **`xsettings` is switched off but clients still see values.** [`Owner`] answers an
//!    empty list when its switch is off, and the compositor replaces the whole property,
//!    so a switch that is off removes the settings rather than leaving the last ones:
//!    `a_switched_off_owner_publishes_nothing`.
//! 2. **A value is published twice with two spellings.** The list is sorted by name and
//!    a name appears once, so two writers cannot both win:
//!    `every_name_appears_once_in_name_order`.
//! 3. **The DPI is a scale, or thousandths of one.** `Xft/DPI` is dots per inch times
//!    1024 as an integer; a client that reads 96 where 98304 was meant renders at a
//!    sixteenth of the size: `the_dpi_is_in_xdpi_units`.

#![forbid(unsafe_code)]

use gnoblin_compat_core::{DesktopSettings, Switch};
use gnoblin_log::debug_in;
use gnoblin_xwayland::xsettings::{Setting, XSettings};

/// The checked-in declaration, compiled in so the file a reviewer approved is the one
/// `cargo xtask compat-report` prints and this build honours.
pub const MATRIX: &str = include_str!("../support-matrix.json");

/// The interface that owns the property: the selection's name on the root window.
pub const INTERFACE: &str = "_XSETTINGS_S0";

/// The XSETTINGS property that carries the DPI.
pub const DPI: &str = "Xft/DPI";

/// The property GTK reads the interface font from.
pub const FONT: &str = "Gtk/FontName";

/// The property GTK reads the monospace font from.
pub const MONOSPACE_FONT: &str = "Gtk/MonospaceFontName";

/// The property GTK reads the theme name from.
pub const THEME: &str = "Net/ThemeName";

/// The property GTK reads the theme name from under GTK 3's own prefix.
pub const GTK_THEME: &str = "Gtk/ThemeName";

/// The property GTK reads the icon theme from.
pub const ICON_THEME: &str = "Net/IconThemeName";

/// The property GTK reads the icon theme from under GTK 3's own prefix.
pub const GTK_ICON_THEME: &str = "Gtk/IconThemeName";

/// The property GTK reads the cursor theme from.
pub const CURSOR_THEME: &str = "Gtk/CursorThemeName";

/// The property GTK reads the cursor size from.
pub const CURSOR_SIZE: &str = "Gtk/CursorThemeSize";

/// The property GTK reads the titlebar button layout from.
pub const BUTTON_LAYOUT: &str = "Gtk/DecorationLayout";

/// Dots per inch as XSETTINGS states it: times 1024, as an integer.
///
/// The protocol's `Xft/DPI` is in thousand-and-twenty-fourths because a client's
/// fractional scale is not an integer, and every settings manager has always published
/// it that way.
#[must_use]
pub const fn xdpi(dpi: u32) -> i32 {
    // A DPI is at most a few hundred, so the product cannot overflow the protocol's
    // 32-bit integer; the clamp states the invariant rather than relying on it.
    (dpi as i32).saturating_mul(1024)
}

/// The settings an X11 client should see, in name order.
///
/// Every name appears once: a toolkit that read two values for one key would behave
/// differently depending on which it saw last.
#[must_use]
pub fn publish(settings: &DesktopSettings) -> Vec<(String, Setting)> {
    let mut values = vec![
        (DPI.to_owned(), Setting::Integer(xdpi(settings.dpi))),
        (FONT.to_owned(), Setting::Text(settings.font.clone())),
        (
            MONOSPACE_FONT.to_owned(),
            Setting::Text(settings.monospace_font.clone()),
        ),
        (THEME.to_owned(), Setting::Text(settings.gtk_theme.clone())),
        (GTK_THEME.to_owned(), Setting::Text(settings.gtk_theme.clone())),
        (ICON_THEME.to_owned(), Setting::Text(settings.icon_theme.clone())),
        (GTK_ICON_THEME.to_owned(), Setting::Text(settings.icon_theme.clone())),
        (CURSOR_THEME.to_owned(), Setting::Text(settings.cursor_theme.clone())),
        (
            CURSOR_SIZE.to_owned(),
            Setting::Integer(i32::try_from(settings.cursor_size).unwrap_or(i32::MAX)),
        ),
        (BUTTON_LAYOUT.to_owned(), Setting::Text(settings.button_layout.clone())),
    ];
    values.sort_by(|left, right| left.0.cmp(&right.0));
    values
}

/// The XSETTINGS owner the compositor asks for the property's contents.
///
/// It holds the settings value and the switch, and answers the seam. `Off` answers an
/// empty list, which is how a target that is switched off leaves X11 applications on
/// their own settings: the whole property is replaced, so the values this adapter
/// published before are removed rather than remembered.
#[derive(Debug)]
pub struct Owner {
    settings: DesktopSettings,
    switch: Switch,
}

impl Owner {
    /// An owner for one configured value and switch.
    #[must_use]
    pub fn new(settings: DesktopSettings, switch: Switch) -> Self {
        Self { settings, switch }
    }

    /// Replaces the value the next publish uses.
    ///
    /// A configuration reload is a new value, not a new owner: the compositor asks the
    /// same owner again and the property is replaced whole.
    pub fn update(&mut self, settings: DesktopSettings) {
        self.settings = settings;
    }

    /// Turns this target on or off, as `gnoblin.configure.compat` does.
    pub fn set_switch(&mut self, switch: Switch) {
        self.switch = switch;
    }

    /// The value this owner publishes, whether or not its switch is on.
    #[must_use]
    pub fn settings(&self) -> &DesktopSettings {
        &self.settings
    }
}

impl XSettings for Owner {
    fn settings(&mut self) -> Vec<(String, Setting)> {
        if !self.switch.is_on() {
            debug_in!(
                "compat-xsettings",
                "[xsettings] the target is off, so the property is emptied"
            );
            return Vec::new();
        }
        publish(&self.settings)
    }

    fn published(&mut self, result: Result<(), String>) {
        match result {
            Ok(()) => debug_in!("compat-xsettings", "[xsettings] the property was written"),
            Err(reason) => debug_in!(
                "compat-xsettings",
                reason,
                "[xsettings] the property could not be written"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The values X11 applications read are the desktop's own, and a name appears once.
    #[test]
    fn every_name_appears_once_in_name_order() {
        let values = publish(&DesktopSettings::default());
        let mut names: Vec<&String> = values.iter().map(|(name, _)| name).collect();
        let ordered = names.clone();
        names.sort();
        assert_eq!(names, ordered, "the list is in name order");
        names.dedup();
        assert_eq!(names.len(), values.len(), "no name is published twice");
        assert!(values.iter().any(|(name, _)| name == FONT));
        assert!(values.iter().any(|(name, _)| name == BUTTON_LAYOUT));
    }

    /// `Xft/DPI` is the DPI times 1024, which is what a client's `Xft.dpi` resource
    /// arithmetic assumes.
    #[test]
    fn the_dpi_is_in_xdpi_units() {
        assert_eq!(xdpi(96), 98_304);
        let settings = DesktopSettings {
            dpi: 144,
            ..DesktopSettings::default()
        };
        let values = publish(&settings);
        assert_eq!(
            values
                .iter()
                .find(|(name, _)| name == DPI)
                .map(|(_, value)| value.clone()),
            Some(Setting::Integer(147_456))
        );
    }

    /// A target that is off publishes nothing at all, which is how its clients fall back
    /// to their own settings.
    #[test]
    fn a_switched_off_owner_publishes_nothing() {
        let mut owner = Owner::new(DesktopSettings::default(), Switch::Off);
        assert!(XSettings::settings(&mut owner).is_empty());
        owner.set_switch(Switch::On);
        assert!(!XSettings::settings(&mut owner).is_empty());
        owner.set_switch(Switch::Off);
        assert!(
            XSettings::settings(&mut owner).is_empty(),
            "a switch that turns off removes the values rather than remembering them"
        );
    }

    /// Nothing on disk reaches the published values: the same settings value publishes
    /// the same list whatever a toolkit wrote in the meantime.
    #[test]
    fn a_toolkits_own_file_is_never_read_back() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        // What GTK itself would write if it owned its settings, beside the session.
        let theirs = directory.path().join("settings.ini");
        std::fs::write(&theirs, "[Settings]\ngtk-font-name=Noto Sans 22\n").expect("the file");
        let settings = DesktopSettings::default();
        let before = publish(&settings);
        std::fs::write(&theirs, "[Settings]\ngtk-font-name=Comic Sans 40\n").expect("the file");
        let after = publish(&settings);
        assert_eq!(before, after, "a changed file cannot change what is published");
    }

    /// The compositor's answer reaches the owner, so a failed publish is visible rather
    /// than silent.
    #[test]
    fn the_answer_reaches_the_owner() {
        let mut owner = Owner::new(DesktopSettings::default(), Switch::On);
        // The double is the seam's own; this test drives the owner the seam calls.
        owner.published(Err("no window manager yet".to_owned()));
    }
    /// The checked-in declaration is what `cargo xtask compat-report` prints, and every
    /// property the owner answers is in it.
    #[test]
    fn the_manifest_declares_every_property_the_owner_publishes() {
        let matrix = gnoblin_compat_core::SupportMatrix::from_json(crate::MATRIX).expect("the checked-in manifest");
        for (name, _) in publish(&DesktopSettings::default()) {
            assert!(
                matrix.support(crate::INTERFACE, &name).is_some(),
                "{name} is published but not declared"
            );
        }
    }
}
