//! What a shell that holds the tree may do with the surfaces the compositor made nodes of: the popups and subsurfaces of
//! a window or of a layer surface, and the layer-shell surfaces themselves (`placement_override`, `stack`, `visible` and
//! `reserve` on a `surface` node).
//!
//! A move out of `compose.rs`: these are methods of [`Composer`] and read its fields. The composer draws a subsurface
//! where the shell put it, orders surfaces by their `stack`, skips a hidden one and its subtree, and answers the questions
//! the protocol engine asks to act on the rest: a popup is reconfigured and a layer surface re-placed by `gnoblin-wayland`,
//! which owns the protocol, from what [`Composer::surface_shell`] and [`Composer::layer_override`] report. The layer
//! surface's own node, its pixels and its place in the frame are in `layers.rs`.
//!
//! Everything here is read only while a shell holds the tree, and [`Composer::release_surface_controls`] puts it all
//! back when it lets go, so the compositor takes the placement over again and the next shell starts from the protocol's.

use super::*;

/// What the shell last asked of a surface node, so a change is noticed once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ShellSeen {
    /// Whether `placement_override` held.
    placed: bool,
    /// The place the shell wrote, whole pixels, when it placed the surface.
    at: (i32, i32),
    /// The `stack` the shell wrote, when it placed the surface.
    stack: i64,
    /// Whether the node was hidden.
    hidden: bool,
}

/// What the shell asked of one surface of a surface tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceShell {
    /// The surface's key in the list [`Composer::set_window_surfaces`] or [`Composer::set_layer_surfaces`] was given.
    pub key: u64,
    /// What the surface is.
    pub role: TreeSurfaceRole,
    /// The place the shell wrote on the node, in its parent's plane, when `placement_override` is true.
    pub placed: Option<(f64, f64)>,
    /// Whether the shell hid the node.
    pub hidden: bool,
}

/// One surface of a surface tree as the pointer finds it, in the order it is found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PickSurface {
    /// The surface's key, or `None` for the root surface: a window's toplevel surface or a layer surface's own.
    pub key: Option<u64>,
    /// Whether the surface is a popup or hangs from one.
    pub popup: bool,
    /// Where the surface's top left corner is, in the root node's plane. A window's node starts at the window geometry's
    /// origin, a layer surface's node where its surface does.
    pub origin: (i32, i32),
}

impl Composer {
    /// Whether the shell places this surface node itself: it holds the tree and wrote `placement_override`.
    pub(super) fn surface_placed(&self, node: NodeId) -> bool {
        self.scene.holder_attached()
            && matches!(
                self.scene.value(node, "placement_override"),
                Some(TreeValue::Bool(true))
            )
    }

    /// Which side of its parent a surface paints on and where it stands among its siblings: the shell's `stack` of a
    /// surface it placed (negative is behind the parent, positive in front), the protocol's side and 0 otherwise.
    pub(super) fn surface_side(&self, node: NodeId, protocol_behind: bool) -> (bool, i64) {
        if !self.surface_placed(node) {
            return (protocol_behind, 0);
        }
        let stack = match self.scene.value(node, "stack") {
            Some(TreeValue::Int(stack)) => *stack,
            _ => 0,
        };
        let behind = match stack.cmp(&0) {
            std::cmp::Ordering::Less => true,
            std::cmp::Ordering::Greater => false,
            std::cmp::Ordering::Equal => protocol_behind,
        };
        (behind, stack)
    }

    /// The place a node holds in its parent's plane, in whole pixels.
    fn node_origin(&self, node: NodeId) -> (i32, i32) {
        self.scene.transform(node).map_or((0, 0), |transform| {
            (
                transform.base.offset.x.round() as i32,
                transform.base.offset.y.round() as i32,
            )
        })
    }

    /// What the shell asked of a surface node now.
    fn shell_seen(&self, node: NodeId) -> ShellSeen {
        let placed = self.surface_placed(node);
        ShellSeen {
            placed,
            at: if placed { self.node_origin(node) } else { (0, 0) },
            stack: if placed { self.surface_side(node, false).1 } else { 0 },
            hidden: self.scene.visible(node) == Some(false),
        }
    }

    /// Every owner of a surface tree: the windows, then the layer surfaces.
    fn tree_owners(&self) -> Vec<TreeOwner> {
        self.windows
            .keys()
            .map(|window| TreeOwner::Window(*window))
            .chain(self.layer_nodes.keys().map(|node| TreeOwner::Layer(*node)))
            .collect()
    }

    /// Notices a surface node the shell placed, restacked or hid since the last call, and builds the owner's tree again so
    /// the scene follows. Returns the owners that changed, so the protocol engine acts on them too.
    pub fn follow_surface_shell(&mut self) -> Vec<TreeOwner> {
        let mut changed = Vec::new();
        for owner in self.tree_owners() {
            let Some(tree) = self.tree_ref(owner) else {
                continue;
            };
            let moved: Vec<(u64, ShellSeen)> = tree
                .nodes
                .iter()
                .map(|(key, node)| (*key, self.shell_seen(node.node), node.seen))
                .filter(|(_, now, before)| now != before)
                .map(|(key, now, _)| (key, now))
                .collect();
            if moved.is_empty() {
                continue;
            }
            if let Some(tree) = self.tree_mut(owner) {
                for (key, now) in moved {
                    if let Some(node) = tree.nodes.get_mut(&key) {
                        node.seen = now;
                    }
                }
            }
            self.sync_tree(owner);
            if let TreeOwner::Window(window) = owner {
                self.apply_extents(window);
            }
            changed.push(owner);
        }
        changed
    }

    /// What the shell asked of the surfaces of an owner's tree that it placed or hid.
    #[must_use]
    pub fn surface_shell(&self, owner: TreeOwner) -> Vec<SurfaceShell> {
        let Some(tree) = self.tree_ref(owner) else {
            return Vec::new();
        };
        tree.nodes
            .iter()
            .filter_map(|(key, node)| {
                let placed = self.surface_placed(node.node).then(|| {
                    let at = self.node_origin(node.node);
                    (f64::from(at.0), f64::from(at.1))
                });
                let hidden = self.scene.visible(node.node) == Some(false);
                (placed.is_some() || hidden).then_some(SurfaceShell {
                    key: *key,
                    role: node.role,
                    placed,
                    hidden,
                })
            })
            .collect()
    }

    /// The surfaces of an owner's tree as the pointer finds them, topmost first: each visible surface of its tree and the
    /// root surface, in the order the scene draws them, with the place each has in the root node's plane. A surface the
    /// shell hid is left out with everything under it.
    #[must_use]
    pub fn pick_order(&self, owner: TreeOwner) -> Vec<PickSurface> {
        let (Some(tree), Some((node, offset))) = (self.tree_ref(owner), self.tree_root(owner)) else {
            return Vec::new();
        };
        let mut painted = Vec::new();
        self.collect_painted(tree, node, (0, 0), false, true, &mut painted);
        painted.push(PickSurface {
            key: None,
            popup: false,
            origin: (-offset.0, -offset.1),
        });
        self.collect_painted(tree, node, (0, 0), false, false, &mut painted);
        painted.reverse();
        painted
    }

    /// Adds the surfaces under `parent` that paint behind it (`behind`) or in front of it, each with the surfaces under it,
    /// in the order [`Composer::push_children`] draws them.
    fn collect_painted(
        &self,
        tree: &SurfaceTree,
        parent: NodeId,
        parent_origin: (i32, i32),
        in_popup: bool,
        behind: bool,
        out: &mut Vec<PickSurface>,
    ) {
        for child in self.scene.children(parent).unwrap_or(&[]) {
            if self.scene.behind(*child) != Some(behind) || self.scene.visible(*child) == Some(false) {
                continue;
            }
            let Some(key) = tree.keys.get(child) else {
                continue;
            };
            let Some(node) = tree.nodes.get(key) else {
                continue;
            };
            let at = self.node_origin(*child);
            let origin = (parent_origin.0 + at.0, parent_origin.1 + at.1);
            let popup = in_popup || node.role == TreeSurfaceRole::Popup;
            self.collect_painted(tree, *child, origin, popup, true, out);
            out.push(PickSurface {
                key: Some(*key),
                popup,
                origin,
            });
            self.collect_painted(tree, *child, origin, popup, false, out);
        }
    }

    /// The rectangle, in the window node's plane, that the subsurfaces the shell placed cover: where the pointer must be
    /// let reach the window although the protocol's own tree does not extend there.
    pub(super) fn placed_tree_extent(&self, window: WindowId) -> Option<Rect> {
        let entry = self.windows.get(&window)?;
        let mut union: Option<Rect> = None;
        for surface in self.pick_order(TreeOwner::Window(window)) {
            let Some(key) = surface.key else {
                continue;
            };
            let Some(tree) = entry.tree.nodes.get(&key) else {
                continue;
            };
            if tree.role != TreeSurfaceRole::Subsurface || !self.surface_placed(tree.node) {
                continue;
            }
            let size = self.scene.bounds(tree.node)?;
            let rect = Rect::new(surface.origin.0, surface.origin.1, size.width, size.height);
            union = Some(union.map_or(rect, |held| held.union(rect)));
        }
        union
    }

    /// Sets what a shell wrote to place or hide a surface back to its default, once the shell no longer holds the tree:
    /// `placement_override` is false, `stack` 0, `reserve` empty, and every surface is visible. Each change is published
    /// like any other. The popups and subsurfaces are drawn from the protocol's own places again at the next sync, and the
    /// protocol engine puts the popups and layer surfaces where the protocol says.
    pub(super) fn release_surface_controls(&mut self) {
        let owners = self.tree_owners();
        let mut nodes: Vec<NodeId> = owners
            .iter()
            .filter_map(|owner| self.tree_ref(*owner))
            .flat_map(|tree| tree.nodes.values().map(|node| node.node))
            .collect();
        nodes.extend(self.layer_nodes.keys().copied());
        for node in nodes {
            for (key, value) in [
                ("placement_override", TreeValue::Bool(false)),
                ("stack", TreeValue::Int(0)),
                ("reserve", TreeValue::ListDouble(Vec::new())),
            ] {
                if self.scene.value(node, key).is_some_and(|held| *held != value) {
                    self.write_surface_value(node, key, value);
                }
            }
            if self.scene.visible(node) == Some(false) && self.scene.set_visible(node, true).is_ok() {
                self.tree_changes.push(TreeChange::Set {
                    node,
                    key: "visible".to_owned(),
                    value: TreeValue::Bool(true),
                });
            }
        }
        for owner in owners {
            if let Some(tree) = self.tree_mut(owner) {
                for node in tree.nodes.values_mut() {
                    node.seen = ShellSeen::default();
                }
            }
            self.sync_tree(owner);
            if let TreeOwner::Window(window) = owner {
                self.apply_extents(window);
            }
        }
    }

    /// Stores a value on a surface node and queues it for the next change batch.
    pub(super) fn write_surface_value(&mut self, node: NodeId, key: &str, value: TreeValue) {
        if self.scene.set_value(node, key, value.clone()).is_ok() {
            self.tree_changes.push(TreeChange::Set {
                node,
                key: key.to_owned(),
                value,
            });
        }
    }

    /// Stores a value on a surface node and queues it for the next change batch, unless the node already holds it.
    pub(super) fn publish_surface_value(&mut self, node: NodeId, key: &str, value: TreeValue) {
        if self.scene.value(node, key) != Some(&value) {
            self.write_surface_value(node, key, value);
        }
    }
}
