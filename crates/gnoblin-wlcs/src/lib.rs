//! The wlcs integration for the Rust compositor.
//!
//! [wlcs](https://github.com/canonical/wlcs) is Canonical's Wayland Conformance Suite. It does not start a
//! compositor itself: it loads a shared library that exports `wlcs_server_integration`, and that library starts the
//! compositor under test, hands out client connections and injects input. This crate is that library for Gnoblin,
//! `libgnoblin_wlcs.so`, run as `wlcs libgnoblin_wlcs.so` (`scripts/wlcs.sh` builds and runs both).
//!
//! # What runs
//!
//! Each wlcs test gets a [`ConformanceSession`]: the compositor's own [`gnoblin_wayland::Server`] on a headless backend, with
//! its one virtual output and the software renderer, on a thread of its own and a socket in a private directory.
//! Clients are adopted over socket pairs, input goes through the session's own input path, and
//! `position_window_absolute` moves a window through the window manager's record, so what a test observes is the
//! behaviour a real client sees, not a double of it.
//!
//! # Layout
//!
//! - [`harness`] is safe Rust: the session, its clients and the fake devices.
//! - `ffi` is the C ABI over it and holds every `unsafe` block of the crate.
//!
//! wlcs is GPL-2.0 and is built and run by the script, never linked or shipped by this repository.

mod ffi;
pub mod harness;

pub use ffi::extensions;
pub use harness::{ConformanceSession, Injector};
