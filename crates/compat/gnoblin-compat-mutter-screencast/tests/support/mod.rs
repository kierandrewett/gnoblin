//! Test support: the private bus and the fake IPC host.
//!
//! The fake host is the compositor's half of the [`CaptureBridge`] seam. It records
//! what the adapter asked it to open and close, which is the only thing the adapter
//! can do wrong on that side; the compositor's capture table is `gnoblin-capture`'s
//! and is tested there.

#![allow(dead_code)]

pub mod bus;

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use futures_lite::future;
use gnoblin_compat_mutter_screencast::{BridgeError, CaptureBridge, CaptureHandle};

/// What the fake host saw, shared with the test.
#[derive(Debug, Default)]
pub struct Host {
    /// `(owner, remote_session)` for every open.
    pub opened: Mutex<Vec<(String, String)>>,
    /// Every handle that was closed.
    pub closed: Mutex<Vec<u64>>,
    /// Whether an open is refused.
    pub refuse: AtomicBool,
    next: AtomicU64,
}

impl Host {
    /// The number of sessions the adapter opened.
    pub fn opened(&self) -> usize {
        self.opened.lock().expect("the host lock is not poisoned").len()
    }

    /// The number of sessions the adapter closed.
    pub fn closed(&self) -> usize {
        self.closed.lock().expect("the host lock is not poisoned").len()
    }
}

/// The host as the service holds it.
#[derive(Clone, Debug)]
pub struct SharedHost(pub Arc<Host>);

impl CaptureBridge for SharedHost {
    fn open(&self, owner: &str, remote_session: &str) -> Result<CaptureHandle, BridgeError> {
        if self.0.refuse.load(Ordering::SeqCst) {
            return Err(BridgeError::new("the fake compositor refused the capture session"));
        }
        self.0
            .opened
            .lock()
            .expect("the host lock is not poisoned")
            .push((owner.to_owned(), remote_session.to_owned()));
        let id = self.0.next.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(CaptureHandle::new(id))
    }

    fn close(&self, handle: CaptureHandle) -> Result<(), BridgeError> {
        self.0
            .closed
            .lock()
            .expect("the host lock is not poisoned")
            .push(handle.get());
        Ok(())
    }
}

/// Yield until `condition` holds, or fail the test rather than hang.
pub async fn until(mut condition: impl FnMut() -> bool) {
    for _ in 0..20_000 {
        if condition() {
            return;
        }
        future::yield_now().await;
    }
    panic!("the condition never became true");
}
