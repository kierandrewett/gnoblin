//! `org.gnome.Mutter.Clipboard` for callers that move selections between apps.
//!
//! This is the admission half of the adapter, from `design/rust-compositor.md`
//! ("Compatibility crates", D13 and D14). Each upstream member is a [`ClipboardCall`];
//! every call passes the mode gate and the support matrix through
//! [`compat::ClipboardAdmission`] before any mapping, so a refusal names the rule that
//! fired.
//!
//! # What it is
//!
//! The declaration and the gate for the Mutter clipboard interface, vendored beside
//! this crate (`interfaces/org.gnome.Mutter.Clipboard.xml`). The interface is served by
//! the compositor's selection table, which this crate does not own.
//!
//! # What it is not
//!
//! - **Not a selection store.** Selection contents are the compositor's. This crate
//!   holds no clipboard data.
//! - **Not a D-Bus service yet.** The `zbus` face and its private-bus tests are not in
//!   this crate: the compositor half (`clipboard` IPC object) does not exist, so every
//!   member is declared `unsupported` and refuses by name. Nothing is faked.
//! - **Not consent.** Reading a selection is a capture-class operation (`SelectionRead`),
//!   refused in the default `portal` mode (D13).
//!
//! # Failure modes first
//!
//! 1. **A selection is read in the default mode.** `SelectionRead` is `Capture` and
//!    `portal` refuses it: `the_default_mode_refuses_a_selection_read`.
//! 2. **A selection is read while the session is locked.** The lock embargo refuses
//!    capture-class reads even in `unmediated` mode: `the_lock_refuses_a_selection_read`.
//! 3. **An unsupported member is served anyway.** A declared-`unsupported` member
//!    refuses with the matrix's own reason: `an_unsupported_member_is_refused_by_name`.
//! 4. **The matrix and the call enum drift.** Every call maps to a declared member:
//!    `every_call_has_a_declared_member`.

#![forbid(unsafe_code)]

pub mod compat;

pub use compat::{ClipboardAdmission, ClipboardCall, INTERFACE, MATRIX, NAME, VERSION};
