//! The recorded parent relations, and the one place they are kept.
//!
//! A relation is one Wayland window and one X11 window: the child is the Wayland toplevel
//! the protocol named, the parent is whichever of this session's windows that X11 id
//! belongs to. Both are [`gnoblin_types::WindowId`], because the X11 window table gives
//! its windows identities from the same numbering as everything else.
//!
//! The table is separate from the dispatch so a test can state a relation without a
//! socket: `gnoblin-wm` has no event to change an already-managed window's transient, so
//! what this crate does with a relation today is record it, and what it records is
//! observable in exactly one place.

use std::collections::BTreeMap;

use gnoblin_types::WindowId;

/// The parent of each Wayland window that has one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParentRelations {
    /// Child to parent.
    parents: BTreeMap<WindowId, WindowId>,
}

impl ParentRelations {
    /// An empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a child's parent, replacing an earlier relation for the same child.
    ///
    /// A dialog that is re-parented (the portal naming a different X11 window) ends up
    /// with one parent, which is the last one stated: the protocol has no "remove the
    /// parent" request, so the alternative is a table that grows a list nothing reads.
    pub fn record(&mut self, child: WindowId, parent: WindowId) {
        self.parents.insert(child, parent);
    }

    /// The parent of one child, if it has one.
    #[must_use]
    pub fn parent_of(&self, child: WindowId) -> Option<WindowId> {
        self.parents.get(&child).copied()
    }

    /// Removes one child's relation, when the child goes away.
    ///
    /// Removing a child that had none is not an error: a dialog that was never
    /// re-parented and then closed is the ordinary case.
    pub fn forget(&mut self, child: WindowId) {
        self.parents.remove(&child);
    }

    /// How many relations are recorded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.parents.len()
    }

    /// Whether no relation is recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.parents.is_empty()
    }

    /// Every relation, child first, in identity order.
    pub fn iter(&self) -> impl Iterator<Item = (WindowId, WindowId)> + '_ {
        self.parents.iter().map(|(child, parent)| (*child, *parent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table is keyed by the child, so one parent can hold any number of dialogs.
    #[test]
    fn one_parent_can_have_many_children() {
        let mut relations = ParentRelations::new();
        let parent = WindowId::new(1);
        relations.record(WindowId::new(2), parent);
        relations.record(WindowId::new(3), parent);
        assert_eq!(relations.len(), 2);
        assert_eq!(relations.parent_of(WindowId::new(2)), Some(parent));
        assert_eq!(relations.parent_of(WindowId::new(3)), Some(parent));
    }

    /// Re-parenting replaces the relation rather than adding a second one.
    #[test]
    fn a_second_parent_replaces_the_first() {
        let mut relations = ParentRelations::new();
        relations.record(WindowId::new(2), WindowId::new(1));
        relations.record(WindowId::new(2), WindowId::new(9));
        assert_eq!(relations.len(), 1);
        assert_eq!(relations.parent_of(WindowId::new(2)), Some(WindowId::new(9)));
    }

    /// Forgetting a child removes that relation and no other.
    #[test]
    fn forgetting_a_child_removes_only_that_child() {
        let mut relations = ParentRelations::new();
        relations.record(WindowId::new(2), WindowId::new(1));
        relations.record(WindowId::new(3), WindowId::new(1));
        relations.forget(WindowId::new(2));
        assert_eq!(relations.parent_of(WindowId::new(2)), None);
        assert_eq!(relations.parent_of(WindowId::new(3)), Some(WindowId::new(1)));
        relations.forget(WindowId::new(7));
        assert_eq!(relations.len(), 1, "forgetting an unknown child is not an error");
    }
}
