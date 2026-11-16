//! Lock UI: the compositor's lock node, the lock screen the shell draws on it, and the keys it takes.
//!
//! A move out of `compose.rs`: these are methods of [`Composer`] and read its fields.

use super::*;

impl Composer {
    /// Whether the lock node, or a canvas under it, changed since the last call. The lock cover redraws on it.
    pub fn take_lock_ui_changed(&mut self) -> bool {
        std::mem::take(&mut self.lock_ui_changed)
    }

    /// Takes the lock nodes whose session lock went `pending` since the last take. Each is
    /// announced to the shell once per transition to `pending`.
    pub fn take_lock_requested(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.lock_requested)
    }

    /// Adds the compositor's lock node under the root, in the `not-locked` state. Idempotent.
    pub fn add_lock_node(&mut self) -> Option<NodeId> {
        if let Some(lock) = self.lock_node {
            return Some(lock);
        }
        let id = self.node_ids.allocate()?;
        let root = *self.scene.root_ids().first()?;
        let mut spec = NodeSpec::new(Rect::new(0, 0, 0, 0));
        spec.parent = Some(root);
        spec.input = false;
        self.scene.insert(id, spec).ok()?;
        let _ = self.scene.set_class(id, Some("lock".to_owned()));
        let _ = self.scene.set_text(id, "state", "not-locked".to_owned());
        let index = self.scene.root_ids().iter().position(|root| *root == id).unwrap_or(0);
        self.tree_changes.push(TreeChange::Created {
            node: id,
            class: "lock".to_owned(),
            index,
        });
        self.lock_node = Some(id);
        Some(id)
    }

    /// Publishes the session lock's state on the lock node: `not-locked`, `pending` or
    /// `locked`. Only a change is published. A lock that leaves `not-locked` asks the shell for
    /// its lock screen, once: a lock can go straight to `locked` in one dispatch, so `pending`
    /// is not always published first.
    pub fn set_lock_state(&mut self, state: &str) {
        let Some(lock) = self.lock_node else {
            return;
        };
        let previous = self.scene.text(lock, "state").map(str::to_owned);
        if previous.as_deref() == Some(state) {
            return;
        }
        let _ = self.scene.set_text(lock, "state", state.to_owned());
        self.tree_changes.push(TreeChange::Set {
            node: lock,
            key: "state".to_owned(),
            value: TreeValue::Text(state.to_owned()),
        });
        if previous.as_deref() == Some("not-locked") && state != "not-locked" {
            self.lock_requested.push(lock);
        }
    }

    /// The shell's node that takes the keys while the compositor's own lock covers the session: the same node
    /// [`Self::shell_key_target`] names, only while the shell's lock screen is up (the `lock` node's `engaged` is
    /// true), and only when that node is part of the lock screen, under the `lock` node. The shell is the compositor's
    /// own trusted peer and the lock screen is what it draws, so the typed password reaches the screen that checks
    /// it; no client ever sees the keys, and no other node of the shell does either.
    #[must_use]
    pub fn shell_lock_key_target(&self) -> Option<NodeId> {
        if !self.lock_ui_engaged() {
            return None;
        }
        self.shell_key_target().filter(|node| self.under_lock_node(*node))
    }

    /// Whether `node` is the `lock` node's descendant: a canvas of the shell's lock screen.
    pub(super) fn under_lock_node(&self, mut node: NodeId) -> bool {
        let Some(lock) = self.lock_node else {
            return false;
        };
        while let Some(parent) = self.scene.parent(node) {
            if parent == lock {
                return true;
            }
            node = parent;
        }
        false
    }

    /// Whether the shell has engaged the lock: its lock screen is up and holds the lock UI.
    #[must_use]
    pub fn lock_ui_engaged(&self) -> bool {
        self.lock_node
            .is_some_and(|lock| matches!(self.scene.value(lock, "engaged"), Some(TreeValue::Bool(true))))
    }

    /// Clears the lock node's `engaged`: the shell that set it no longer holds the tree, so nothing is holding the lock
    /// screen. Published like any other change, so the next shell reads a lock nobody holds.
    pub fn release_lock_engaged(&mut self) {
        let Some(lock) = self.lock_node else {
            return;
        };
        if self.scene.set_value(lock, "engaged", TreeValue::Bool(false)).is_ok() {
            self.tree_changes.push(TreeChange::Set {
                node: lock,
                key: "engaged".to_owned(),
                value: TreeValue::Bool(false),
            });
        }
    }

    /// Draws the lock screen the shell made over a lock cover: the canvases under the `lock` node, in paint order,
    /// while the shell holds the tree and has engaged the lock. They are drawn into a frame whose background is the
    /// cover, so a shell that draws nothing leaves the bare cover, and nothing but those canvases ever shows: no window
    /// and no canvas outside the lock node's subtree. Returns whether anything was drawn.
    pub fn compose_lock_ui(&self, output: OutputId, frame: &mut OutputFrame) -> bool {
        let (Some(_), Some(rect)) = (self.lock_node, self.scene.outputs().get(&output).copied()) else {
            return false;
        };
        if !self.lock_ui_engaged() || !self.scene.holder_attached() {
            return false;
        }
        let ranks = self.paint_ranks();
        let mut canvases: Vec<(u32, NodeId)> = self
            .canvases
            .iter()
            .filter(|(node, picture)| {
                picture.image.is_some() && self.scene.drawn(**node) && self.under_lock_node(**node)
            })
            .filter_map(|(node, _)| Some((*ranks.get(node)?, *node)))
            .collect();
        canvases.sort_unstable();
        let mut spatial = false;
        for (_, canvas) in &canvases {
            self.push_canvas(*canvas, rect, frame, &mut spatial);
        }
        !canvases.is_empty()
    }
}
