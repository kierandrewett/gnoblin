//! The desktop settings the compat adapters publish, and the session environment they need.
//!
//! The Qt adapter is exported into the guardian's session environment, because a Qt
//! application inherits that environment and reads its platform theme from it.

use gnoblin_compat_core::{Mode, Switch};

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
    use super::export_qt_settings;
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

    #[test]
    fn the_qt_platform_theme_is_absent_when_the_mode_is_off() {
        let mut environment = Environment::default();
        export_qt_settings(&mut environment, Mode::Off);
        assert_eq!(platform_theme(&environment), None);
    }
}
