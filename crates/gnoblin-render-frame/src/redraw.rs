//! A renderer's request for another frame.
//!
//! A renderer that finishes work after `draw` returned (the Vulkan renderer builds a shell's shader pipeline on a worker
//! thread) cannot show the result until the output is drawn again, and the compositor only draws what changed. The renderer
//! says so on a [`RedrawSignal`]: the worker calls [`RedrawSignal::request`], and the backend asks the target
//! ([`take`](RedrawSignal::take) through `Target::take_redraw`) before it renders an output, and damages the output when it
//! was asked. A backend that wants its event loop woken too installs a waker with [`RedrawSignal::set_waker`]; the waker runs
//! on the renderer's thread, so it must only wake the loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// What wakes a loop: it is called from the renderer's worker thread.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct RedrawState {
    wanted: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

/// A shared flag a renderer sets when it wants the output drawn again. Clones share it.
#[derive(Clone, Default)]
pub struct RedrawSignal(Arc<RedrawState>);

impl std::fmt::Debug for RedrawSignal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RedrawSignal")
            .field("wanted", &self.0.wanted.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

impl RedrawSignal {
    /// A signal nobody has asked on.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks for another frame, and runs the waker if there is one.
    pub fn request(&self) {
        self.0.wanted.store(true, Ordering::Release);
        let waker = self.0.waker.lock().unwrap_or_else(PoisonError::into_inner).clone();
        if let Some(waker) = waker {
            waker();
        }
    }

    /// Whether a frame was asked for since the last call, which clears the request.
    #[must_use]
    pub fn take(&self) -> bool {
        self.0.wanted.swap(false, Ordering::AcqRel)
    }

    /// Installs what [`request`](Self::request) calls to wake the loop, replacing any earlier one.
    pub fn set_waker(&self, waker: Waker) {
        *self.0.waker.lock().unwrap_or_else(PoisonError::into_inner) = Some(waker);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_taken_once_and_wakes_the_installed_waker() {
        let signal = RedrawSignal::new();
        assert!(!signal.take(), "nobody asked");
        let woken = Arc::new(AtomicBool::new(false));
        let seen = Arc::clone(&woken);
        signal.set_waker(Arc::new(move || seen.store(true, Ordering::Release)));
        let clone = signal.clone();
        clone.request();
        assert!(woken.load(Ordering::Acquire), "the waker ran");
        assert!(signal.take(), "a clone's request is the signal's");
        assert!(!signal.take(), "and a take clears it");
    }
}
