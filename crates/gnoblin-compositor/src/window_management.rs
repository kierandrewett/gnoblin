//! The stacking, placement, drag, focus and liveness policy, from the `window-management`
//! configuration section.
//!
//! A document with no `window-management` section keeps the built-in defaults. A section
//! with a field missing keeps that field's default. The values are checked by the schema
//! before they reach this module, so an unknown or out-of-range value is refused there and
//! never arrives here.

use std::path::Path;
use std::time::Duration;

use gnoblin_config::{FocusMode as ConfiguredFocusMode, FocusNewWindows, SearchPaths, Store, WindowManagement};

use gnoblin_wayland::{DragConstraint, FocusMode, NewWindowFocus, TitlebarActions, WmOptions};

use crate::SessionConfig;

use crate::log;

/// The declared workspaces as the `id:name` items the main seat publishes. A workspace with no id takes its
/// position, from 1; one with no name is named by its id.
fn workspace_names(declared: &[gnoblin_config::Workspace]) -> Vec<String> {
    declared
        .iter()
        .enumerate()
        .map(|(index, workspace)| {
            let id = workspace.id.clone().unwrap_or_else(|| (index + 1).to_string());
            let name = workspace.name.as_deref().unwrap_or(&id);
            format!("{id}:{name}")
        })
        .collect()
}

/// Sets the window-management fields of `config.wayland` from the last accepted
/// configuration in `store`, read from `source`.
pub fn configure_from_store(config: &mut SessionConfig, store: &Store, source: &Path) {
    match store.load(source) {
        Ok(stored) => {
            let section = stored.config.window_management.clone().unwrap_or_default();
            apply(config, &section);
            config.wayland.workspace_names = workspace_names(&stored.config.workspaces);
            log::debug!(
                area = "window-management",
                "loaded from the last accepted configuration"
            );
        }
        Err(error) if error.is_absent() => {
            log::debug!(area = "window-management", %error, "no configuration file; the defaults are used")
        }
        Err(error) => {
            log::warn!(area = "window-management", %error, "configuration not readable; the defaults are used")
        }
    }
}

/// [`configure_from_store`] for the process's own state directory and configuration path.
///
/// With no configuration file there is no accepted document, so the defaults are used.
pub fn configure_from_environment(config: &mut SessionConfig) {
    let Some(store) = Store::from_env() else {
        log::debug!(
            area = "window-management",
            "no state directory is set; the defaults are used"
        );
        return;
    };
    match gnoblin_config::locate(&SearchPaths::from_env()) {
        Ok(path) => configure_from_store(config, &store, path.path()),
        Err(error) => {
            log::debug!(area = "window-management", %error, "no configuration file; the defaults are used")
        }
    }
}

/// Copies an accepted `window-management` section into the compositor configuration. A leaf
/// the section leaves out keeps its compositor default.
fn apply(config: &mut SessionConfig, section: &WindowManagement) {
    config.wayland.wm = wm_options_of(section);
    config.wayland.titlebar_actions = TitlebarActions::of(section);
    config.wayland.check_alive_timeout = check_alive_timeout_of(section);
}

/// The check-alive timeout a `window-management` section sets, or the compositor's default when
/// it leaves the key out. Startup and a committed reload both read it from here.
pub(crate) fn check_alive_timeout_of(section: &WindowManagement) -> Duration {
    let default_timeout = gnoblin_wayland::Config::default().check_alive_timeout;
    section
        .check_alive_timeout
        .and_then(|ms| u64::try_from(ms).ok())
        .map_or(default_timeout, Duration::from_millis)
}

/// The window manager options a `window-management` section sets. A leaf the section leaves
/// out keeps its compositor default. Startup and a committed reload both read the policy from
/// here, so the two cannot disagree about what a section means.
pub(crate) fn wm_options_of(section: &WindowManagement) -> WmOptions {
    let wm_defaults = WmOptions::default();
    WmOptions {
        edge_tiling: section.edge_tiling.unwrap_or(wm_defaults.edge_tiling),
        drag_constraint: section
            .constrain_drag_to_work_area
            .map_or(wm_defaults.drag_constraint, |constrain| {
                if constrain {
                    DragConstraint::WorkArea
                } else {
                    DragConstraint::MinimumStrip
                }
            }),
        new_window_focus: section
            .focus_new_windows
            .map_or(wm_defaults.new_window_focus, |focus| match focus {
                FocusNewWindows::Allow => NewWindowFocus::Allow,
                FocusNewWindows::Prevent => NewWindowFocus::Prevent,
            }),
        focus_mode: section.focus_mode.map_or(wm_defaults.focus_mode, focus_mode),
        focus_change_on_pointer_rest: section
            .focus_change_on_pointer_rest
            .unwrap_or(wm_defaults.focus_change_on_pointer_rest),
        raise_on_click: section.raise_on_click.unwrap_or(wm_defaults.raise_on_click),
        auto_raise: section.auto_raise.unwrap_or(wm_defaults.auto_raise),
        auto_raise_delay: section
            .auto_raise_delay
            .and_then(|ms| u64::try_from(ms).ok())
            .unwrap_or(wm_defaults.auto_raise_delay),
    }
}

/// The pointer focus mode for a configured `focus-mode`.
fn focus_mode(mode: ConfiguredFocusMode) -> FocusMode {
    match mode {
        ConfiguredFocusMode::Click => FocusMode::Click,
        ConfiguredFocusMode::Hover => FocusMode::Hover,
        ConfiguredFocusMode::HoverStrict => FocusMode::HoverStrict,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gnoblin_config::{Config, Value};

    #[test]
    fn an_absent_section_keeps_the_defaults() {
        let mut config = SessionConfig::default();
        apply(&mut config, &WindowManagement::default());
        assert_eq!(config.wayland.wm, WmOptions::default());
        assert_eq!(config.wayland.titlebar_actions, TitlebarActions::default());
        assert_eq!(config.wayland.check_alive_timeout, Duration::from_secs(5));
    }

    #[test]
    fn every_leaf_maps_into_the_compositor_configuration() {
        let section = WindowManagement {
            edge_tiling: Some(true),
            constrain_drag_to_work_area: Some(false),
            check_alive_timeout: Some(1500),
            focus_new_windows: Some(FocusNewWindows::Allow),
            ..WindowManagement::default()
        };

        let mut config = SessionConfig::default();
        apply(&mut config, &section);

        assert!(config.wayland.wm.edge_tiling);
        assert_eq!(config.wayland.wm.drag_constraint, DragConstraint::MinimumStrip);
        assert_eq!(config.wayland.wm.new_window_focus, NewWindowFocus::Allow);
        assert_eq!(config.wayland.check_alive_timeout, Duration::from_millis(1500));
        assert_eq!(config.wayland.titlebar_actions, TitlebarActions::of(&section));
    }

    #[test]
    fn a_zero_timeout_disables_the_pings() {
        let section = WindowManagement {
            check_alive_timeout: Some(0),
            ..WindowManagement::default()
        };

        let mut config = SessionConfig::default();
        apply(&mut config, &section);

        assert_eq!(config.wayland.check_alive_timeout, Duration::ZERO);
    }

    #[test]
    fn every_focus_mode_maps_into_the_compositor_configuration() {
        for (configured, expected) in [
            (ConfiguredFocusMode::Click, FocusMode::Click),
            (ConfiguredFocusMode::Hover, FocusMode::Hover),
            (ConfiguredFocusMode::HoverStrict, FocusMode::HoverStrict),
        ] {
            let section = WindowManagement {
                focus_mode: Some(configured),
                ..WindowManagement::default()
            };

            let mut config = SessionConfig::default();
            apply(&mut config, &section);

            assert_eq!(config.wayland.wm.focus_mode, expected);
        }
    }

    #[test]
    fn an_omitted_focus_leaf_keeps_the_wm_default() {
        let mut config = SessionConfig::default();
        apply(&mut config, &WindowManagement::default());

        assert_eq!(config.wayland.wm.focus_mode, WmOptions::default().focus_mode);
        assert!(!config.wayland.wm.focus_change_on_pointer_rest);
    }

    #[test]
    fn focus_change_on_pointer_rest_maps_into_the_compositor_configuration() {
        let section = WindowManagement {
            focus_change_on_pointer_rest: Some(true),
            ..WindowManagement::default()
        };

        let mut config = SessionConfig::default();
        apply(&mut config, &section);

        assert!(config.wayland.wm.focus_change_on_pointer_rest);
    }

    #[test]
    fn a_bad_focus_mode_is_refused_naming_the_leaf() {
        let document = Value::table([(
            "window-management",
            Value::table([("focus-mode", Value::String("sideways".to_owned()))]),
        )]);

        let report = Config::from_value(document).expect_err("an unknown focus-mode must be refused");
        let paths: Vec<String> = report
            .problems()
            .iter()
            .map(|problem| problem.path.to_string())
            .collect();

        assert!(
            paths.iter().any(|path| path == "window-management.focus-mode"),
            "the refusal names the leaf, got {paths:?}"
        );
    }

    #[test]
    fn the_raise_leaves_keep_their_defaults_when_omitted() {
        let mut config = SessionConfig::default();
        apply(&mut config, &WindowManagement::default());

        assert!(config.wayland.wm.raise_on_click);
        assert!(!config.wayland.wm.auto_raise);
        assert_eq!(config.wayland.wm.auto_raise_delay, 500);
    }

    #[test]
    fn every_raise_leaf_value_maps_into_the_compositor_configuration() {
        let section = WindowManagement {
            raise_on_click: Some(false),
            auto_raise: Some(true),
            auto_raise_delay: Some(0),
            ..WindowManagement::default()
        };
        let mut config = SessionConfig::default();
        apply(&mut config, &section);
        assert!(!config.wayland.wm.raise_on_click);
        assert!(config.wayland.wm.auto_raise);
        assert_eq!(config.wayland.wm.auto_raise_delay, 0);

        let section = WindowManagement {
            auto_raise_delay: Some(10000),
            ..WindowManagement::default()
        };
        apply(&mut config, &section);
        assert_eq!(config.wayland.wm.auto_raise_delay, 10000);
    }

    #[test]
    fn a_bad_auto_raise_delay_is_refused_naming_the_leaf() {
        let document = Value::table([(
            "window-management",
            Value::table([("auto-raise-delay", Value::Integer(10001))]),
        )]);

        let report = Config::from_value(document).expect_err("an out-of-range auto-raise-delay must be refused");
        let paths: Vec<String> = report
            .problems()
            .iter()
            .map(|problem| problem.path.to_string())
            .collect();

        assert!(
            paths.iter().any(|path| path == "window-management.auto-raise-delay"),
            "the refusal names the leaf, got {paths:?}"
        );
    }

    #[test]
    fn the_options_a_reload_derives_are_the_options_startup_sets() {
        let section = WindowManagement {
            edge_tiling: Some(true),
            focus_mode: Some(ConfiguredFocusMode::HoverStrict),
            auto_raise: Some(true),
            auto_raise_delay: Some(250),
            ..WindowManagement::default()
        };
        let mut startup = SessionConfig::default();
        apply(&mut startup, &section);

        assert_eq!(wm_options_of(&section), startup.wayland.wm);
        assert_eq!(wm_options_of(&WindowManagement::default()), WmOptions::default());
    }
}

#[cfg(test)]
mod workspace_name_tests {
    use gnoblin_config::Workspace;

    use super::workspace_names;

    fn declared(id: Option<&str>, name: Option<&str>) -> Workspace {
        Workspace {
            id: id.map(str::to_owned),
            name: name.map(str::to_owned),
        }
    }

    #[test]
    fn each_declared_workspace_is_an_id_and_name_item_in_declaration_order() {
        let list = [declared(Some("code"), Some("Code")), declared(Some("web"), None)];
        assert_eq!(workspace_names(&list), ["code:Code", "web:web"]);
    }

    #[test]
    fn a_workspace_with_no_id_takes_its_position() {
        let list = [declared(Some("a"), Some("A")), declared(None, Some("Second"))];
        assert_eq!(workspace_names(&list), ["a:A", "2:Second"]);
    }

    #[test]
    fn no_declared_workspaces_is_an_empty_list() {
        assert!(workspace_names(&[]).is_empty());
    }
}
