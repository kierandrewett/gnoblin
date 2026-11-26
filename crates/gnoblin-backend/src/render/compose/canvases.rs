//! Canvases: the picture of each Canvas node, painted from its `content`, and the layers that draw it.
//!
//! A move out of `compose.rs`: these are methods of [`Composer`] and read its fields.

use super::*;

/// A Canvas node's rendered picture: what it was painted from, and the image, or `None` when its content paints
/// nothing (empty, a zero-size node, or too large to draw).
///
/// The picture is kept until the node's `content` is written again, its size changes, or the scale of the output it
/// is on does. `revision` is the scene's count of byte writes to the node, so a rewrite of equal bytes repaints too:
/// comparing bytes every frame would cost more than the rare repeated write.
#[derive(Debug)]
pub(super) struct CanvasPicture {
    pub(super) revision: u64,
    /// The node's size in logical pixels when it was painted.
    pub(super) size: Size,
    /// Physical pixels per logical pixel the picture was painted at. The image is `size * scale` pixels, and the
    /// renderer draws it over the node's logical rectangle.
    pub(super) scale: Scale,
    pub(super) image: Option<Arc<Image>>,
    /// What the picture is, and that it was replaced each time the canvas was painted.
    pub(super) track: ContentTrack,
}

impl Composer {
    /// Paints each Canvas node whose picture is stale: it has none yet, its `content` was written since, or its size
    /// changed. The scene already damaged the node's rectangle when the content or the size changed, so a repaint
    /// adds no damage of its own.
    pub(super) fn render_canvases(&mut self) {
        let canvases = self.canvas_nodes();
        self.canvases.retain(|canvas, _| canvases.contains(canvas));
        for canvas in canvases {
            let (Some(bounds), Some(revision)) = (self.scene.bounds(canvas), self.scene.content_revision(canvas))
            else {
                continue;
            };
            let size = Size::new(bounds.width, bounds.height);
            let scale = self.canvas_scale(canvas);
            if self
                .canvases
                .get(&canvas)
                .is_some_and(|picture| picture.revision == revision && picture.size == size && picture.scale == scale)
            {
                continue;
            }
            let image = self.paint_canvas(canvas, size, scale);
            // The picture is new pixels, under the key the canvas has had since it was made.
            let mut track = self
                .canvases
                .get(&canvas)
                .map_or_else(ContentTrack::new, |picture| picture.track.clone());
            track.replaced();
            self.canvases.insert(
                canvas,
                CanvasPicture {
                    revision,
                    size,
                    scale,
                    image,
                    track,
                },
            );
        }
    }

    /// The scale a canvas is painted at: the largest among the outputs it is on, so it is sharp on each and only
    /// shrunk on a lower-density neighbour. A canvas on no output is painted at one.
    pub(super) fn canvas_scale(&self, canvas: NodeId) -> Scale {
        let placed = self.scene.absolute_bounds(canvas);
        let mut best: Option<Scale> = None;
        for (output, rect) in self.scene.outputs() {
            if placed.is_some_and(|placed| placed.intersection(*rect).is_none()) {
                continue;
            }
            let scale = self.output_scales.get(output).copied().unwrap_or(Scale::IDENTITY);
            if best.is_none_or(|best| scale.as_f64() > best.as_f64()) {
                best = Some(scale);
            }
        }
        best.unwrap_or(Scale::IDENTITY)
    }

    /// The image of a canvas's `content` at `size` logical pixels drawn at `scale`, or `None` when it has no content
    /// or draws nothing. A scale whose picture would be too large is painted at one, and a canvas too large even then
    /// draws nothing.
    pub(super) fn paint_canvas(&self, canvas: NodeId, size: Size, scale: Scale) -> Option<Arc<Image>> {
        let bytes = self.scene.bytes(canvas, "content").filter(|bytes| !bytes.is_empty())?;
        let list = match decode(bytes) {
            Ok(list) => list,
            Err(error) => {
                // The tree refuses such a value when it is written, so this is a bug if it happens.
                log::warn!(area = "render", %canvas, %error, "a canvas holds content that does not decode");
                return None;
            }
        };
        let painter = self.painter.get();
        let rendered = painter
            .paint_scaled(&list, size.width, size.height, scale.as_f64())
            .or_else(|| painter.paint_scaled(&list, size.width, size.height, 1.0))?;
        for name in &rendered.missing_icons {
            log::debug!(area = "render", %canvas, icon = %name, "a canvas names an icon that is not built in");
        }
        let picture = rendered.picture;
        Image::new(
            Size::new(picture.width, picture.height),
            picture.width as usize * 4,
            picture.data,
        )
        .map(Arc::new)
    }

    /// Every Canvas node in the scene.
    pub(super) fn canvas_nodes(&self) -> BTreeSet<NodeId> {
        self.scene
            .node_ids()
            .filter(|node| self.scene.class(*node) == Some("canvas"))
            .collect()
    }

    /// The picture a Canvas node was last painted as, or `None` when it has none to draw.
    #[must_use]
    pub fn canvas_picture(&self, canvas: NodeId) -> Option<Arc<Image>> {
        self.canvases.get(&canvas)?.image.clone()
    }

    /// The canvases that are painted before the window they hang under, by that window's node: each canvas child of a
    /// window node whose `placement` is `under` (it is `behind` in the scene) and everything beneath it, in paint order.
    /// A canvas is otherwise painted after its parent, in front of the window's content.
    pub(super) fn canvases_under_windows(&self) -> BTreeMap<NodeId, Vec<NodeId>> {
        let mut under = BTreeMap::new();
        for entry in self.windows.values() {
            let Some(window_node) = entry.node else {
                continue;
            };
            let mut nodes = Vec::new();
            let mut pending: Vec<NodeId> = self
                .scene
                .children(window_node)
                .unwrap_or(&[])
                .iter()
                .rev()
                .copied()
                .filter(|child| self.scene.behind(*child) == Some(true) && self.scene.class(*child) == Some("canvas"))
                .collect();
            while let Some(next) = pending.pop() {
                nodes.push(next);
                pending.extend(self.scene.children(next).unwrap_or(&[]).iter().rev());
            }
            if !nodes.is_empty() {
                under.insert(window_node, nodes);
            }
        }
        under
    }

    /// Adds the layer of a Canvas node's picture, where the scene places the node now and at the opacity and
    /// corner radius it composes to.
    pub(super) fn push_canvas(&self, canvas: NodeId, output: Rect, frame: &mut OutputFrame, spatial: &mut bool) {
        let Some(image) = self.canvases.get(&canvas).and_then(|picture| picture.image.as_ref()) else {
            return;
        };
        let (Some(placed), Some(transform), Some(opacity)) = (
            self.scene.content_bounds(canvas),
            self.scene.transform(canvas),
            self.scene.composed_opacity(canvas),
        ) else {
            return;
        };
        let dst = Rect::new(
            placed.x.round() as i32 - output.x,
            placed.y.round() as i32 - output.y,
            placed.width.round() as u32,
            placed.height.round() as u32,
        );
        let model = self.model(canvas, output);
        let on_output = match model {
            Some(_) => self
                .scene
                .absolute_bounds(canvas)
                .is_some_and(|bounds| bounds.intersection(output).is_some()),
            None => dst.intersection(Rect::new(0, 0, output.width, output.height)).is_some(),
        };
        if !on_output {
            return;
        }
        if model.is_some() && !*spatial {
            frame.push_damage(Rect::new(0, 0, output.width, output.height));
            *spatial = true;
        }
        if transform.base.rotation_degrees.z != 0.0 {
            log::debug!(area = "render", %canvas, "a rotated canvas is not drawn by the software renderer");
            return;
        }
        let size = image.size();
        // A decoration child of a rounded window (its titlebar) takes the window's radius and is clipped to the
        // window's frame, so the bar and the content have one set of corners and not two.
        // A layer over or under the window (`placement`) is not part of the frame and is not rounded to it.
        let inherited = (transform.corner_radius <= 0.0 && model.is_none() && self.scene.framed(canvas) != Some(false))
            .then(|| self.scene.parent(canvas))
            .flatten()
            .filter(|parent| self.window_of_node(*parent).is_some())
            .and_then(|parent| self.window_shape(parent, output))
            .filter(|shape| shape.radius > 0.0);
        let (corner_radius, silhouette) = match inherited {
            Some(shape) => (shape.radius, Some(shape.rect)),
            None => (transform.corner_radius, None),
        };
        frame.push_layer(Layer::Surface(Surface {
            source: SurfaceSource::Snapshot(Arc::clone(image)),
            src: RectF {
                x: 0.0,
                y: 0.0,
                width: f64::from(size.width),
                height: f64::from(size.height),
            },
            dst,
            transform: Transform::Normal,
            opacity,
            tint: Color::new(1.0, 1.0, 1.0, 1.0),
            corner_radius,
            silhouette,
            effects: self.scene.effects(canvas).cloned().unwrap_or_else(Effects::new),
            model,
            secure: self.secure_scope.contains(&canvas),
            content: self.canvases.get(&canvas).map(|picture| picture.track.key()),
        }));
    }
}
