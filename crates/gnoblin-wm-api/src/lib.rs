//! The window-manager provider contract (`design/modular-framework.md`, 9.4).
//!
//! Core holds mechanism and no policy about where windows go. What decides that, while no shell holds the tree, is a
//! provider: something that implements [`WindowManager`]. The compositor keeps one provider per session and applies the
//! [`WmEffect`]s it returns. A core with no provider uses [`NullWm`], which places windows at the origin and has no focus
//! policy.
//!
//! The crate names only the shared primitives, so core and the crates that implement a provider can both depend on it
//! without either depending on the other. It holds:
//!
//! - the trait, [`WindowManager`];
//! - the records and vocabulary it speaks in: [`Window`], [`OutputRecord`], [`Open`], [`Standing`], [`WmEffect`] and
//!   [`WmRefusal`];
//! - the window management options a configuration document carries, [`WmOptions`] and its three choices;
//! - [`NullWm`], and the [`WindowManagerFactory`] a configuration names a provider with.

mod factory;
mod manager;
mod null;
mod policy;
mod window;

pub use factory::WindowManagerFactory;
pub use manager::WindowManager;
pub use null::NullWm;
pub use policy::{DragConstraint, FocusMode, NewWindowFocus, WmOptions};
pub use window::{
    DEFAULT_SIZE, MAX_WINDOWS, Open, OutputRecord, Standing, Window, WindowKind, WindowState, WmEffect, WmRefusal,
};
