//! The compositor seam: one capture session, opened and closed through the IPC.
//!
//! The D-Bus object cannot hold the IPC client: a zbus interface must be `Send +
//! Sync`, and the client a host threads onto the IPC is a blocking `Client<C>` whose
//! transport need not be. This trait is the one place the two halves meet, following
//! the same seam `gnoblin-compat-mutter-displayconfig` uses for its write path.
//!
//! A host implements it over its privileged connection by calling the schema's
//! `capture.open` and `capture.close` members with these arguments:
//!
//! | call | `capture.open` | `capture.close` |
//! | --- | --- | --- |
//! | session | the identity the host allocated | the identity it opened with |
//! | owner | the D-Bus unique name of the caller | not needed |
//! | remote session | `SessionId` of the linked remote desktop session, or `""` | not needed |
//!
//! The handle is opaque to the adapter. Nothing here reads a frame: the PipeWire
//! producer is missing for the reason [`crate`] states, and a session that produces
//! none still has a lifetime the compositor must end.
//!
//! # Failure modes first
//!
//! 1. **A refused open leaves a session that was never opened.** `Session.Start`
//!    stores the handle only after [`CaptureBridge::open`] answered, so a bridge
//!    refusal leaves the session unstarted: `a_refused_open_leaves_the_session_unstarted`.
//! 2. **A close runs twice.** The session record takes its handle out under the lock
//!    before the bridge is told, so one session produces at most one close:
//!    `a_second_stop_does_not_close_twice`.

use std::fmt;

/// One capture session the compositor owns.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct CaptureHandle(u64);

impl CaptureHandle {
    /// A handle the host's IPC layer returned for a session it opened.
    pub const fn new(id: u64) -> CaptureHandle {
        CaptureHandle(id)
    }

    /// The identity the host allocated.
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for CaptureHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "capture session {}", self.0)
    }
}

/// Why the compositor did not open or close a session.
///
/// The sentence is the host's, because only it knows whether the IPC refused the call,
/// the connection ended, or the schema named no `capture` object. The adapter never
/// invents a cause.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BridgeError {
    reason: String,
}

impl BridgeError {
    /// A refusal with the host's own wording.
    pub fn new(reason: impl Into<String>) -> BridgeError {
        BridgeError { reason: reason.into() }
    }

    /// What the host said.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl fmt::Display for BridgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl std::error::Error for BridgeError {}

/// The one compositor-side effect this adapter performs.
///
/// `open` is called once per started session and `close` once per session that
/// started, so a host may treat the pair as a lease and end the compositor's session
/// when the connection goes away.
pub trait CaptureBridge: Send + Sync {
    /// Announce a session to the compositor and answer the handle its table holds.
    ///
    /// `owner` is the D-Bus caller's unique name and `remote_session` is the linked
    /// remote desktop session id, empty when the caller linked none.
    ///
    /// # Errors
    ///
    /// [`BridgeError`] when the IPC refused the call or could not reach the compositor.
    fn open(&self, owner: &str, remote_session: &str) -> Result<CaptureHandle, BridgeError>;

    /// End the session behind `handle`.
    ///
    /// # Errors
    ///
    /// [`BridgeError`] when the IPC refused the call. The adapter still forgets the
    /// session: a compositor that refused the close is a compositor that will end it
    /// when the connection does.
    fn close(&self, handle: CaptureHandle) -> Result<(), BridgeError>;
}
