//! The desktop settings the compat adapters publish, and the session environment they need.
//!
//! The Qt adapter is exported into the guardian's session environment, because a Qt
//! application inherits that environment and reads its platform theme from it. The dconf
//! layer is exported the same way: `DCONF_PROFILE` points every child at the profile the
//! services role wrote, so GTK reads the configuration's values.

use std::path::Path;

use gnoblin_compat_core::{Mode, Switch};

use crate::services::dconf;

/// Exports `DCONF_PROFILE` into `environment` when the dconf target is on.
///
/// The layer lives in `directory`, the session's dconf profile directory. A switched-off mode
/// exports nothing, so dconf reads the user's database as it did before Gnoblin.
pub fn export_dconf_profile(environment: &mut gnoblin_session::Environment, mode: Mode, directory: &Path) {
    if mode == Mode::Off {
        return;
    }
    environment.set("DCONF_PROFILE", directory.join(dconf::PROFILE).display().to_string());
}

/// Exports the Qt platform theme into `environment` when the Qt target is on.
///
/// The adapter decides what is exported; this only copies its settings into the guardian's
/// session environment, which is what every child inherits.
pub fn export_qt_settings(environment: &mut gnoblin_session::Environment, mode: Mode) {
    let effects = gnoblin_compat_qt_portal_settings::apply(Switch::from_enabled(mode != Mode::Off));
    for effect in effects {
        if let gnoblin_session::Effect::ExportEnv {
            target: gnoblin_session::EnvTarget::Session,
            environment: exported,
        } = effect
        {
            for (key, value) in exported.set {
                environment.set(key, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{export_dconf_profile, export_qt_settings};
    use gnoblin_compat_core::Mode;
    use gnoblin_session::Environment;

    fn platform_theme(environment: &Environment) -> Option<&str> {
        environment
            .set
            .iter()
            .find(|(key, _)| key == "QT_QPA_PLATFORMTHEME")
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn the_qt_platform_theme_is_exported_when_the_mode_is_on() {
        let mut environment = Environment::default();
        export_qt_settings(&mut environment, Mode::Portal);
        assert_eq!(platform_theme(&environment), Some("xdgdesktopportal"));
    }

    fn dconf_profile(environment: &Environment) -> Option<&str> {
        environment
            .set
            .iter()
            .find(|(key, _)| key == "DCONF_PROFILE")
            .map(|(_, value)| value.as_str())
    }

    /// The guardian points dconf at the layer the services role wrote, when the mode is on.
    #[test]
    fn the_dconf_profile_is_exported_when_the_mode_is_on() {
        let mut environment = Environment::default();
        let directory = std::path::Path::new("/run/user/1000/gnoblin/dconf");
        export_dconf_profile(&mut environment, Mode::Portal, directory);
        assert_eq!(
            dconf_profile(&environment),
            Some("/run/user/1000/gnoblin/dconf/profile")
        );
    }

    #[test]
    fn the_dconf_profile_is_absent_when_the_mode_is_off() {
        let mut environment = Environment::default();
        export_dconf_profile(
            &mut environment,
            Mode::Off,
            std::path::Path::new("/run/user/1000/gnoblin/dconf"),
        );
        assert_eq!(dconf_profile(&environment), None);
    }

    #[test]
    fn the_qt_platform_theme_is_absent_when_the_mode_is_off() {
        let mut environment = Environment::default();
        export_qt_settings(&mut environment, Mode::Off);
        assert_eq!(platform_theme(&environment), None);
    }
}

/// Exports every settings target into the guardian's session environment, which each child
/// inherits: the mode is read once from the environment, then the Qt platform theme and the
/// dconf profile follow from it.
///
/// The dconf profile is exported only when the session's profile directory is known.
pub fn export_guardian_environment(environment: &mut gnoblin_session::Environment) {
    let mode = gnoblin_compat_gnome_shell_introspect::mode_from_environment();
    export_qt_settings(environment, mode);
    if let Some(directory) = dconf::profile_directory() {
        export_dconf_profile(environment, mode, &directory);
    }
}
