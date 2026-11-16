//! The stock window manager (`design/modular-framework.md`, 9.3 and 9.4).
//!
//! [`Fallback`] is the provider a desktop runs while no shell holds the tree: it cascades new windows, focuses on map and
//! on click, rotates focus when the focused window goes, and fits maximised and fullscreen windows to the output. It
//! implements `gnoblin_wm_api::WindowManager` and names nothing of the compositor, so it is std: a core with no provider
//! runs `gnoblin_wm_api::NullWm` instead, and a shell that holds the tree decides in place of either.
//!
//! The compositor links this crate only behind its `std-providers` cargo feature (`gnoblin-compositor/src/std_providers`).

mod cascade;
mod fallback;

pub use fallback::Fallback;

use gnoblin_wm_api::WindowManagerFactory;

/// The factory that names the stock provider: what a configuration sets as its window manager to run the stock desktop.
#[must_use]
pub fn factory() -> WindowManagerFactory {
    WindowManagerFactory::new(|| Box::new(Fallback::default()))
}
