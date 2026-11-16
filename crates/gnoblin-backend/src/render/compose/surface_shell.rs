//! What a shell that holds the tree may do with the surfaces the compositor made nodes of: the popups and subsurfaces of
//! a window and the layer-shell surfaces (`placement_override`, `stack`, `visible` and `reserve` on a `surface` node).
//!
//! A move out of `compose.rs`: these are methods of [`Composer`] and read its fields. The composer draws a subsurface
//! where the shell put it, orders surfaces by their `stack`, skips a hidden one and its subtree, and answers the questions
//! the protocol engine asks to act on the rest: a popup is reconfigured and a layer surface re-placed by `gnoblin-wayland`,
//! which owns the protocol, from what [`Composer::window_surface_shell`] and [`Composer::layer_override`] report.
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

/// What the shell asked of one surface of a window's tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceShell {
    /// The surface's key in the list [`Composer::set_window_surfaces`] was given.
    pub key: u64,
    /// What the surface is.
    pub role: TreeSurfaceRole,
    /// The place the shell wrote on the node, in its parent's plane, when `placement_override` is true.
    pub placed: Option<(f64, f64)>,
    /// Whether the shell hid the node.
    pub hidden: bool,
}

/// One surface of a window as the pointer finds it, in the order it is found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PickSurface {
    /// The surface's key, or `None` for the window's own toplevel surface.
    pub key: Option<u64>,
    /// Whether the surface is a popup or hangs from one.
    pub popup: bool,
    /// Where the surface's top left corner is, in the window node's plane (which starts at the window geometry's origin).
    pub origin: (i32, i32),
}

/// What a layer-shell surface publishes of itself to its node: what the client asked for and where it is.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerNodeInfo {
    /// The namespace the client named.
    pub namespace: String,
    /// `background`, `bottom`, `top` or `overlay`.
    pub layer: &'static str,
    /// The anchored edges, among `top`, `bottom`, `left` and `right`.
    pub anchor: Vec<String>,
    /// The margins the client asked for: top, right, bottom, left.
    pub margin: [f64; 4],
    /// The size the client asked for: width, height.
    pub requested: [f64; 2],
    /// The exclusive zone the client asked for, as the protocol states it.
    pub zone: f64,
    /// Whether the surface has committed a buffer.
    pub mapped: bool,
}

/// What the shell wrote on a layer surface's node while it places the surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerOverride {
    /// The rectangle the shell wants the surface at, on the output plane. A width or height of 0 is left to the protocol.
    pub rect: Rect,
    /// The insets the shell wants the surface to reserve instead of its own exclusive zone's strip, when it wrote four.
    pub reserve: Option<gnoblin_types::Insets>,
}

/// A layer surface's node and what was last published of its place.
#[derive(Clone, Copy, Debug)]
pub(super) struct LayerNode {
    /// The `[x, y, width, height]` last published; NaN before the first.
    published: [f64; 4],
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

    /// Notices a surface node the shell placed, restacked or hid since the last call, and builds the window's tree again so
    /// the scene follows. Returns the windows that changed, so the protocol engine acts on them too.
    pub fn follow_surface_shell(&mut self) -> Vec<WindowId> {
        let mut changed = Vec::new();
        let windows: Vec<WindowId> = self.windows.keys().copied().collect();
        for window in windows {
            let Some(entry) = self.windows.get(&window) else {
                continue;
            };
            let moved: Vec<(u64, ShellSeen)> = entry
                .tree_nodes
                .iter()
                .map(|(key, node)| (*key, self.shell_seen(node.node), node.seen))
                .filter(|(_, now, before)| now != before)
                .map(|(key, now, _)| (key, now))
                .collect();
            if moved.is_empty() {
                continue;
            }
            if let Some(entry) = self.windows.get_mut(&window) {
                for (key, now) in moved {
                    if let Some(node) = entry.tree_nodes.get_mut(&key) {
                        node.seen = now;
                    }
                }
            }
            self.sync_tree(window);
            self.apply_extents(window);
            changed.push(window);
        }
        changed
    }

    /// What the shell asked of the surfaces of a window's tree that it placed or hid.
    #[must_use]
    pub fn window_surface_shell(&self, window: WindowId) -> Vec<SurfaceShell> {
        let Some(entry) = self.windows.get(&window) else {
            return Vec::new();
        };
        entry
            .tree_nodes
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

    /// The surfaces of a window as the pointer finds them, topmost first: each visible surface of its tree and the
    /// window's own surface, in the order the scene draws them, with the place each has in the window node's plane. A
    /// surface the shell hid is left out with everything under it.
    #[must_use]
    pub fn window_pick_order(&self, window: WindowId) -> Vec<PickSurface> {
        let Some(entry) = self.windows.get(&window) else {
            return Vec::new();
        };
        let Some(node) = entry.node else {
            return Vec::new();
        };
        let mut painted = Vec::new();
        self.collect_painted(entry, node, (0, 0), false, true, &mut painted);
        painted.push(PickSurface {
            key: None,
            popup: false,
            origin: (-entry.surface_offset.0, -entry.surface_offset.1),
        });
        self.collect_painted(entry, node, (0, 0), false, false, &mut painted);
        painted.reverse();
        painted
    }

    /// Adds the surfaces under `parent` that paint behind it (`behind`) or in front of it, each with the surfaces under it,
    /// in the order [`Composer::push_children`] draws them.
    fn collect_painted(
        &self,
        entry: &WindowNode,
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
            let Some(key) = entry.tree_keys.get(child) else {
                continue;
            };
            let Some(node) = entry.tree_nodes.get(key) else {
                continue;
            };
            let at = self.node_origin(*child);
            let origin = (parent_origin.0 + at.0, parent_origin.1 + at.1);
            let popup = in_popup || node.role == TreeSurfaceRole::Popup;
            self.collect_painted(entry, *child, origin, popup, true, out);
            out.push(PickSurface {
                key: Some(*key),
                popup,
                origin,
            });
            self.collect_painted(entry, *child, origin, popup, false, out);
        }
    }

    /// The rectangle, in the window node's plane, that the subsurfaces the shell placed cover: where the pointer must be
    /// let reach the window although the protocol's own tree does not extend there.
    pub(super) fn placed_tree_extent(&self, window: WindowId) -> Option<Rect> {
        let entry = self.windows.get(&window)?;
        let mut union: Option<Rect> = None;
        for surface in self.window_pick_order(window) {
            let Some(key) = surface.key else {
                continue;
            };
            let Some(tree) = entry.tree_nodes.get(&key) else {
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
        let mut nodes: Vec<NodeId> = self
            .windows
            .values()
            .flat_map(|entry| entry.tree_nodes.values().map(|node| node.node))
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
        let windows: Vec<WindowId> = self.windows.keys().copied().collect();
        for window in windows {
            if let Some(entry) = self.windows.get_mut(&window) {
                for node in entry.tree_nodes.values_mut() {
                    node.seen = ShellSeen::default();
                }
            }
            self.sync_tree(window);
            self.apply_extents(window);
        }
    }

    /// Stores a value on a surface node and queues it for the next change batch.
    fn write_surface_value(&mut self, node: NodeId, key: &str, value: TreeValue) {
        if self.scene.set_value(node, key, value.clone()).is_ok() {
            self.tree_changes.push(TreeChange::Set {
                node,
                key: key.to_owned(),
                value,
            });
        }
    }

    /// Stores a value on a surface node and queues it for the next change batch, unless the node already holds it.
    fn publish_surface_value(&mut self, node: NodeId, key: &str, value: TreeValue) {
        if self.scene.value(node, key) != Some(&value) {
            self.write_surface_value(node, key, value);
        }
    }

    /// Makes the node of a layer-shell surface: a `surface` of role `layer` with what the client asked for, unparented
    /// like a window so the shell may place it. The compositor draws nothing for it: a layer surface's buffer is not in
    /// the scene, so the node carries the rectangle the surface is placed at, and the shell's write of that rectangle
    /// while `placement_override` is true is what the protocol engine configures it to.
    pub fn add_layer_node(&mut self, info: &LayerNodeInfo) -> Option<NodeId> {
        let id = self.node_ids.allocate()?;
        let mut spec = NodeSpec::new(Rect::new(0, 0, 0, 0));
        spec.input = false;
        self.scene.insert(id, spec).ok()?;
        let _ = self.scene.set_class(id, Some("surface".to_owned()));
        let index = self.scene.root_ids().iter().position(|root| *root == id).unwrap_or(0);
        self.tree_changes.push(TreeChange::Created {
            node: id,
            class: "surface".to_owned(),
            index,
        });
        self.write_surface_value(id, "role", TreeValue::Text("layer".to_owned()));
        self.publish_layer_info(id, info);
        self.layer_nodes.insert(
            id,
            LayerNode {
                published: [f64::NAN; 4],
            },
        );
        Some(id)
    }

    /// Publishes the parts of a layer surface's request that changed.
    fn publish_layer_info(&mut self, node: NodeId, info: &LayerNodeInfo) {
        self.publish_surface_value(node, "mapped", TreeValue::Bool(info.mapped));
        self.publish_surface_value(node, "namespace", TreeValue::Text(info.namespace.clone()));
        self.publish_surface_value(node, "layer", TreeValue::Text(info.layer.to_owned()));
        self.publish_surface_value(node, "anchor", TreeValue::ListText(info.anchor.clone()));
        self.publish_surface_value(node, "margin", TreeValue::ListDouble(info.margin.to_vec()));
        self.publish_surface_value(node, "requested_size", TreeValue::ListDouble(info.requested.to_vec()));
        self.publish_surface_value(node, "exclusive_zone", TreeValue::Double(info.zone));
    }

    /// Brings a layer surface's node to what the surface asked for and, unless the shell places it (`shell_places`), to
    /// the rectangle the protocol put it at. A rectangle the shell wrote that the compositor does not honour (the shell
    /// is not placing the surface, or the session is locked) is written back, and published again so the shell sees it.
    pub fn update_layer_node(&mut self, node: NodeId, info: &LayerNodeInfo, rect: Rect, shell_places: bool) {
        let Some(mut layer) = self.layer_nodes.get(&node).copied() else {
            return;
        };
        self.publish_layer_info(node, info);
        if !shell_places {
            let held = self.scene.transform(node).copied();
            let bounds = self.scene.bounds(node);
            let wanted = Rect::new(rect.x, rect.y, rect.width, rect.height);
            let shown = [
                held.map_or(f64::NAN, |held| held.base.offset.x),
                held.map_or(f64::NAN, |held| held.base.offset.y),
                bounds.map_or(f64::NAN, |bounds| f64::from(bounds.width)),
                bounds.map_or(f64::NAN, |bounds| f64::from(bounds.height)),
            ];
            if shown != layer.published {
                layer.published = [f64::NAN; 4];
            }
            if let Some(mut held) = held {
                held.base.offset = Vec3::new(f64::from(wanted.x), f64::from(wanted.y), held.base.offset.z);
                if self.scene.transform(node) != Some(&held) {
                    let _ = self.scene.set_transform(node, held);
                }
            }
            if bounds != Some(Rect::from_size(Size::new(wanted.width, wanted.height))) {
                let _ = self
                    .scene
                    .set_bounds(node, Rect::from_size(Size::new(wanted.width, wanted.height)));
            }
        }
        let values = if shell_places {
            // The shell's own rectangle stands: what it wrote is what it reads.
            self.layer_shown(node)
        } else {
            [
                f64::from(rect.x),
                f64::from(rect.y),
                f64::from(rect.width),
                f64::from(rect.height),
            ]
        };
        for (index, (key, value)) in ["x", "y", "width", "height"].into_iter().zip(values).enumerate() {
            if layer.published[index] == value {
                continue;
            }
            layer.published[index] = value;
            self.tree_changes.push(TreeChange::Set {
                node,
                key: key.to_owned(),
                value: TreeValue::Double(value),
            });
        }
        self.layer_nodes.insert(node, layer);
    }

    /// The `[x, y, width, height]` a layer surface's node holds now.
    fn layer_shown(&self, node: NodeId) -> [f64; 4] {
        let offset = self.scene.transform(node).map_or((0.0, 0.0), |transform| {
            (transform.base.offset.x, transform.base.offset.y)
        });
        let size = self
            .scene
            .bounds(node)
            .map_or((0, 0), |bounds| (bounds.width, bounds.height));
        [offset.0, offset.1, f64::from(size.0), f64::from(size.1)]
    }

    /// Removes a layer surface's node, recording its destruction for the tree.
    pub fn remove_layer_node(&mut self, node: NodeId) {
        if self.layer_nodes.remove(&node).is_some() {
            self.drop_node(node);
        }
    }

    /// What the shell wrote on a layer surface's node while it places the surface, or `None` when it does not: no shell
    /// holds the tree, or `placement_override` is not true. The protocol engine adds the session's lock to that.
    #[must_use]
    pub fn layer_override(&self, node: NodeId) -> Option<LayerOverride> {
        if !self.layer_nodes.contains_key(&node) || !self.surface_placed(node) {
            return None;
        }
        let shown = self.layer_shown(node);
        let whole = |value: f64| value.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
        let size = |value: f64| value.round().clamp(0.0, f64::from(u32::MAX)) as u32;
        let reserve = match self.scene.value(node, "reserve") {
            Some(TreeValue::ListDouble(insets)) if insets.len() == 4 => Some(gnoblin_types::Insets::new(
                whole(insets[0]),
                whole(insets[1]),
                whole(insets[2]),
                whole(insets[3]),
            )),
            _ => None,
        };
        Some(LayerOverride {
            rect: Rect::new(whole(shown[0]), whole(shown[1]), size(shown[2]), size(shown[3])),
            reserve,
        })
    }
}
