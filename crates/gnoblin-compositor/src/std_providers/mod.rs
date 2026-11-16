//! The stock providers: the desktop policy the compositor registers when the `std-providers` feature is on.
//!
//! Core holds mechanism and a minimal safe default (`design/modular-framework.md`, 9.1). What the desktop does by
//! convention is a provider. The window manager that places and focuses windows while no shell holds the tree is
//! `gnoblin-std-wm`'s, and this module registers it. With the feature off none of this is compiled and the session runs
//! `NullWm`. Until the C ABI carries providers (M2.11) this module is the whole of std's registration.

use gnoblin_wayland::WindowManagerFactory;

/// The window-manager provider a session starts with when its configuration names none: the stock one.
pub(crate) fn window_manager() -> WindowManagerFactory {
    gnoblin_std_wm::factory()
}
