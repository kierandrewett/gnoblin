//! Qt's settings, by pointing Qt at the portal (D15's `qt` target).
//!
//! Qt 6.5 and later can read the desktop's colour scheme, accent colour and contrast from
//! the portal's `org.freedesktop.portal.Settings` interface, but only when the
//! `xdgdesktopportal` platform theme is its style: without it, Qt reads its own config
//! files and knows nothing about the session. D15's decision is therefore the whole of
//! this adapter — set `QT_QPA_PLATFORMTHEME=xdgdesktopportal` in the session environment
//! and write **no Qt config file at all**.
//!
//! # Why the guardian and not the compositor
//!
//! The variable has to be in the environment a Qt application *inherits*, which is the
//! guardian's: it starts every child and mirrors its environment into the D-Bus
//! activation environment, so an application activated over D-Bus sees the same value.
//! [`publish`] answers an [`Export`], [`apply`] turns it into the guardian's own
//! [`Effect::ExportEnv`], and this crate never writes a file: two writers of
//! `qt5ct.conf` is exactly the second source of truth D15 rejected.
//!
//! # Failure modes first
//!
//! 1. **The target is switched off and the session still sets the variable.** [`apply`]
//!    answers no effects at all when the switch is off, so a session that turns the `qt`
//!    target off leaves Qt on its own settings:
//!    `a_switched_off_target_exports_nothing`.
//! 2. **A Qt config file is written anyway.** The crate has no file-writing code and a
//!    test asserts the session's own configuration directory is untouched:
//!    `no_qt_file_is_written`.
//! 3. **The variable is exported to the wrong environment.** It goes to
//!    [`EnvTarget::Session`], which is what children inherit; exporting it only to the
//!    activation environment would leave every application the session started itself
//!    without a theme: `the_export_targets_the_session`.

#![forbid(unsafe_code)]

use gnoblin_compat_core::Switch;
use gnoblin_session::{Effect, EnvTarget, Environment};

/// The checked-in declaration, compiled in so the file a reviewer approved is the one
/// `cargo xtask compat-report` prints and this build honours.
pub const MATRIX: &str = include_str!("../support-matrix.json");

/// The variable Qt 6.5 reads its platform theme from.
pub const PLATFORM_THEME: &str = "QT_QPA_PLATFORMTHEME";

/// The platform theme that forwards to the portal.
pub const PORTAL_THEME: &str = "xdgdesktopportal";

/// One environment value to export to the session.
///
/// A value rather than an effect, so what this adapter asks for can be read and compared
/// without a guardian, and turning it into an effect is one call in [`apply`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Export {
    /// The variable's name.
    pub name: &'static str,
    /// The value to set it to.
    pub value: &'static str,
}

/// What the session should export for Qt.
///
/// There is exactly one value, and there is no input: Qt needs nothing from the desktop's
/// settings because the portal interface is where it reads them. A future target that did
/// need a value would add it here with the same shape.
#[must_use]
pub fn publish() -> Vec<Export> {
    vec![Export {
        name: PLATFORM_THEME,
        value: PORTAL_THEME,
    }]
}

/// The effects that make the session export what [`publish`] asks for.
///
/// A switch that is off answers no effects: the variable is then whatever the session
/// inherited, which is the "Qt reads its own settings" state D15 describes.
#[must_use]
pub fn apply(switch: Switch) -> Vec<Effect> {
    if !switch.is_on() {
        return Vec::new();
    }
    let mut environment = Environment::new();
    for export in publish() {
        environment.set(export.name, export.value);
    }
    vec![Effect::ExportEnv {
        target: EnvTarget::Session,
        environment,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one value is Qt's own spelling, and it is the portal theme rather than a
    /// toolkit's.
    #[test]
    fn the_value_is_the_portals_theme() {
        let exports = publish();
        assert_eq!(exports.len(), 1, "Qt needs one variable and no config file");
        assert_eq!(exports[0].name, "QT_QPA_PLATFORMTHEME");
        assert_eq!(exports[0].value, "xdgdesktopportal");
    }

    /// A target that is off exports nothing, so Qt keeps its own settings.
    #[test]
    fn a_switched_off_target_exports_nothing() {
        assert!(apply(Switch::Off).is_empty());
        assert_eq!(apply(Switch::On).len(), 1);
    }

    /// The value goes to the environment children inherit, not only to the activation
    /// environment.
    #[test]
    fn the_export_targets_the_session() {
        let effects = apply(Switch::On);
        let Effect::ExportEnv { target, environment } = effects.first().expect("one effect") else {
            panic!("the only effect is an environment export");
        };
        assert_eq!(*target, EnvTarget::Session);
        assert_eq!(environment.get(PLATFORM_THEME), Some(PORTAL_THEME));
    }

    /// Nothing is written: the crate's only output is an effect, and an application's own
    /// configuration directory is untouched.
    #[test]
    fn no_qt_file_is_written() {
        let home = tempfile::tempdir().expect("a temporary directory");
        let configuration = home.path().join(".config/qt5ct");
        let _ = apply(Switch::On);
        assert!(
            !configuration.exists(),
            "no Qt configuration is written by this adapter"
        );
        assert_eq!(
            std::fs::read_dir(home.path()).expect("the directory").count(),
            0,
            "the adapter wrote nothing beside the session's own files"
        );
    }
    /// The checked-in declaration is what `cargo xtask compat-report` prints, and it
    /// names the one variable this adapter exports.
    #[test]
    fn the_manifest_declares_the_exported_variable() {
        let matrix = gnoblin_compat_core::SupportMatrix::from_json(crate::MATRIX).expect("the checked-in manifest");
        assert!(matrix.support(PLATFORM_THEME, "XDG_CURRENT_DESKTOP").is_some());
        for export in publish() {
            assert_eq!(export.name, PLATFORM_THEME);
        }
    }
}
