//! The captures a frame loop holds: one per View the plan draws, kept in step with the plan.
//!
//! Capture is best effort. Nothing here blocks the frame loop beyond the wait for a new
//! session's first `done`, and a capture that faults is dropped and reopened after
//! [`RETRY_AFTER`], so a session the compositor keeps stopping does not churn every frame.
//!
//! # Teardown order
//!
//! A [`Capture`](crate::wayland::Capture) is a handle linked into its connection: its `Drop` sends the protocol's
//! destructors for the session, the buffer and the source through proxies that belong to that connection. That is the
//! shape of the bug fixed in `712a904a7` (a PipeWire listener is a hook linked into its proxy's list, and removing it
//! after the proxy was freed wrote into freed memory). The rule here is the same as the fix's: a handle is dropped before
//! the object it is linked into. [`ViewCaptures`] declares `set` before `client`, so the captures go first on every path
//! (the value dropped, a capture removed, the connection lost), and the tests at the end of this file pin each path with a
//! connection that records whether it was still alive when each capture went.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use crate::set::CaptureSet;
use crate::wayland::{Capture, CaptureError, Client, Frame};

/// How long a node whose capture failed is left alone before it may be opened again.
pub const RETRY_AFTER: Duration = Duration::from_secs(1);

/// A failure: the View node it concerns, or `None` when the connection itself failed.
pub type Fault = (Option<u64>, CaptureError);

/// What the bridge needs from a set of captures: advance them, keep them in step with the planned Views, and read
/// the latest frame of one. [`ViewCaptures`] is the Wayland implementation, and tests use a scripted one.
pub trait FrameSource {
    /// Advances every capture without waiting. Returns what failed.
    fn pump(&mut self) -> Vec<Fault>;
    /// Opens a capture for each node in `wanted` that lacks one and drops the rest. Returns what failed to open.
    fn sync(&mut self, wanted: &BTreeSet<u64>) -> Vec<Fault>;
    /// The most recent frame of the View `node`, if one has been read.
    fn latest(&self, node: u64) -> Option<&Frame>;
}

/// The connection captures are made on, and what a capture needs from it while it lives. [`Client`] is the Wayland one;
/// the tests use one that records when it is dropped.
pub trait Connection {
    /// One capture of one View. Dropping it ends the session on the connection, so the connection must outlive it.
    type Capture;

    /// Opens a capture of the View `node`.
    fn open(&mut self, node: u64) -> Result<Self::Capture, CaptureError>;
    /// Reads what has arrived on the connection without waiting.
    fn dispatch(&mut self) -> Result<(), CaptureError>;
    /// Advances one capture after the connection was dispatched.
    fn advance(&self, capture: &mut Self::Capture) -> Result<(), CaptureError>;
    /// The most recent frame of a capture, if one has been read.
    fn latest(capture: &Self::Capture) -> Option<&Frame>;
}

impl Connection for Client {
    type Capture = Capture;

    fn open(&mut self, node: u64) -> Result<Capture, CaptureError> {
        Client::open(self, node)
    }

    fn dispatch(&mut self) -> Result<(), CaptureError> {
        Client::dispatch(self, Duration::ZERO)
    }

    fn advance(&self, capture: &mut Capture) -> Result<(), CaptureError> {
        capture.pump(self)
    }

    fn latest(capture: &Capture) -> Option<&Frame> {
        capture.latest()
    }
}

impl<K: Connection> FrameSource for ViewCaptures<K> {
    fn pump(&mut self) -> Vec<Fault> {
        Self::pump(self)
    }

    fn sync(&mut self, wanted: &BTreeSet<u64>) -> Vec<Fault> {
        Self::sync(self, wanted)
    }

    fn latest(&self, node: u64) -> Option<&Frame> {
        Self::latest(self, node)
    }
}

/// The live captures. With no client (no socket in the environment, or the connection failed)
/// it holds nothing and every call is a no-op.
///
/// The field order is the teardown order: the captures are dropped before the connection they are linked into.
pub struct ViewCaptures<K: Connection = Client> {
    set: CaptureSet<K::Capture>,
    retry_at: BTreeMap<u64, Instant>,
    client: Option<K>,
}

impl<K: Connection> ViewCaptures<K> {
    pub fn new(client: Option<K>) -> Self {
        Self {
            set: CaptureSet::new(),
            retry_at: BTreeMap::new(),
            client,
        }
    }

    /// Whether a client is connected, so captures may be opened.
    pub fn enabled(&self) -> bool {
        self.client.is_some()
    }

    /// The most recent frame of the View `node`, if one has been read.
    pub fn latest(&self, node: u64) -> Option<&Frame> {
        self.set.get(node).and_then(K::latest)
    }

    /// Opens a capture for each node in `wanted` that lacks one, and drops the captures of
    /// nodes no longer wanted. Returns the nodes that failed to open.
    pub fn sync(&mut self, wanted: &BTreeSet<u64>) -> Vec<Fault> {
        let Some(client) = self.client.as_mut() else {
            return Vec::new();
        };
        let now = Instant::now();
        self.retry_at.retain(|_, at| *at > now);
        let retry_at = &self.retry_at;
        let wanted: BTreeSet<u64> = wanted
            .iter()
            .copied()
            .filter(|node| !retry_at.contains_key(node))
            .collect();
        let failures = self.set.sync(&wanted, |node| client.open(node));
        let mut faults = Vec::with_capacity(failures.len());
        for (node, error) in failures {
            self.retry_at.insert(node, now + RETRY_AFTER);
            faults.push((Some(node), error));
        }
        faults
    }

    /// Dispatches the connection without waiting, then advances every capture. A capture that
    /// faults is dropped and cooled down. A connection failure disables capture for good.
    pub fn pump(&mut self) -> Vec<Fault> {
        let dispatched = match self.client.as_mut() {
            Some(client) => client.dispatch(),
            None => return Vec::new(),
        };
        if let Err(error) = dispatched {
            // The captures go first: each one's drop talks to the connection that is about to be dropped.
            self.set = CaptureSet::new();
            self.client = None;
            self.retry_at.clear();
            return vec![(None, error)];
        }
        let Some(client) = self.client.as_ref() else {
            return Vec::new();
        };
        let mut failed = Vec::new();
        for (node, capture) in self.set.iter_mut() {
            if let Err(error) = client.advance(capture) {
                failed.push((node, error));
            }
        }
        let now = Instant::now();
        let mut faults = Vec::with_capacity(failed.len());
        for (node, error) in failed {
            self.set.remove(node);
            self.retry_at.insert(node, now + RETRY_AFTER);
            faults.push((Some(node), error));
        }
        faults
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;

    /// What the fake connection and its captures saw, shared between them and the test.
    #[derive(Default)]
    struct Log {
        /// Whether the connection was still alive when each capture was dropped, in drop order, by node.
        captures_dropped: RefCell<Vec<(u64, bool)>>,
        /// Whether the connection has been dropped.
        connection_gone: Cell<bool>,
        /// The connection fails its next dispatch.
        fail_dispatch: Cell<bool>,
        /// A node whose capture fails to advance.
        fail_advance: Cell<Option<u64>>,
    }

    struct FakeConnection(Rc<Log>);

    struct FakeCapture {
        node: u64,
        log: Rc<Log>,
    }

    impl Drop for FakeCapture {
        fn drop(&mut self) {
            // Alive means the connection has not been dropped yet: a capture dropped after it would send its
            // destructors into a connection that is gone.
            let alive = !self.log.connection_gone.get();
            self.log.captures_dropped.borrow_mut().push((self.node, alive));
        }
    }

    impl Drop for FakeConnection {
        fn drop(&mut self) {
            self.0.connection_gone.set(true);
        }
    }

    impl Connection for FakeConnection {
        type Capture = FakeCapture;

        fn open(&mut self, node: u64) -> Result<FakeCapture, CaptureError> {
            Ok(FakeCapture {
                node,
                log: Rc::clone(&self.0),
            })
        }

        fn dispatch(&mut self) -> Result<(), CaptureError> {
            if self.0.fail_dispatch.get() {
                Err(CaptureError::SessionNotReady)
            } else {
                Ok(())
            }
        }

        fn advance(&self, capture: &mut FakeCapture) -> Result<(), CaptureError> {
            if self.0.fail_advance.get() == Some(capture.node) {
                Err(CaptureError::SessionNotReady)
            } else {
                Ok(())
            }
        }

        fn latest(_: &FakeCapture) -> Option<&Frame> {
            None
        }
    }

    fn captures(nodes: &[u64]) -> (ViewCaptures<FakeConnection>, Rc<Log>) {
        let log = Rc::new(Log::default());
        let mut captures = ViewCaptures::new(Some(FakeConnection(Rc::clone(&log))));
        assert!(captures.sync(&nodes.iter().copied().collect()).is_empty());
        (captures, log)
    }

    fn all_before_the_connection(log: &Log, nodes: &[u64]) {
        let dropped = log.captures_dropped.borrow();
        let mut seen: Vec<u64> = dropped.iter().map(|(node, _)| *node).collect();
        seen.sort_unstable();
        assert_eq!(seen, nodes, "every capture was dropped exactly once");
        assert!(
            dropped.iter().all(|(_, alive)| *alive),
            "a capture was dropped after its connection: {dropped:?}"
        );
    }

    #[test]
    fn dropping_the_captures_drops_every_capture_before_the_connection() {
        let (captures, log) = captures(&[2, 4, 6]);
        drop(captures);
        assert!(log.connection_gone.get());
        all_before_the_connection(&log, &[2, 4, 6]);
    }

    #[test]
    fn a_capture_no_longer_wanted_is_dropped_while_the_connection_is_alive() {
        let (mut captures, log) = captures(&[2, 4]);
        assert!(captures.sync(&BTreeSet::from([4])).is_empty());
        assert_eq!(*log.captures_dropped.borrow(), vec![(2, true)]);
        assert!(!log.connection_gone.get());
        drop(captures);
        all_before_the_connection(&log, &[2, 4]);
    }

    #[test]
    fn a_lost_connection_drops_the_captures_before_it_drops_the_connection() {
        let (mut captures, log) = captures(&[2, 4]);
        log.fail_dispatch.set(true);
        let faults = captures.pump();
        assert_eq!(faults.len(), 1);
        assert!(faults[0].0.is_none(), "the failure is the connection's, not a View's");
        assert!(log.connection_gone.get(), "the connection was discarded");
        all_before_the_connection(&log, &[2, 4]);
        assert!(!captures.enabled());
        assert!(captures.pump().is_empty(), "a lost connection stays lost");
    }

    #[test]
    fn a_capture_that_faults_is_dropped_while_the_connection_is_alive_and_left_alone_for_a_while() {
        let (mut captures, log) = captures(&[2, 4]);
        log.fail_advance.set(Some(4));
        let faults = captures.pump();
        assert_eq!(faults.iter().map(|(node, _)| *node).collect::<Vec<_>>(), vec![Some(4)]);
        assert_eq!(*log.captures_dropped.borrow(), vec![(4, true)]);
        // It is not opened again at once, and not before the cool-down is over.
        log.fail_advance.set(None);
        assert!(captures.sync(&BTreeSet::from([2, 4])).is_empty());
        assert_eq!(captures.set.len(), 1, "node 4 is cooling down");
        drop(captures);
        all_before_the_connection(&log, &[2, 4]);
    }

    #[test]
    fn with_no_connection_nothing_is_captured_and_nothing_fails() {
        let mut captures: ViewCaptures<FakeConnection> = ViewCaptures::new(None);
        assert!(!captures.enabled());
        assert!(captures.sync(&BTreeSet::from([2])).is_empty());
        assert!(captures.pump().is_empty());
        assert!(captures.latest(2).is_none());
    }
}
