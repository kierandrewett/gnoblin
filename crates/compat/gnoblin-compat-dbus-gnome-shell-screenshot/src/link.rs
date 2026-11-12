//! The IPC client half: one tree attachment, for the one thing this adapter needs from it.
//!
//! The screenshot adapter serves no reads, because the IPC publishes no image capture: what it
//! needs from the compositor is the lock state, and only so the gate's embargo is the
//! compositor's rather than the adapter's own guess. That is why this module is a
//! [`gnoblin_client::Client`] attached to the tree's `lock` node and nothing else.
//!
//! # Failure modes first
//!
//! 1. **The tree has no lock node.** Refused at connect, because a released embargo
//!    inferred from an absent node is the one guess this adapter must not make:
//!    [`ScreenshotLink::connect`].
//! 2. **The link ends.** [`ScreenshotLink::advance`] reports `Ok(false)`, so a host stops instead
//!    of folding the same state forever.
//! 3. **A pending lock.** The embargo is the node's `covers`, true from the moment a lock is taken, so
//!    a capture never races the cover.

use gnoblin_client::{Client, ClientError, Event, Transport};
use gnoblin_compat_core::LockState;

use crate::error::ScreenshotError;

/// One attachment to the compositor's tree, and the lock state it has delivered.
#[derive(Debug)]
pub struct ScreenshotLink<C: Transport> {
    client: Client<C>,
    lock: LockState,
}

impl<C: Transport> ScreenshotLink<C> {
    /// Attach to the tree and read the lock node.
    ///
    /// # Errors
    ///
    /// [`ScreenshotError::Link`] when the attach fails and [`ScreenshotError::NoLock`]
    /// when the tree has no lock node.
    pub fn connect(client: Client<C>) -> Result<ScreenshotLink<C>, ScreenshotError> {
        let mut client = client;
        let state = match client.watch_lock() {
            Ok(state) => state,
            Err(ClientError::Refused { code, .. }) if code == "not-found" => return Err(ScreenshotError::NoLock),
            Err(error) => return Err(ScreenshotError::from(error)),
        };
        Ok(ScreenshotLink {
            client,
            lock: LockState::from_node(state.as_str()),
        })
    }

    /// The lock state as of the last folded change.
    pub const fn lock(&self) -> LockState {
        self.lock
    }

    /// Fold one event.
    ///
    /// `Ok(true)` means one event was folded and [`ScreenshotLink::lock`] may have changed;
    /// `Ok(false)` means the connection ended.
    ///
    /// # Errors
    ///
    /// [`ScreenshotError::Link`] for a transport failure.
    pub fn advance(&mut self) -> Result<bool, ScreenshotError> {
        match self.client.next_event() {
            Ok(Event::Lock(state)) => {
                self.lock = LockState::from_node(state.as_str());
                Ok(true)
            }
            Ok(_) => Ok(true),
            Err(ClientError::Disconnected) => Ok(false),
            Err(error) => Err(ScreenshotError::from(error)),
        }
    }

    /// Fold every event the link has already delivered.
    ///
    /// # Errors
    ///
    /// The same refusals as [`ScreenshotLink::advance`], except that an ended link is not one of
    /// them.
    pub fn drain(&mut self) -> Result<usize, ScreenshotError> {
        let mut folded = 0;
        while self.advance()? {
            folded += 1;
        }
        Ok(folded)
    }
}
