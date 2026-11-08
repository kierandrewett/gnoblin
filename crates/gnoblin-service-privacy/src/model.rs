//! The pure model behind the privacy service: which capture nodes exist, which of them are
//! running, and whether the PipeWire connection is currently trustworthy.
//!
//! Nothing here touches PipeWire. The `pipewire` module translates registry, node and core
//! events into calls on [`Tracker`], and [`Tracker::take_change`] says when the published
//! `(available, microphone, camera)` triple must be re-emitted. Keeping this separate is what
//! lets every failure mode below be a unit test rather than a live PipeWire session.
//!
//! # Failure modes first
//!
//! 1. **Nothing is known until the core syncs.** A node may be running before the initial
//!    sync completes, but `available` stays false, so no activity is reported:
//!    [`tests::nothing_is_reported_before_the_core_syncs`].
//! 2. **A node stops running** while still bound: the activity clears:
//!    [`tests::a_node_that_stops_running_clears_its_activity`].
//! 3. **A node is removed** while running: the activity clears:
//!    [`tests::removing_a_running_node_clears_its_activity`].
//! 4. **One node's proxy errors**: only that node is dropped; other capture nodes keep
//!    reporting: [`tests::a_node_error_drops_only_that_node`].
//! 5. **An error on an unknown proxy** changes nothing: [`tests::an_error_on_an_unknown_proxy_is_ignored`].
//! 6. **The core or registry errors**: the connection is lost, every node is forgotten and
//!    availability drops; a late sync from the dead connection cannot restore it:
//!    [`tests::losing_the_connection_forgets_nodes_and_ignores_a_stale_sync`].
//! 7. **Reconnecting** starts from an empty tree and accepts the new connection's sync:
//!    [`tests::a_reset_connection_accepts_a_fresh_sync`].
//! 8. **Repeated identical state** is not re-emitted, and the initial all-false state is not
//!    emitted at all: [`tests::take_change_emits_only_real_changes`].

use std::collections::BTreeMap;

/// The triple the service publishes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Activity {
    /// Whether the PipeWire graph has been read since the connection was made.
    pub available: bool,
    /// Whether a running microphone capture node exists.
    pub microphone_in_use: bool,
    /// Whether a running camera capture node exists.
    pub camera_in_use: bool,
}

/// One capture node the registry announced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TrackedNode {
    /// The id of the proxy bound to the node, which is what core errors name.
    proxy: u32,
    microphone: bool,
    camera: bool,
    running: bool,
}

/// The folded PipeWire state, keyed by the registry's global id.
#[derive(Debug, Default)]
pub struct Tracker {
    nodes: BTreeMap<u32, TrackedNode>,
    available: bool,
    lost: bool,
    published: Activity,
}

impl Tracker {
    /// Start a new connection: forget the previous tree and accept the next sync.
    pub fn reset(&mut self) {
        self.nodes.clear();
        self.available = false;
        self.lost = false;
    }

    /// A capture node appeared. `microphone` and `camera` say which class it belongs to; a
    /// node can be neither only if the caller bound it in error.
    pub fn node_bound(&mut self, global: u32, proxy: u32, microphone: bool, camera: bool) {
        self.nodes.insert(
            global,
            TrackedNode {
                proxy,
                microphone,
                camera,
                running: false,
            },
        );
    }

    /// The node's state changed. Unknown globals are ignored: a late info event for a node
    /// already removed must not resurrect it.
    pub fn node_info(&mut self, global: u32, running: bool) {
        if let Some(node) = self.nodes.get_mut(&global) {
            node.running = running;
        }
    }

    /// The registry removed a global.
    pub fn node_removed(&mut self, global: u32) {
        self.nodes.remove(&global);
    }

    /// A proxy reported an error. Only the node bound to that proxy is dropped.
    pub fn node_errored(&mut self, proxy: u32) {
        self.nodes.retain(|_, node| node.proxy != proxy);
    }

    /// The core answered the initial sync. Ignored once the connection is lost.
    pub fn synced(&mut self) {
        if !self.lost {
            self.available = true;
        }
    }

    /// The core or registry failed: the connection is unusable, so forget every node.
    pub fn lost(&mut self) {
        self.lost = true;
        self.available = false;
        self.nodes.clear();
    }

    /// Whether the connection has failed and must be rebuilt before it reports anything.
    pub fn is_lost(&self) -> bool {
        self.lost
    }

    /// The activity the current state describes.
    pub fn activity(&self) -> Activity {
        let running = |class: fn(&TrackedNode) -> bool| {
            self.available && self.nodes.values().any(|node| node.running && class(node))
        };
        Activity {
            available: self.available,
            microphone_in_use: running(|node| node.microphone),
            camera_in_use: running(|node| node.camera),
        }
    }

    /// The activity to publish, if it differs from the last one returned. The published
    /// state starts all-false, so a fresh tracker that has seen nothing yields `None`.
    pub fn take_change(&mut self) -> Option<Activity> {
        let activity = self.activity();
        if activity == self.published {
            return None;
        }
        self.published = activity;
        Some(activity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIC: u32 = 10;
    const CAM: u32 = 11;

    fn synced_tracker() -> Tracker {
        let mut tracker = Tracker::default();
        tracker.reset();
        tracker.synced();
        tracker
    }

    #[test]
    fn nothing_is_reported_before_the_core_syncs() {
        let mut tracker = Tracker::default();
        tracker.reset();
        tracker.node_bound(MIC, 100, true, false);
        tracker.node_info(MIC, true);
        assert_eq!(tracker.activity(), Activity::default());
        assert_eq!(tracker.take_change(), None);
    }

    #[test]
    fn a_running_microphone_reports_only_the_microphone() {
        let mut tracker = synced_tracker();
        tracker.node_bound(MIC, 100, true, false);
        tracker.node_info(MIC, true);
        assert_eq!(
            tracker.activity(),
            Activity {
                available: true,
                microphone_in_use: true,
                camera_in_use: false,
            }
        );
    }

    #[test]
    fn a_running_camera_reports_only_the_camera() {
        let mut tracker = synced_tracker();
        tracker.node_bound(CAM, 101, false, true);
        tracker.node_info(CAM, true);
        assert_eq!(
            tracker.activity(),
            Activity {
                available: true,
                microphone_in_use: false,
                camera_in_use: true,
            }
        );
    }

    #[test]
    fn an_idle_node_reports_nothing() {
        let mut tracker = synced_tracker();
        tracker.node_bound(MIC, 100, true, false);
        tracker.node_info(MIC, false);
        assert_eq!(
            tracker.activity(),
            Activity {
                available: true,
                ..Activity::default()
            }
        );
    }

    #[test]
    fn a_node_that_stops_running_clears_its_activity() {
        let mut tracker = synced_tracker();
        tracker.node_bound(MIC, 100, true, false);
        tracker.node_info(MIC, true);
        tracker.node_info(MIC, false);
        assert!(!tracker.activity().microphone_in_use);
    }

    #[test]
    fn removing_a_running_node_clears_its_activity() {
        let mut tracker = synced_tracker();
        tracker.node_bound(MIC, 100, true, false);
        tracker.node_info(MIC, true);
        tracker.node_removed(MIC);
        assert_eq!(
            tracker.activity(),
            Activity {
                available: true,
                ..Activity::default()
            }
        );
    }

    #[test]
    fn a_node_error_drops_only_that_node() {
        let mut tracker = synced_tracker();
        tracker.node_bound(MIC, 100, true, false);
        tracker.node_bound(CAM, 101, false, true);
        tracker.node_info(MIC, true);
        tracker.node_info(CAM, true);
        tracker.node_errored(100);
        assert_eq!(
            tracker.activity(),
            Activity {
                available: true,
                microphone_in_use: false,
                camera_in_use: true,
            }
        );
    }

    #[test]
    fn an_error_on_an_unknown_proxy_is_ignored() {
        let mut tracker = synced_tracker();
        tracker.node_bound(MIC, 100, true, false);
        tracker.node_info(MIC, true);
        tracker.node_errored(999);
        assert!(tracker.activity().microphone_in_use);
    }

    #[test]
    fn info_for_an_unknown_node_does_not_resurrect_it() {
        let mut tracker = synced_tracker();
        tracker.node_bound(MIC, 100, true, false);
        tracker.node_removed(MIC);
        tracker.node_info(MIC, true);
        assert!(!tracker.activity().microphone_in_use);
    }

    #[test]
    fn losing_the_connection_forgets_nodes_and_ignores_a_stale_sync() {
        let mut tracker = synced_tracker();
        tracker.node_bound(MIC, 100, true, false);
        tracker.node_info(MIC, true);
        tracker.lost();
        assert_eq!(tracker.activity(), Activity::default());
        tracker.synced();
        assert_eq!(tracker.activity(), Activity::default());
    }

    #[test]
    fn a_reset_connection_accepts_a_fresh_sync() {
        let mut tracker = synced_tracker();
        tracker.lost();
        tracker.reset();
        tracker.synced();
        assert_eq!(
            tracker.activity(),
            Activity {
                available: true,
                ..Activity::default()
            }
        );
    }

    #[test]
    fn take_change_emits_only_real_changes() {
        let mut tracker = Tracker::default();
        tracker.reset();
        assert_eq!(
            tracker.take_change(),
            None,
            "the initial all-false state is not emitted"
        );

        tracker.synced();
        assert_eq!(
            tracker.take_change(),
            Some(Activity {
                available: true,
                ..Activity::default()
            })
        );
        assert_eq!(tracker.take_change(), None, "an unchanged state is not re-emitted");

        tracker.node_bound(MIC, 100, true, false);
        tracker.node_info(MIC, true);
        assert_eq!(
            tracker.take_change(),
            Some(Activity {
                available: true,
                microphone_in_use: true,
                camera_in_use: false,
            })
        );
        tracker.node_info(MIC, true);
        assert_eq!(
            tracker.take_change(),
            None,
            "a duplicate running update is not re-emitted"
        );

        tracker.lost();
        assert_eq!(tracker.take_change(), Some(Activity::default()));
    }
}
