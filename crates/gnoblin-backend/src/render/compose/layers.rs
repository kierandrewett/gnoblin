//! What the composer draws and answers for a layer-shell surface (`zwlr_layer_shell_v1`): its node, its pixels and its place in
//! the frame.
//!
//! The protocol engine (`gnoblin-wayland`) owns the protocol: it computes the rectangle from the anchors, margins and size,
//! configures the client and reserves the zone. The composer owns the scene, so what it keeps here is what a frame needs:
//!
//! - **A node per surface**, unparented like a window (class `surface`, `role` `layer`), made when the client creates the
//!   surface. The node's offset and bounds are the rectangle the surface is placed at: the protocol's, or the shell's while it
//!   places the surface (`placement_override`). The node carries the surface's subsurfaces and popups as children, so they are
//!   placed against it, move with it and are damaged with it, exactly as a window's are ([`TreeOwner::Layer`]).
//! - **Its pixels**, the buffer the client last committed ([`Composer::set_layer_content`]), drawn at the node's origin at
//!   the size the buffer has. A commit damages the surface's rectangle, so a frame is made for it.
//! - **A place in the frame's order.** The protocol names four layers, bottom first: `background`, `bottom`, `top`, `overlay`.
//!   `background` and `bottom` are drawn under every window and every node of the shell's own; `top` and `overlay` above them.
//!   Surfaces in one layer are drawn in the order the shell's `stack` gives and then in the order they were created.
//!   A popup of a layer surface is drawn above every layer, so a menu of a bar on the `bottom` layer is not hidden behind the
//!   windows it opens over ([`Composer::compose`]).
//!
//! # Failure modes
//!
//! 1. **A surface that never mapped is drawn.** A surface has pixels once its client commits a buffer, and none before:
//!    `a_layer_surface_with_no_buffer_is_not_drawn`.
//! 2. **A surface that goes leaves its pixels on the output.** Removing its node, or unmapping it, damages the rectangle it
//!    covered: `a_layer_surface_that_goes_damages_what_it_covered`.
//! 3. **A layer is drawn in the wrong place in the stack.** The order of the four layers is the protocol's whatever order the
//!    surfaces were created in, and the shell's `stack` reorders only inside one layer: `layers_are_drawn_bottom_first_around_the_windows`.
//! 4. **A hidden surface is drawn.** A surface the shell hid, and what hangs from it, is neither drawn nor picked:
//!    `a_layer_surface_the_shell_hides_is_not_drawn`.

use super::*;

/// The layers of the layer-shell protocol, bottom first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LayerBand {
    /// The bottom of everything: a wallpaper.
    Background,
    /// Above the background and under the windows: a desktop widget or an icon grid.
    Bottom,
    /// Above the windows: a bar, a dock.
    Top,
    /// Above everything the protocol can draw: a lock prompt, a launcher.
    Overlay,
}

impl LayerBand {
    /// The layer's name as the protocol and the node's `layer` property give it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::Bottom => "bottom",
            Self::Top => "top",
            Self::Overlay => "overlay",
        }
    }

    /// Whether the layer is drawn above the windows.
    #[must_use]
    pub const fn above_windows(self) -> bool {
        matches!(self, Self::Top | Self::Overlay)
    }
}

/// What a layer-shell surface publishes of itself to its node: what the client asked for and where it is.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerNodeInfo {
    /// The namespace the client named.
    pub namespace: String,
    /// The layer the client asked for.
    pub layer: LayerBand,
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

/// A layer surface's node, what was last published of its place, and what it draws.
#[derive(Clone, Debug)]
pub(super) struct LayerNode {
    /// The `[x, y, width, height]` last published; NaN before the first.
    pub(super) published: [f64; 4],
    /// The layer the surface is in.
    pub(super) band: LayerBand,
    /// The crop and size the client's last commit has, once it has committed a buffer.
    pub(super) content: Option<Content>,
    /// The pixels `content` crops.
    pub(super) source: Option<SurfaceSource>,
    /// The subsurfaces and popups of the surface, and their nodes.
    pub(super) tree: SurfaceTree,
}

impl Composer {
    /// Makes the node of a layer-shell surface: a `surface` of role `layer` with what the client asked for, unparented
    /// like a window so the shell may place it. The node is where the surface is drawn and what its subsurfaces and popups
    /// hang from, so it takes no hits itself: the pointer reaches the surface through the protocol engine's own lookup,
    /// which knows input regions.
    pub fn add_layer_node(&mut self, info: &LayerNodeInfo) -> Option<NodeId> {
        let Some(id) = self.node_ids.allocate() else {
            log::warn!(
                area = "render",
                namespace = %info.namespace,
                "no compositor node id is left; a layer surface is not drawn"
            );
            return None;
        };
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
                band: info.layer,
                content: None,
                source: None,
                tree: SurfaceTree::default(),
            },
        );
        Some(id)
    }

    /// Publishes the parts of a layer surface's request that changed.
    fn publish_layer_info(&mut self, node: NodeId, info: &LayerNodeInfo) {
        self.publish_surface_value(node, "mapped", TreeValue::Bool(info.mapped));
        self.publish_surface_value(node, "namespace", TreeValue::Text(info.namespace.clone()));
        self.publish_surface_value(node, "layer", TreeValue::Text(info.layer.name().to_owned()));
        self.publish_surface_value(node, "anchor", TreeValue::ListText(info.anchor.clone()));
        self.publish_surface_value(node, "margin", TreeValue::ListDouble(info.margin.to_vec()));
        self.publish_surface_value(node, "requested_size", TreeValue::ListDouble(info.requested.to_vec()));
        self.publish_surface_value(node, "exclusive_zone", TreeValue::Double(info.zone));
    }

    /// Brings a layer surface's node to what the surface asked for and, unless the shell places it (`shell_places`), to
    /// the rectangle the protocol put it at. A rectangle the shell wrote that the compositor does not honour (the shell
    /// is not placing the surface, or the session is locked) is written back, and published again so the shell sees it.
    pub fn update_layer_node(&mut self, node: NodeId, info: &LayerNodeInfo, rect: Rect, shell_places: bool) {
        let Some(layer) = self.layer_nodes.get_mut(&node) else {
            return;
        };
        let mut published = layer.published;
        layer.band = info.layer;
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
            if shown != published {
                published = [f64::NAN; 4];
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
            if published[index] == value {
                continue;
            }
            published[index] = value;
            self.tree_changes.push(TreeChange::Set {
                node,
                key: key.to_owned(),
                value: TreeValue::Double(value),
            });
        }
        if let Some(layer) = self.layer_nodes.get_mut(&node) {
            layer.published = published;
        }
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

    /// Removes a layer surface's node, and the nodes of its subsurfaces and popups with it, recording its destruction for
    /// the tree. The rectangle it covered is damaged, so the pixels it drew go from the next frame.
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

    /// Records what a layer surface's client committed: the crop to draw and where its pixels come from, or `None` when the
    /// surface has no buffer. The surface is drawn at its node's origin at the size the buffer has, so a client that has
    /// not yet drawn at the size it was configured with shows what it last drew.
    ///
    /// A commit always damages the rectangle the surface covers: the client may have drawn into the same buffer, and nothing
    /// in the handle says whether its pixels changed. The node's draw extent is the buffer's rectangle, so a buffer that is
    /// not the size of the node's rectangle is damaged where it is drawn, and a surface that unmaps damages what it covered.
    pub fn set_layer_content(&mut self, node: NodeId, content: Option<(Content, SurfaceSource)>) {
        let Some(layer) = self.layer_nodes.get_mut(&node) else {
            return;
        };
        let extent = content.as_ref().map(|(content, _)| Rect::from_size(content.size));
        (layer.content, layer.source) = match content {
            Some((content, source)) => (Some(content), Some(source)),
            None => (None, None),
        };
        let _ = self.scene.set_draw_extent(node, extent);
        // A View that draws this surface reads its pixels again.
        let _ = self.scene.mark_dirty(node);
    }

    /// The layer surfaces that are drawn, bottom first: in the order of the protocol's layers, then of the shell's `stack`
    /// (read only while it places the surface), then of creation. A surface with no buffer, one the shell hid and one the
    /// shell made fully transparent is not among them: nothing of it is on the output, so nothing of it is picked.
    #[must_use]
    pub fn layer_stack(&self) -> Vec<NodeId> {
        let mut drawn: Vec<(LayerBand, i64, NodeId)> = self
            .layer_nodes
            .iter()
            .filter(|(node, layer)| layer.content.is_some() && self.scene.drawn(**node))
            .map(|(node, layer)| (layer.band, self.surface_side(*node, false).1, *node))
            .collect();
        drawn.sort_unstable();
        drawn.into_iter().map(|(_, _, node)| node).collect()
    }

    /// The layer a layer surface's node is in, or `None` when `node` is not one.
    #[must_use]
    pub fn layer_band(&self, node: NodeId) -> Option<LayerBand> {
        self.layer_nodes.get(&node).map(|layer| layer.band)
    }

    /// Whether the shell hid a layer surface's node (`visible` false). A client's frame callbacks are not answered for a
    /// surface nobody can see, and nothing of it is picked.
    #[must_use]
    pub fn layer_hidden(&self, node: NodeId) -> bool {
        self.scene.visible(node) == Some(false)
    }

    /// `point` of the output plane in the plane of a layer surface's node, where the surface's own pixels start at the
    /// origin: the point a pointer over the surface is at in it. `None` for a node that has gone, or one the point cannot be
    /// mapped into.
    #[must_use]
    pub fn layer_local_point(&self, node: NodeId, point: PointF) -> Option<PointF> {
        self.layer_nodes.get(&node)?;
        self.scene.local_point(node, point)
    }

    /// The rectangle a layer surface's node covers on the output plane, rounded to whole pixels: where a shell that holds
    /// the tree put it, or the protocol did. `None` for a node that has gone.
    #[must_use]
    pub fn layer_frame(&self, node: NodeId) -> Option<Rect> {
        self.layer_nodes.get(&node)?;
        self.scene.content_bounds(node).map(RectF::to_pixel)
    }

    /// Adds to `frame` the layers of the layer surfaces `bands` admits, bottom first, each with its subsurfaces and without
    /// its popups. The caller names the bands that go under the windows and the ones that go over them.
    pub(super) fn push_layer_surfaces(
        &self,
        bands: impl Fn(LayerBand) -> bool,
        output: Rect,
        frame: &mut OutputFrame,
        spatial: &mut bool,
    ) {
        for node in self.layer_stack() {
            let Some(layer) = self.layer_nodes.get(&node).filter(|layer| bands(layer.band)) else {
                continue;
            };
            let Some(paint) = self.layer_paint(node, layer, output) else {
                continue;
            };
            self.push_children(&paint, node, true, true, frame, spatial);
            if let (Some(content), Some(source)) = (layer.content, layer.source.clone()) {
                let transform = self.scene.transform(node).copied().unwrap_or(VisualTransform::IDENTITY);
                let (effects, _) = self.scene.effects(node).map(Effects::split_borders).unwrap_or_default();
                let drawn = Drawn {
                    source,
                    content,
                    local: Some(Rect::from_size(content.size)),
                    opacity: transform.opacity,
                    corner_radius: transform.corner_radius,
                    silhouette: None,
                    effects,
                };
                self.push_surface(&paint, node, drawn, frame, spatial);
            }
            self.push_children(&paint, node, false, true, frame, spatial);
        }
    }

    /// Adds to `frame` the popups of every layer surface, in the order of the layers. They are drawn above every layer, over
    /// the windows and the surfaces of the top and overlay layers alike: a menu that opens from a bar belongs over what the
    /// bar opens it over.
    pub(super) fn push_layer_popups(&self, output: Rect, frame: &mut OutputFrame, spatial: &mut bool) {
        for node in self.layer_stack() {
            let Some(layer) = self.layer_nodes.get(&node) else {
                continue;
            };
            if let Some(paint) = self.layer_paint(node, layer, output) {
                self.push_popups(&paint, node, frame, spatial);
            }
        }
    }

    /// What the layers of one layer surface's tree are drawn with, or `None` when the surface is not drawn.
    fn layer_paint<'a>(&self, node: NodeId, layer: &'a LayerNode, output: Rect) -> Option<Paint<'a>> {
        let transform = self.scene.transform(node)?;
        if !self.scene.drawn(node) {
            return None;
        }
        Some(Paint {
            output,
            node,
            tree: &layer.tree,
            tint: transform.tint,
            // The software renderer cannot rotate a layer: the surface is left out, as a window is.
            rotated: transform.base.rotation_degrees.z != 0.0,
        })
    }
}
