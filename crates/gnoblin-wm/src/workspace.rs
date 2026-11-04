//! The declared workspace list and the addressing rules that go with it.
//!
//! A workspace has a stable id and a position. Positions change as workspaces are
//! added, removed or reordered, so everything that must survive that (window
//! membership, remembered placement) uses the id, and only user-facing ordering
//! uses the one-based number.

use crate::ids::WorkspaceId;
use crate::rules::WorkspaceSelector;

/// Workspaces in position order.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub(crate) struct Workspaces {
    ids: Vec<WorkspaceId>,
}

impl Workspaces {
    /// Appends a workspace unless its id is already present.
    pub(crate) fn add(&mut self, id: WorkspaceId) -> bool {
        if self.contains(id) {
            return false;
        }
        self.ids.push(id);
        true
    }

    /// Removes a workspace, reporting whether it was present.
    pub(crate) fn remove(&mut self, id: WorkspaceId) -> bool {
        let before = self.ids.len();
        self.ids.retain(|existing| *existing != id);
        self.ids.len() != before
    }

    /// Whether the id is declared.
    pub(crate) fn contains(&self, id: WorkspaceId) -> bool {
        self.ids.contains(&id)
    }

    /// The declared ids, in position order.
    pub(crate) fn ids(&self) -> &[WorkspaceId] {
        &self.ids
    }

    /// Whether nothing is declared.
    pub(crate) fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The workspace at a zero-based position.
    pub(crate) fn at(&self, position: usize) -> Option<WorkspaceId> {
        self.ids.get(position).copied()
    }

    /// The workspace at a one-based position, which is what rules and the CLI use.
    pub(crate) fn by_number(&self, number: u32) -> Option<WorkspaceId> {
        usize::try_from(number)
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|p| self.at(p))
    }

    /// The workspace that takes over when `id` is removed.
    ///
    /// The previous workspace wins so that removing the active workspace moves the
    /// user back towards the start, which matches the reading order of the list.
    /// Removing the first workspace falls forward to the next one, and the last
    /// remaining workspace has no successor at all.
    pub(crate) fn neighbour_of(&self, id: WorkspaceId) -> Option<WorkspaceId> {
        let position = self.ids.iter().position(|existing| *existing == id)?;
        if position > 0 {
            return self.at(position - 1);
        }
        self.at(position + 1)
    }

    /// Resolves a rule's workspace target against the declared list.
    pub(crate) fn resolve(&self, selector: WorkspaceSelector) -> Option<WorkspaceId> {
        match selector {
            WorkspaceSelector::Id(id) => self.contains(id).then_some(id),
            WorkspaceSelector::Number(number) => self.by_number(number),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: u32) -> WorkspaceId {
        WorkspaceId::new(raw)
    }

    #[test]
    fn positions_follow_declaration_order() {
        let mut workspaces = Workspaces::default();
        assert!(workspaces.add(id(10)));
        assert!(workspaces.add(id(20)));
        assert!(workspaces.add(id(30)));
        assert_eq!(workspaces.by_number(2), Some(id(20)));
        assert_eq!(workspaces.by_number(3), Some(id(30)));
        assert_eq!(workspaces.by_number(0), None);
        assert_eq!(workspaces.by_number(4), None);
    }

    #[test]
    fn duplicate_ids_are_refused() {
        let mut workspaces = Workspaces::default();
        assert!(workspaces.add(id(10)));
        assert!(!workspaces.add(id(10)));
        assert_eq!(workspaces.ids().len(), 1);
    }

    #[test]
    fn removing_prefers_the_previous_workspace() {
        let mut workspaces = Workspaces::default();
        for raw in [10, 20, 30] {
            workspaces.add(id(raw));
        }
        assert_eq!(workspaces.neighbour_of(id(20)), Some(id(10)));
        assert_eq!(workspaces.neighbour_of(id(10)), Some(id(20)));
        assert!(workspaces.remove(id(20)));
        assert_eq!(workspaces.ids(), [id(10), id(30)]);
        assert_eq!(workspaces.neighbour_of(id(30)), Some(id(10)));
    }

    #[test]
    fn the_last_workspace_has_no_neighbour() {
        let mut workspaces = Workspaces::default();
        workspaces.add(id(10));
        assert_eq!(workspaces.neighbour_of(id(10)), None);
        assert_eq!(workspaces.neighbour_of(id(99)), None);
    }

    #[test]
    fn selectors_resolve_by_id_or_number() {
        let mut workspaces = Workspaces::default();
        workspaces.add(id(10));
        workspaces.add(id(20));
        assert_eq!(workspaces.resolve(WorkspaceSelector::Id(id(20))), Some(id(20)));
        assert_eq!(workspaces.resolve(WorkspaceSelector::Id(id(30))), None);
        assert_eq!(workspaces.resolve(WorkspaceSelector::Number(1)), Some(id(10)));
        assert_eq!(workspaces.resolve(WorkspaceSelector::Number(9)), None);
    }
}
