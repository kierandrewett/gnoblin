//! `org.gnome.Mutter.ScreenCast` for `xdg-desktop-portal-gnome` and screen recorders.
//!
//! This is `gnoblin-compat-dbus-mutter-screencast` of `design/rust-compositor.md`
//! ("Compatibility crates", the redirect map's `ScreenCast` row and D13): the
//! adapter that lets a caller create a capture session, record one monitor, window
//! or area, and learn why a stream cannot be started.
//!
//! # What it is
//!
//! An IPC client with a D-Bus face. [`service`] serves the upstream interface, and
//! the capture session it starts is announced to the compositor through
//! [`CaptureBridge`], which the services role implements over its privileged IPC
//! connection. The upstream interface is vendored beside this crate
//! (`interfaces/org.gnome.Mutter.ScreenCast.xml`, from Mutter 51.0, the release
//! `data/gnome-versions.json` pins), so the signature a caller sees is the one this
//! adapter was written against rather than a paraphrase of it.
//!
//! # What it is not
//!
//! - **Not a frame producer.** PipeWire streams are not implemented: `gnoblin-capture`
//!   states why (the `pipewire-sys` build fails with the pinned toolchain), and this
//!   adapter does not fake a node id for them. `Stream.Start` answers
//!   `unsupported-pipewire` and names that blocker, exactly as the support matrix
//!   declares.
//! - **Not the consent.** D13's default `portal` mode makes every capture member
//!   answer `capture-portal-only`; `unmediated` is the user's explicit opt-in for
//!   tools such as `gnome-remote-desktop`, and the documentation states the cost.
//! - **Not a session store.** Session and stream handles are wire state, as the plan
//!   requires: they map a caller's object paths to the capture session the compositor
//!   owns, and they end when the caller's bus name vanishes or the lock goes up.
//!
//! # The missing compositor half
//!
//! The schema slice declares the `capture` object (`capture.open`, `capture.close`) so
//! the bridge is an ordinary IPC call rather than a private channel. No compositor
//! handler creates that object yet: `gnoblin-compositor`'s IPC server serves only
//! `window.focus` today. Until the capture lane adds the handler, a real services role
//! reports `capture.open` as refused to a caller that reaches `Session.Start` in
//! `unmediated` mode; the adapter itself is complete and is proved against a fake IPC
//! host, which is what [`CaptureBridge`] is.
//!
//! # Failure modes first
//!
//! Each entry names the test that covers it, in `tests/`.
//!
//! 1. **An adapter call skips the lock embargo.** Every member admits through
//!    [`compat::ScreenCastAdmission`], and capture is refused while the session is
//!    locking or locked: `capture_is_refused_while_locked`.
//! 2. **Capture is allowed in the default mode.** `CreateSession`, every `Record*`, and
//!    the session and stream starts answer `capture-portal-only` in `portal`:
//!    `the_default_mode_refuses_every_capture_call`.
//! 3. **A stream outlives its caller.** The caller's bus name vanishing ends its
//!    sessions and streams, and the bridge is told: `a_session_ends_with_its_caller`.
//! 4. **A session outlives the lock.** A session started before the lock ends at the
//!    lock, rather than resuming when the lock is released:
//!    `locking_ends_every_session`.
//! 5. **An unsupported method fails silently.** `RecordVirtual`, `Stream.Start` and the
//!    PipeWire signal are refused with the matrix's own reason:
//!    `an_unsupported_member_is_refused_by_name`.
//! 6. **A caller reads or stops another caller's session.** Every session and stream
//!    member checks the sender against the recorded owner: `a_foreign_caller_cannot_use_a_session`.
//! 7. **The matrix and the served interface disagree.** Every declared member is called
//!    and behaves as the declaration says: `the_matrix_matches_what_the_adapter_serves`.

#![forbid(unsafe_code)]

/// The compositor seam: one capture session, opened and closed through the IPC.
pub mod bridge;
/// Caller identity as far as the session bus can tell.
pub mod caller;
/// The admission gate: the support matrix and the mode check for each call.
pub mod compat;
/// The `ScreenCast` error type, returned when a session cannot be served.
pub mod error;
/// The D-Bus service objects and the export that serves them.
pub mod service;

/// The compositor-side capture bridge, its error and its session handle.
pub use bridge::{BridgeError, CaptureBridge, CaptureHandle};
/// Admission types and the support matrix, re-exported from `compat`.
pub use compat::{MATRIX, NAME, ScreenCastAdmission, ScreenCastCall};
/// The error returned when a `ScreenCast` call cannot be served.
pub use error::ScreenCastError;
/// Service constants, the service object, the export function and the watch lookup.
pub use service::{
    BUS_NAME, INTERFACE, PATH, SESSION_INTERFACE, STREAM_INTERFACE, ScreenCastService, VERSION, export, watch_callers,
};

gnoblin_log::logger!(log = "compat-screencast");
