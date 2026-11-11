//! The captures a frame loop holds: one per View the plan draws, kept in step with the plan.
//!
//! Capture is best effort. Nothing here blocks the frame loop beyond the wait for a new
//! session's first `done`, and a capture that faults is dropped and reopened after
//! [`RETRY_AFTER`], so a session the compositor keeps stopping does not churn every frame.

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

impl FrameSource for ViewCaptures {
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
pub struct ViewCaptures {
    client: Option<Client>,
    set: CaptureSet<Capture>,
    retry_at: BTreeMap<u64, Instant>,
}

impl ViewCaptures {
    pub fn new(client: Option<Client>) -> Self {
        Self {
            client,
            set: CaptureSet::new(),
            retry_at: BTreeMap::new(),
        }
    }

    /// Whether a client is connected, so captures may be opened.
    pub fn enabled(&self) -> bool {
        self.client.is_some()
    }

    /// The most recent frame of the View `node`, if one has been read.
    pub fn latest(&self, node: u64) -> Option<&Frame> {
        self.set.get(node).and_then(Capture::latest)
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
            Some(client) => client.dispatch(Duration::ZERO),
            None => return Vec::new(),
        };
        if let Err(error) = dispatched {
            self.client = None;
            self.set = CaptureSet::new();
            self.retry_at.clear();
            return vec![(None, error)];
        }
        let Some(client) = self.client.as_ref() else {
            return Vec::new();
        };
        let mut failed = Vec::new();
        for (node, capture) in self.set.iter_mut() {
            if let Err(error) = capture.pump(client) {
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
