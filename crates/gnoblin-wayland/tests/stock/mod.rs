//! The window-manager provider the test sessions run.
//!
//! This crate holds no stock provider: a session with none runs `NullWm` (`null_wm.rs` proves it). A suite that needs a desktop
//! to act on (a window that takes the keyboard focus when it maps, a click that focuses the window under it) starts its session
//! through [`config`], which names the stock window manager of `gnoblin-std-wm`, a development dependency, so the suites run
//! against the desktop users get, cascade, focus rotation and raise included.

#![allow(dead_code)]

use gnoblin_wayland::{Config, WindowManagerFactory};

/// The provider a test session starts with: the stock window manager.
pub fn window_manager() -> Option<WindowManagerFactory> {
    Some(gnoblin_std_wm::factory())
}

/// The default session configuration of the test sessions: [`Config::default`] with [`window_manager`].
pub fn config() -> Config {
    Config {
        window_manager: window_manager(),
        ..Config::default()
    }
}
