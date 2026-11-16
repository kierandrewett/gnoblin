//! The stock providers: the desktop policy the compositor registers when the `std-providers` feature is on.
//!
//! Core holds mechanism and a minimal safe default (`design/modular-framework.md`, 9.1). What the desktop does by
//! convention is a provider, and this module is where the compositor registers them until the C ABI carries providers
//! (M2.11), when `gnoblin-std-impl` takes over and this module is deleted:
//!
//! - the window manager that places and focuses windows while no shell holds the tree is `gnoblin-std-wm`'s
//!   (`Fallback`), registered by [`window_manager`]; the `window-management` section that configures it is [`wm`];
//! - the visual accessibility ([`a11y`]) and the visual bell ([`bell`]) turn their configuration sections into the
//!   composer's `a11y.zoom`, `a11y.filter` and `bell.flash` settings, which the frame and the bell mechanisms read;
//! - the portal dialogs ([`portal_ui`]) are the built-in [`crate::portal::UiProvider`], with the `dialogs.style`
//!   section that styles every built-in dialog.
//!
//! With the feature off none of this is compiled: the session has no window-manager provider (`NullWm`), reads none of
//! those sections, draws no portal dialog (a request that needs one is refused `no-ui`), and the settings stay at their
//! defaults. Everything that uses this module names it through `#[cfg(feature = "std-providers")]`, so the one place a
//! core module reaches std is a feature-gated line.

pub mod a11y;
pub mod bell;
pub(crate) mod portal_ui;
pub mod wm;

use gnoblin_wayland::{ControlHandle, Setting, TitlebarActions, WindowManagerFactory};

use crate::portal::UiProvider;

/// The window-manager provider a session starts with when its configuration names none: the stock one.
pub(crate) fn window_manager() -> WindowManagerFactory {
    gnoblin_std_wm::factory()
}

/// The portal UI provider a session starts with: the built-in dialogs, drawn into a frame of `width` by `height` pixels.
pub(crate) fn portal_ui(width: usize, height: usize) -> Box<dyn UiProvider> {
    Box::new(portal_ui::PortalUi::new(width, height))
}

/// Puts `settings` into `list`, replacing a setting of the same key already there, so a document read twice (from the
/// store, then from the environment) leaves one value per key.
pub(crate) fn set_settings(list: &mut Vec<Setting>, settings: impl IntoIterator<Item = Setting>) {
    for setting in settings {
        list.retain(|held| std::mem::discriminant(held) != std::mem::discriminant(&setting));
        list.push(setting);
    }
}

/// Applies the sections the stock providers read from an accepted document to a running session: the window
/// management policy and titlebar gestures, the magnifier and colour filter, the visual bell. A section the document
/// leaves out puts its settings back to the default, so removing it turns the feature off on this commit.
pub(crate) fn commit(control: &ControlHandle, config: &gnoblin_config::Config) {
    // The window management policy and titlebar gestures follow the accepted document, so a reload applies them
    // without a restart.
    let section = config.window_management.clone().unwrap_or_default();
    control.set_window_management(
        wm::wm_options_of(&section),
        TitlebarActions::of(&section),
        wm::check_alive_timeout_of(&section),
    );
    // The magnifier and colour filter are re-derived the same way, so removing the `accessibility` section turns
    // them off on this commit.
    for setting in a11y::visual_accessibility_settings(&config.accessibility.clone().unwrap_or_default()) {
        control.setting_set(setting);
    }
    // The visual bell is re-derived the same way, so removing the `compositor` section turns the flash off.
    control.setting_set(bell::visual_bell_setting(
        &config.compositor.clone().unwrap_or_default(),
    ));
}
