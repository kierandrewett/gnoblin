//! The stacking, placement, drag, focus and liveness policy, from the `window-management`
//! configuration section.
//!
//! A document with no `window-management` section keeps the built-in defaults. A section
//! with a field missing keeps that field's default. The values are checked by the schema
//! before they reach this module, so an unknown or out-of-range value is refused there and
//! never arrives here.

use std::path::Path;
use std::time::Duration;

use gnoblin_config::{FocusNewWindows, NewWindowPlacement, SearchPaths, Store, WindowManagement};
use gnoblin_wayland::TitlebarActions;
use gnoblin_wm::{DragConstraint, NewWindowFocus, PlacementMode, WmOptions};
use tracing::{debug, warn};

use crate::SessionConfig;

/// Sets the window-management fields of `config.wayland` from the last accepted
/// configuration in `store`, read from `source`.
pub fn configure_from_store(config: &mut SessionConfig, store: &Store, source: &Path) {
    match store.load(source) {
        Ok(stored) => {
            let section = stored.config.window_management.clone().unwrap_or_default();
            apply(config, &section);
            debug!("[window-management] loaded from the last accepted configuration");
        }
        Err(error) => warn!(%error, "[window-management] configuration not readable; the defaults are used"),
    }
}

/// [`configure_from_store`] for the process's own state directory and configuration path.
///
/// With no configuration file there is no accepted document, so the defaults are used.
pub fn configure_from_environment(config: &mut SessionConfig) {
    let Some(store) = Store::from_env() else {
        debug!("[window-management] no state directory is set; the defaults are used");
        return;
    };
    match gnoblin_config::locate(&SearchPaths::from_env()) {
        Ok(path) => configure_from_store(config, &store, path.path()),
        Err(error) => debug!(%error, "[window-management] no configuration file; the defaults are used"),
    }
}

/// Copies an accepted `window-management` section into the compositor configuration. A leaf
/// the section leaves out keeps its compositor default.
fn apply(config: &mut SessionConfig, section: &WindowManagement) {
    let wm_defaults = WmOptions::default();
    config.wayland.wm = WmOptions {
        placement: section
            .new_window_placement
            .map_or(wm_defaults.placement, placement_mode),
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
    };
    config.wayland.titlebar_actions = TitlebarActions::of(section);
    let default_timeout = gnoblin_wayland::Config::default().check_alive_timeout;
    config.wayland.check_alive_timeout = section
        .check_alive_timeout
        .and_then(|ms| u64::try_from(ms).ok())
        .map_or(default_timeout, Duration::from_millis);
}

/// The placement mode for a configured `new-window-placement`.
fn placement_mode(placement: NewWindowPlacement) -> PlacementMode {
    match placement {
        NewWindowPlacement::FreeSpace => PlacementMode::FreeSpace,
        NewWindowPlacement::Center => PlacementMode::Center,
        NewWindowPlacement::Cascade => PlacementMode::Cascade,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            new_window_placement: Some(NewWindowPlacement::Cascade),
            edge_tiling: Some(true),
            constrain_drag_to_work_area: Some(false),
            check_alive_timeout: Some(1500),
            focus_new_windows: Some(FocusNewWindows::Allow),
            ..WindowManagement::default()
        };

        let mut config = SessionConfig::default();
        apply(&mut config, &section);

        assert_eq!(config.wayland.wm.placement, PlacementMode::Cascade);
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
}
