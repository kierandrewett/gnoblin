//! `org.gnome.Mutter.RemoteDesktop` for callers that drive the desktop's keyboard, pointer and touch.
//!
//! The admission half of the adapter, from `design/rust-compositor.md` ("Compatibility
//! crates", D13 and D14). Each upstream member is a [`compat::RemoteDesktopCall`]; every call
//! passes the mode gate and the support matrix through [`compat::RemoteDesktopAdmission`]
//! before any mapping, so a refusal names the rule that fired.
//!
//! # What it is
//!
//! The declaration and the gate for the Mutter interface, vendored beside this crate
//! (`interfaces/`). The bridge seam in [`bridge`] is the one place a host would meet
//! the compositor's IPC.
//!
//! # What it is not
//!
//! - **Not served.** The compositor has no IPC object for this interface, so every member
//!   is declared `unsupported` and refuses by name. Nothing is faked.
//! - **Not a D-Bus service yet.** The `zbus` face and its private-bus tests are not in
//!   this crate; they wait on the compositor half.
//! - **No production bridge.** [`bridge`] declares the seam only; no host implements it.
//!
//! # Failure modes first
//!
//! 1. **Input is injected in the default mode.** Injection is `InputInjection` and `portal`
//!    refuses it: `the_default_mode_refuses_input_injection`.
//! 2. **Input is injected while locked.** The lock embargo refuses it even in
//!    `unmediated`: `the_lock_refuses_input_injection`.
//! 3. **An unsupported member is served anyway.** A declared-`unsupported` member refuses
//!    with the matrix's reason: `an_unsupported_member_is_refused_by_name`.
//! 4. **The matrix and the call enum drift.** Every call maps to a declared member:
//!    `every_call_has_a_declared_member`.

#![forbid(unsafe_code)]

pub mod bridge;
pub mod compat;

pub use compat::{INTERFACE, MATRIX, NAME, RemoteDesktopAdmission, RemoteDesktopCall, SESSION_INTERFACE, VERSION};
