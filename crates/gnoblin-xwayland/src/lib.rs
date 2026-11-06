//! Xwayland startup and window management.
//!
//! This is the `gnoblin-xwayland` component of `design/rust-compositor.md` (D2: Smithay's
//! XWM in-tree). It owns three things and nothing else:
//!
//! - **The start parameters.** [`Settings`] is the `xwayland` configuration section
//!   turned into what a start needs, and [`probe`] is the answer to "is there a server to
//!   start" as a sentence rather than a panic.
//! - **The supervision policy.** [`Xwayland`] decides when to start, when to retry after a
//!   crash and when to give up. It is a state machine ticked by its caller, so the
//!   compositor keeps the process and the clock and this crate keeps the policy.
//! - **The window table.** [`Xwm`] is the X11 window management a floating desktop needs:
//!   it turns X11 windows into the [`gnoblin_wm::Event`]s the window manager already
//!   consumes, and answers with the X11 requests the compositor has to send.
//!
//! The XSETTINGS seam is [`XSettings`], an extension trait `gnoblin-compat-x11-xsettings`
//! implements. Nothing here implements it for real; [`Recording`] is the test double that
//! proves the seam.
//!
//! # What is not here
//!
//! The process, the Wayland socket, the `xwayland-shell-v1` global and Smithay's `X11Wm`
//! live in `gnoblin-wayland`, because they need the Wayland display and the event loop
//! this crate must not know about. The two crates meet through
//! `gnoblin_backend::x11`, whose types are the vocabulary they share. Everything in this
//! crate is therefore free of Smithay, `wayland-*` and the compose loop, and its tests
//! run without a session except the ones in `tests/` that start the real server.
//!
//! # Failure modes first
//!
//! The lists at the top of [`settings`], [`supervisor`], [`windows`] and [`xsettings`]
//! name the failure and the test for it. The ones that need the real server are in
//! `tests/real_xwayland.rs`, and they are skipped with a printed reason only when the
//! machine has no `Xwayland` binary at all.

pub mod settings;
pub mod supervisor;
pub mod windows;
pub mod xsettings;

pub use settings::{Capabilities, Settings, SettingsError, probe};
pub use supervisor::{Handle, Status, Xwayland};
pub use windows::{Xwm, window_id};
pub use xsettings::{RecordingOwner, Setting, XSettings};
