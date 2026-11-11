//! The set of live captures, one per View node a frame plans. Knows nothing of Wayland: the
//! opener is a closure, so the diff and the failure handling are tested without a compositor.

use std::collections::{BTreeMap, BTreeSet};

/// The node ids to open and to close to turn the current set into the wanted one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changes {
    /// Wanted but not current, in ascending order.
    pub added: Vec<u64>,
    /// Current but not wanted, in ascending order.
    pub removed: Vec<u64>,
}

/// The diff from `current` to `wanted`.
pub fn changes(current: &BTreeSet<u64>, wanted: &BTreeSet<u64>) -> Changes {
    Changes {
        added: wanted.difference(current).copied().collect(),
        removed: current.difference(wanted).copied().collect(),
    }
}

/// One capture per node id. `C` is the capture; the set only owns and replaces them.
pub struct CaptureSet<C> {
    captures: BTreeMap<u64, C>,
}

impl<C> Default for CaptureSet<C> {
    fn default() -> Self {
        Self {
            captures: BTreeMap::new(),
        }
    }
}

impl<C> CaptureSet<C> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.captures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.captures.is_empty()
    }

    pub fn get(&self, node: u64) -> Option<&C> {
        self.captures.get(&node)
    }

    /// Every capture with its node id, in ascending id order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (u64, &mut C)> {
        self.captures.iter_mut().map(|(node, capture)| (*node, capture))
    }

    /// Drops the capture of `node`, if any, and returns it.
    pub fn remove(&mut self, node: u64) -> Option<C> {
        self.captures.remove(&node)
    }

    /// Brings the set to `wanted`. Captures no longer wanted are dropped; each newly wanted node
    /// is opened with `open`. A node whose open fails is left out of the set, and its error is
    /// returned with it. The other nodes are still opened, and the failed node is tried again by
    /// the next `sync` that still wants it.
    pub fn sync<E>(&mut self, wanted: &BTreeSet<u64>, mut open: impl FnMut(u64) -> Result<C, E>) -> Vec<(u64, E)> {
        let current: BTreeSet<u64> = self.captures.keys().copied().collect();
        let Changes { added, removed } = changes(&current, wanted);
        for node in removed {
            self.captures.remove(&node);
        }
        let mut failures = Vec::new();
        for node in added {
            match open(node) {
                Ok(capture) => {
                    self.captures.insert(node, capture);
                }
                Err(error) => failures.push((node, error)),
            }
        }
        failures
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodes(ids: &[u64]) -> BTreeSet<u64> {
        ids.iter().copied().collect()
    }

    #[test]
    fn changes_are_the_symmetric_split() {
        let current = nodes(&[1, 2, 3]);
        let wanted = nodes(&[2, 3, 4]);
        assert_eq!(
            changes(&current, &wanted),
            Changes {
                added: vec![4],
                removed: vec![1],
            }
        );
    }

    #[test]
    fn identical_sets_change_nothing() {
        let set = nodes(&[5, 6]);
        assert_eq!(
            changes(&set, &set),
            Changes {
                added: Vec::new(),
                removed: Vec::new(),
            }
        );
    }

    #[test]
    fn sync_opens_new_nodes_and_drops_unwanted_ones() {
        let mut set: CaptureSet<u64> = CaptureSet::new();
        let failures = set.sync(&nodes(&[1, 2]), |node| Ok::<_, ()>(node * 10));
        assert!(failures.is_empty());
        assert_eq!(set.len(), 2);
        assert_eq!(set.get(2), Some(&20));

        let failures = set.sync(&nodes(&[2, 3]), |node| Ok::<_, ()>(node * 10));
        assert!(failures.is_empty());
        assert_eq!(set.get(1), None);
        assert_eq!(set.get(2), Some(&20));
        assert_eq!(set.get(3), Some(&30));
    }

    #[test]
    fn sync_opens_each_wanted_node_once() {
        let mut set: CaptureSet<()> = CaptureSet::new();
        let mut opened = Vec::new();
        for _ in 0..3 {
            let _ = set.sync(&nodes(&[7]), |node| {
                opened.push(node);
                Ok::<_, ()>(())
            });
        }
        assert_eq!(opened, vec![7]);
    }

    #[test]
    fn a_failed_open_is_reported_and_the_rest_still_open() {
        let mut set: CaptureSet<u64> = CaptureSet::new();
        let failures = set.sync(
            &nodes(&[1, 9, 2]),
            |node| {
                if node == 9 { Err("refused") } else { Ok(node) }
            },
        );
        assert_eq!(failures, vec![(9, "refused")]);
        assert_eq!(set.get(1), Some(&1));
        assert_eq!(set.get(2), Some(&2));
        assert_eq!(set.get(9), None);
    }

    #[test]
    fn a_failed_node_is_tried_again_by_a_later_sync() {
        let mut set: CaptureSet<u64> = CaptureSet::new();
        let _ = set.sync(&nodes(&[9]), |_| Err::<u64, _>("refused"));
        assert!(set.is_empty());
        let failures = set.sync(&nodes(&[9]), Ok::<_, &str>);
        assert!(failures.is_empty());
        assert_eq!(set.get(9), Some(&9));
    }

    #[test]
    fn remove_returns_the_capture() {
        let mut set: CaptureSet<u64> = CaptureSet::new();
        let _ = set.sync(&nodes(&[4]), Ok::<_, ()>);
        assert_eq!(set.remove(4), Some(4));
        assert_eq!(set.remove(4), None);
    }
}
