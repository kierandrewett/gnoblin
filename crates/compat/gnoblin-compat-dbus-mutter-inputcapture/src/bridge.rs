//! The compositor seam: the one place a host meets the compositor's IPC for this interface.
//!
//! A host implements this over its privileged connection. The compositor has no IPC
//! object for the interface yet, so no implementation exists in this repository. This
//! trait is declared only so the seam is named and reviewed before the IPC lands.
//!
//! The handle is opaque to the adapter: the identity the host allocated for the session.

/// The compositor side of one InputCapture session.
pub trait InputCaptureBridge: Send + Sync {
    /// Open a session for `owner`, the D-Bus unique name of the caller, and return the
    /// identity the host allocated for it.
    ///
    /// # Errors
    ///
    /// The host's own sentence when the compositor refused or has no IPC for it.
    fn open(&self, owner: &str) -> Result<u64, String>;

    /// End the session the host opened with `handle`.
    ///
    /// # Errors
    ///
    /// The host's own sentence when the compositor refused the close.
    fn close(&self, handle: u64) -> Result<(), String>;
}
