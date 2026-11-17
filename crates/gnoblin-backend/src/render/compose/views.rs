//! Views: a View node's picture, its camera and its rig, the pictures of the windows it shows, and the
//! pointer ray that goes through it.
//!
//! A move out of `compose.rs`: these are methods of [`Composer`] and read its fields.

use super::*;

/// The orthographic camera's distance from the image plane, in logical pixels.
pub(super) const VIEW_ORTHO_DEPTH: f64 = 100.0;

/// The near and far eye depths a View draws between when its camera sets neither.
pub(super) const VIEW_NEAR: f64 = 0.1;

pub(super) const VIEW_FAR: f64 = 10_000.0;

/// The vertical field of view of a perspective camera that sets none, in degrees.
pub(super) const VIEW_FOV: f64 = 60.0;

/// The scene unit of a camera that sets no `metres`, in metres: a millimetre, as the schema's default.
pub(super) const VIEW_METRES: f64 = 0.001;

/// The distance between a stereo camera's eyes when it sets none, in metres.
pub(super) const VIEW_IPD: f64 = 0.063;

/// How many Views a pointer ray may pass into one after another, so Views that show each other end the ray.
pub(super) const VIEW_DEPTH: u8 = 4;

/// Where a pointer ray lands once the Views it passes into have had it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Landing {
    /// A window, with the point in the window node's local space: the client's.
    Window(WindowId, PointF),
    /// A node that is not a window, with the point in its local space: the shell's.
    Node(NodeId, PointF),
    /// Nothing: the ray went into a View's picture where nothing takes input.
    Nowhere,
}

/// A View node's rendered picture: the size it was drawn at, the image of its source
/// subtree (or `None` when it has no source or the source cannot be drawn), and the view
/// clock's reading when it was drawn, which `fps` paces the next draw from.
///
/// The image is what the display shows. A capture never reads it when the source holds a `secure` node: it reads
/// `redacted`, the same subtree drawn with each secure node replaced the way an output capture replaces a secure
/// layer ([`SecureTreatment`]). `secure` names those nodes as they were when the picture was drawn, so a node that
/// becomes secure, or stops being, draws the View again at once.
#[derive(Debug)]
pub(super) struct ViewPicture {
    pub(super) size: Size,
    /// The picture on the CPU, when the composer drew it there.
    pub(super) image: Option<Arc<Image>>,
    /// The picture a capture reads when the source holds a secure node, or `None` when it holds none and `image` is
    /// the capture's picture too.
    pub(super) redacted: Option<Arc<Image>>,
    /// The scene the GPU draws the picture from, when the active renderer draws a View's subtree itself. Then
    /// `image` and `redacted` are `None`: the picture is a texture, and the pixels reach the CPU only when a capture
    /// reads them (`Backend::read_view`).
    pub(super) scene: Option<Arc<ViewScene>>,
    /// The nodes of the source subtree a capture may not show: each node marked secure, and everything below it.
    pub(super) secure: BTreeSet<NodeId>,
    pub(super) drawn_at_ms: u64,
}

/// What [`Composer::draw_view`] made of a View: its picture on the CPU, the capture's picture when a node is secure,
/// or the scene a GPU renderer draws it from. All three are `None` for a View that draws nothing.
#[derive(Default)]
pub(super) struct DrawnView {
    pub(super) image: Option<Arc<Image>>,
    pub(super) redacted: Option<Arc<Image>>,
    pub(super) scene: Option<Arc<ViewScene>>,
}

/// The pixels of one node of a View's source, in the form a GPU renderer samples.
pub(super) struct ViewPixels {
    pub(super) source: SurfaceSource,
    pub(super) size: Size,
    /// The part of the picture inside the node's rectangle, for a window's own surface when its buffer is not exactly
    /// the window geometry; `None` draws the whole picture over the node's rectangle.
    pub(super) crop: Option<ViewCrop>,
}

/// What one eye's walk of the source subtree draws with.
pub(super) struct ViewEmit<'a> {
    /// The root of the source subtree, the world the camera sits in.
    pub(super) root: NodeId,
    /// The camera: the subtree's world to the eye's pixels and clip depth.
    pub(super) projection: &'a Mat4,
    /// The pixels of each node that has any.
    pub(super) pictures: &'a BTreeMap<NodeId, ViewPixels>,
    /// The nodes to replace with `treatment`, for the capture variant; `None` for the display's.
    pub(super) secure: Option<&'a BTreeSet<NodeId>>,
    pub(super) treatment: SecureTreatment,
    /// The picture a masked node draws.
    pub(super) mask: &'a Arc<Image>,
}

impl ViewEmit<'_> {
    /// The layer that draws `node`'s pixels over `bounds`, placed by `model` at `opacity`, or `None` for a node with
    /// no pixels, no extent, or one the capture omits.
    fn layer(&self, node: NodeId, bounds: Rect, opacity: f64, model: Model) -> Option<Layer> {
        if bounds.width == 0 || bounds.height == 0 {
            return None;
        }
        let pixels = self.pictures.get(&node)?;
        let masked = self.secure.is_some_and(|secure| secure.contains(&node));
        let (source, src, dst, model) = if masked {
            let source = match self.treatment {
                SecureTreatment::Mask => SurfaceSource::Snapshot(Arc::clone(self.mask)),
                SecureTreatment::Omit => return None,
            };
            (
                source,
                RectF::new(0.0, 0.0, 1.0, 1.0),
                Rect::new(0, 0, bounds.width, bounds.height),
                model,
            )
        } else {
            match pixels.crop {
                None => (
                    pixels.source.clone(),
                    RectF::new(0.0, 0.0, f64::from(pixels.size.width), f64::from(pixels.size.height)),
                    Rect::new(0, 0, bounds.width, bounds.height),
                    model,
                ),
                // The picture covers part of the node's rectangle: its layer starts where that part does.
                Some(crop) => (
                    pixels.source.clone(),
                    crop.src,
                    Rect::new(0, 0, crop.dst.width, crop.dst.height),
                    Model {
                        matrix: model
                            .matrix
                            .mul(&Mat4::translation(f64::from(crop.dst.x), f64::from(crop.dst.y), 0.0)),
                        ..model
                    },
                ),
            }
        };
        Some(Layer::Surface(Surface {
            source,
            src,
            dst,
            transform: Transform::Normal,
            opacity: opacity.min(1.0) as f32,
            tint: Color::WHITE,
            corner_radius: 0.0,
            silhouette: None,
            effects: Effects::new(),
            model: Some(model),
            secure: false,
        }))
    }
}

/// The part of a window's buffer a View draws, and where it lies in the node's plane: the buffer's rectangle cut to the
/// window geometry, which is the node's own rectangle.
#[derive(Clone, Copy, Debug)]
pub(super) struct ViewCrop {
    /// The rectangle of the buffer's pixels to sample.
    pub(super) src: RectF,
    /// The rectangle of the node's plane it covers, in logical pixels.
    pub(super) dst: Rect,
}

/// The crop of a buffer of `pixels` that lies inside the window geometry `frame`, when the buffer, which spans `local` in
/// the node's plane, is not exactly it. `None` when none of the buffer is inside.
pub(super) fn view_crop(local: Rect, frame: Rect, pixels: Size) -> Option<ViewCrop> {
    let inside = local.intersection(frame)?;
    let (scale_x, scale_y) = (
        f64::from(pixels.width) / f64::from(local.width.max(1)),
        f64::from(pixels.height) / f64::from(local.height.max(1)),
    );
    Some(ViewCrop {
        src: RectF::new(
            f64::from(inside.x - local.x) * scale_x,
            f64::from(inside.y - local.y) * scale_y,
            f64::from(inside.width) * scale_x,
            f64::from(inside.height) * scale_y,
        ),
        dst: inside,
    })
}

/// A window's buffer cut to the window geometry `frame` and padded to its size with transparent pixels where the
/// buffer does not reach: the picture a View stretches over the window's node. `local` is where the buffer lies in
/// the node's plane.
pub(super) fn geometry_crop(image: &Image, local: Rect, frame: Rect) -> Image {
    let pixels = image.size();
    let (scale_x, scale_y) = (
        f64::from(pixels.width) / f64::from(local.width.max(1)),
        f64::from(pixels.height) / f64::from(local.height.max(1)),
    );
    let width = ((f64::from(frame.width) * scale_x).round() as u32).max(1);
    let height = ((f64::from(frame.height) * scale_y).round() as u32).max(1);
    let stride = width as usize * 4;
    let mut data = vec![0u8; stride * height as usize];
    if let Some(inside) = local.intersection(frame) {
        let to_x = (f64::from(inside.x - frame.x) * scale_x).round() as usize;
        let to_y = (f64::from(inside.y - frame.y) * scale_y).round() as usize;
        let from_x = (f64::from(inside.x - local.x) * scale_x).round() as usize;
        let from_y = (f64::from(inside.y - local.y) * scale_y).round() as usize;
        let columns = ((f64::from(inside.width) * scale_x).round() as usize)
            .min(width as usize - to_x.min(width as usize))
            .min((pixels.width as usize).saturating_sub(from_x));
        let rows = ((f64::from(inside.height) * scale_y).round() as usize)
            .min(height as usize - to_y.min(height as usize))
            .min((pixels.height as usize).saturating_sub(from_y));
        for row in 0..rows {
            let from = (from_y + row) * image.stride() + from_x * 4;
            let to = (to_y + row) * stride + to_x * 4;
            if let (Some(source), Some(target)) = (
                image.data().get(from..from + columns * 4),
                data.get_mut(to..to + columns * 4),
            ) {
                target.copy_from_slice(source);
            }
        }
    }
    Image::new(Size::new(width, height), stride, data).unwrap_or_else(|| image.clone())
}

/// Whether a node's plane, through a camera, lands as a plane a picture can show: the matrix's columns for `u`, `v`
/// and the origin, over the rows for the two pixel coordinates and the divisor, are finite and not singular. The CPU
/// rig drops a node whose homography is not invertible together with its whole subtree, and so does the scene.
pub(super) fn projects_to_a_plane(matrix: &Mat4) -> bool {
    let m = &matrix.m;
    let at = |row: usize, column: usize| m[[0, 1, 3][row]][[0, 1, 3][column]];
    let det = at(0, 0) * (at(1, 1) * at(2, 2) - at(1, 2) * at(2, 1))
        - at(0, 1) * (at(1, 0) * at(2, 2) - at(1, 2) * at(2, 0))
        + at(0, 2) * (at(1, 0) * at(2, 1) - at(1, 1) * at(2, 0));
    matrix.is_finite() && det.is_finite() && det != 0.0
}

/// The size of the picture a surface's source holds, or `None` for one with no extent or no pixels a GPU samples: a
/// buffer that is not a packed 32-bit shared-memory buffer is not one, as the CPU view cannot read it either.
pub(super) fn source_size(source: &SurfaceSource) -> Option<Size> {
    let size = match source {
        SurfaceSource::Buffer(buffer) => {
            let shape = shm::shape(buffer).ok()?;
            Size::new(shape.width, shape.height)
        }
        SurfaceSource::Snapshot(image) => image.size(),
        SurfaceSource::Dmabuf(dmabuf) => {
            let size = dmabuf.size();
            Size::new(u32::try_from(size.w).ok()?, u32::try_from(size.h).ok()?)
        }
        SurfaceSource::View(_) => return None,
    };
    (size.width > 0 && size.height > 0).then_some(size)
}

/// The pixels of a surface's source as the view renderer reads them.
///
/// A GPU buffer (a DMA-BUF) has no CPU pixels, so `reader` reads it back where the buffer lives (a GL readback of the
/// imported texture). A backend that cannot, or a buffer the driver will not read, leaves the window masked as a
/// secure node is, rather than dropped: a View shows a grey rectangle where the window is and not a hole through which
/// whatever lies behind it shows. The first window that is masked is logged; the rest are not, because a View
/// redraws on every change.
pub(super) fn source_picture(source: &SurfaceSource, reader: &mut ReadDmabuf<'_>) -> Option<Image> {
    static LOGGED: std::sync::Once = std::sync::Once::new();
    match source {
        SurfaceSource::Buffer(buffer) => shm::readback(buffer).ok(),
        SurfaceSource::Snapshot(image) => Some(Image::clone(image)),
        SurfaceSource::Dmabuf(dmabuf) => reader(dmabuf).or_else(|| {
            LOGGED.call_once(|| {
                log::error!(
                    area = "render",
                    "a View captured a window that shows a GPU buffer, which the renderer cannot read; the window is masked"
                );
            });
            Some(secure_mask())
        }),
        // A window's buffer is never a View's picture: only the composer's own layer for a View names one.
        SurfaceSource::View(_) => None,
    }
}

/// A premultiplied pixel as straight alpha.
pub(super) fn straight_alpha(pixel: [u8; 4]) -> [u8; 4] {
    let alpha = u32::from(pixel[3]);
    if alpha == 0 || alpha == 255 {
        return pixel;
    }
    let lift = |channel: u8| ((u32::from(channel) * 255 + alpha / 2) / alpha).min(255) as u8;
    [lift(pixel[0]), lift(pixel[1]), lift(pixel[2]), pixel[3]]
}

impl Composer {
    /// Re-renders the View nodes whose picture is stale and damages the pixels each one
    /// redraws. A view is stale when it has no picture yet, changed size, or when it, or a
    /// node in its source or camera subtree, is in `dirty`, or when a node in its source became secure or stopped
    /// being.
    ///
    /// A View with `fps` set is drawn at most `fps` times a second: a source that changes inside the slot waits,
    /// however often it changes, and the view is drawn at the slot's end from the state it is in then. A View with
    /// no picture or one of another size is drawn at once, so a picture of the wrong size is never kept, and `fps`
    /// zero draws on every change. A change in what is secure is drawn at once as well, whatever `fps` says: a
    /// capture must not wait out a slot with pixels the user has since marked secure.
    pub(super) fn render_views(&mut self, dirty: &BTreeSet<NodeId>, reader: &mut ReadDmabuf<'_>) {
        let views = self.view_nodes();
        self.views.retain(|view, _| views.contains(view));
        self.paced_views.retain(|view| views.contains(view));
        for view in views {
            let Some(bounds) = self.scene.bounds(view) else {
                continue;
            };
            let size = Size::new(bounds.width, bounds.height);
            let inputs: Vec<NodeId> = self
                .scene
                .source(view)
                .into_iter()
                .chain(self.scene.camera(view))
                .flat_map(|root| self.subtree(root))
                .collect();
            let secure = self.secure_nodes(self.scene.source(view).into_iter().flat_map(|root| self.subtree(root)));
            let picture = self.views.get(&view).filter(|picture| picture.size == size);
            let secure_changed = picture.is_some_and(|picture| picture.secure != secure);
            let stale = picture.is_none()
                || secure_changed
                || dirty.contains(&view)
                || inputs.iter().any(|node| dirty.contains(node))
                || self.paced_views.contains(&view);
            if !stale {
                continue;
            }
            let fps = self.scene.unsigned(view, "fps").unwrap_or(0);
            if let (Some(picture), 1.., false) = (picture, fps, secure_changed) {
                let slot_ms = 1000u64.div_ceil(fps);
                if self.view_clock_ms.saturating_sub(picture.drawn_at_ms) < slot_ms {
                    self.paced_views.insert(view);
                    continue;
                }
            }
            self.paced_views.remove(&view);
            if !size_fits(size) && picture.is_none() {
                log::warn!(
                    area = "render",
                    %view,
                    width = size.width,
                    height = size.height,
                    "a View of this size has no picture: its sides must be at least one pixel and its size within the cap (`view_scene::size_fits`)"
                );
            }
            self.view_revision = self.view_revision.saturating_add(1);
            let drawn = self.draw_view(view, size, &secure, self.view_revision, reader);
            self.views.insert(
                view,
                ViewPicture {
                    size,
                    image: drawn.image,
                    redacted: drawn.redacted,
                    scene: drawn.scene,
                    secure,
                    drawn_at_ms: self.view_clock_ms,
                },
            );
            if let Some(bounds) = self.scene.absolute_bounds(view) {
                self.scene.damage_rect(bounds);
            }
        }
    }

    /// How a View's source subtree is drawn: the picture itself on the CPU, or the scene the renderer draws it from.
    ///
    /// The first picture is the image of the View's source subtree through its camera. It, the scene and the second
    /// picture are all `None` when the view has no source, the source is missing, the source contains the view
    /// itself, or the View's size is not one a picture can have ([`size_fits`]).
    ///
    /// The second picture is the one a capture reads when `secure` is not empty: the same subtree with every secure
    /// node's pixels replaced as an output capture replaces a secure layer. `Mask` draws a neutral grey in the node's
    /// place, `Omit` draws nothing there, so whatever lies beneath it in the source shows. It is `None` when no node
    /// is secure.
    ///
    /// When the active renderer draws a View on the GPU ([`Composer::set_gpu_views`]) the pictures stay `None` and
    /// the scene, which carries the same two variants, is what the renderer draws.
    pub(super) fn draw_view(
        &self,
        view: NodeId,
        size: Size,
        secure: &BTreeSet<NodeId>,
        revision: u64,
        reader: &mut ReadDmabuf<'_>,
    ) -> DrawnView {
        if !size_fits(size) {
            return DrawnView::default();
        }
        let Some(source) = self.scene.source(view) else {
            return DrawnView::default();
        };
        if self.scene.node(source).is_none() {
            return DrawnView::default();
        }
        let nodes: BTreeSet<NodeId> = self.subtree(source).into_iter().collect();
        if nodes.contains(&view) {
            log::debug!(area = "render", %view, "a view inside its own source draws nothing");
            return DrawnView::default();
        }
        let rig = self.view_rig(view, size);
        if self.gpu_views {
            let scene = self.view_scene(view, source, size, &rig, &nodes, secure, revision);
            return DrawnView {
                scene: Some(Arc::new(scene)),
                ..DrawnView::default()
            };
        }
        let mut pictures = self.window_pictures(&nodes, reader);
        let image = render_rig(&self.scene, source, &rig, size, |node| pictures.get(&node)).into_image();
        if secure.is_empty() {
            return DrawnView {
                image: Some(image),
                ..DrawnView::default()
            };
        }
        // A nested View is drawn as a plain node and has no pixels of its own here, so it can never carry a secure
        // node's pixels into this picture: only a window or a canvas below the source does, and those are replaced.
        for node in secure {
            match self.secure_treatment {
                SecureTreatment::Mask => {
                    if let Some(picture) = pictures.get_mut(node) {
                        *picture = secure_mask();
                    }
                }
                SecureTreatment::Omit => {
                    pictures.remove(node);
                }
            }
        }
        let redacted = render_rig(&self.scene, source, &rig, size, |node| pictures.get(&node)).into_image();
        DrawnView {
            image: Some(image),
            redacted: Some(redacted),
            scene: None,
        }
    }

    /// The scene a GPU renderer draws a View from: the source subtree through each eye of the rig, as the CPU rig
    /// draws it, and the same subtree again with the secure nodes replaced when there are any.
    ///
    /// The layers follow `render_view` rule for rule: a node draws before its children, a child that paints `behind`
    /// before the node, an invisible node or one whose composed opacity is zero takes its subtree with it, a plane that
    /// projects to nothing too, and a clipping ancestor keeps its children to its own rectangle as it projects. The
    /// nodes that have pixels are the windows' surfaces and the canvases.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn view_scene(
        &self,
        view: NodeId,
        source: NodeId,
        size: Size,
        rig: &ViewRig,
        nodes: &BTreeSet<NodeId>,
        secure: &BTreeSet<NodeId>,
        revision: u64,
    ) -> ViewScene {
        let pictures = self.view_sources(nodes);
        let mask = Arc::new(secure_mask());
        let eyes_of = |redacted: bool| -> Vec<ViewEye> {
            rig.eyes(size)
                .into_iter()
                .filter_map(|(viewport, camera)| {
                    // An eye that cannot project draws nothing: its part of the picture stays black.
                    let projection = eye_pixel_matrix(&camera, viewport.size())?;
                    let mut ops = Vec::new();
                    let emit = ViewEmit {
                        root: source,
                        projection: &projection,
                        pictures: &pictures,
                        secure: redacted.then_some(secure),
                        treatment: self.secure_treatment,
                        mask: &mask,
                    };
                    self.emit_view_node(&emit, source, 1.0, &mut ops);
                    Some(ViewEye { viewport, ops })
                })
                .collect()
        };
        let display = eyes_of(false);
        let capture = (!secure.is_empty()).then(|| eyes_of(true));
        ViewScene::new(view.get(), revision, size, display, capture)
    }

    /// The steps that draw `id` and its children into one eye: see [`Composer::view_scene`].
    pub(super) fn emit_view_node(&self, emit: &ViewEmit<'_>, id: NodeId, parent_opacity: f64, ops: &mut Vec<ViewOp>) {
        if self.scene.visible(id) != Some(true) {
            return;
        }
        let Some(transform) = self.scene.transform(id) else {
            return;
        };
        let opacity = parent_opacity * f64::from(transform.opacity);
        if !opacity.is_finite() || opacity <= 0.0 {
            return;
        }
        let (Some(bounds), Some(world)) = (self.scene.bounds(id), self.scene.subtree_matrix(emit.root, id)) else {
            return;
        };
        // The node's own plane starts at its top-left corner, which is the origin a layer's model maps from.
        let matrix = emit
            .projection
            .mul(&world)
            .mul(&Mat4::translation(f64::from(bounds.x), f64::from(bounds.y), 0.0));
        if !projects_to_a_plane(&matrix) {
            return;
        }
        let model = Model { matrix, opaque: false };
        let plane = ClipPlane {
            dst: Rect::new(0, 0, bounds.width, bounds.height),
            model,
        };
        let clipping = self.scene.clip(id) == Some(true);
        let children = self.scene.children(id).unwrap_or(&[]);
        for child in children.iter().filter(|child| self.scene.behind(**child) == Some(true)) {
            self.emit_view_node(emit, *child, opacity, ops);
        }
        if let Some(layer) = emit.layer(id, bounds, opacity, model) {
            ops.push(ViewOp::Draw(layer));
        }
        if clipping {
            ops.push(ViewOp::PushClip(plane));
        }
        for child in children.iter().filter(|child| self.scene.behind(**child) != Some(true)) {
            self.emit_view_node(emit, *child, opacity, ops);
        }
        if clipping {
            ops.push(ViewOp::PopClip(plane));
        }
    }

    /// The pixels every surface a View can draw is made of: each window's own surface and the surfaces of its tree, and each
    /// layer surface's own and the surfaces of its tree. Each comes with the node that draws it and, when the picture does
    /// not start at the node's corner, where it lies in the node and the node's rectangle.
    fn drawn_sources(&self) -> impl Iterator<Item = (NodeId, &SurfaceSource, Option<(Rect, Rect)>)> {
        let windows = self.windows.values().flat_map(|entry| {
            let placement = Self::own_placement(entry);
            let own = entry
                .node
                .zip(entry.buffer.as_ref())
                .map(|(node, source)| (node, source, placement));
            let below = entry
                .tree
                .list
                .iter()
                .filter_map(|surface| Some((entry.tree.nodes.get(&surface.key)?.node, &surface.source, None)));
            own.into_iter().chain(below)
        });
        let layers = self.layer_nodes.iter().flat_map(|(node, layer)| {
            let own = layer.source.as_ref().map(|source| (*node, source, None));
            let below = layer
                .tree
                .list
                .iter()
                .filter_map(|surface| Some((layer.tree.nodes.get(&surface.key)?.node, &surface.source, None)));
            own.into_iter().chain(below)
        });
        windows.chain(layers)
    }

    /// The pixels each node of `nodes` draws from, with their size: a window's or a layer surface's, or a canvas. A buffer
    /// is shared, not read, so a GPU renderer samples the client's own memory and a GPU buffer needs no readback.
    pub(super) fn view_sources(&self, nodes: &BTreeSet<NodeId>) -> BTreeMap<NodeId, ViewPixels> {
        let mut pictures = BTreeMap::new();
        for (node, source, placement) in self.drawn_sources() {
            let (true, Some(size)) = (nodes.contains(&node), source_size(source)) else {
                continue;
            };
            let crop = match placement {
                None => None,
                Some((local, frame)) => match view_crop(local, frame, size) {
                    Some(crop) => Some(crop),
                    // The buffer lies wholly outside the window geometry: nothing of it is the window.
                    None => continue,
                },
            };
            pictures.insert(
                node,
                ViewPixels {
                    source: source.clone(),
                    size,
                    crop,
                },
            );
        }
        for (node, picture) in &self.canvases {
            if let (true, Some(image)) = (nodes.contains(node), picture.image.as_ref()) {
                // A canvas picture is premultiplied, which is what a snapshot is.
                pictures.insert(
                    *node,
                    ViewPixels {
                        source: SurfaceSource::Snapshot(Arc::clone(image)),
                        size: image.size(),
                        crop: None,
                    },
                );
            }
        }
        pictures
    }

    /// The pixels of each window in `nodes` with a readable buffer, keyed by its node. A GPU buffer (a DMA-BUF) is read
    /// by `reader`, and when it cannot be read its window is drawn as the neutral grey a secure node is masked with,
    /// and the first one is logged: see [`source_picture`].
    pub(super) fn window_pictures(
        &self,
        nodes: &BTreeSet<NodeId>,
        reader: &mut ReadDmabuf<'_>,
    ) -> BTreeMap<NodeId, Image> {
        let mut pictures: BTreeMap<NodeId, Image> = BTreeMap::new();
        for (node, source, placement) in self.drawn_sources() {
            let Some(image) = nodes.contains(&node).then(|| source_picture(source, reader)).flatten() else {
                continue;
            };
            // A window's own picture is the part of its buffer inside the window geometry, which is what the
            // node's rectangle is.
            let image = match placement {
                Some((local, frame)) => geometry_crop(&image, local, frame),
                None => image,
            };
            pictures.insert(node, image);
        }
        // A View of a subtree that holds canvases draws them too. The view renderer blends straight alpha and a
        // canvas picture is premultiplied, as every snapshot is.
        for (node, picture) in &self.canvases {
            if !nodes.contains(node) {
                continue;
            }
            let Some(image) = picture.image.as_ref() else {
                continue;
            };
            let straight = image
                .data()
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|pixel| straight_alpha(*pixel))
                .collect();
            if let Some(straight) = Image::new(image.size(), image.stride(), straight) {
                pictures.insert(*node, straight);
            }
        }
        pictures
    }

    /// How a View draws its source: through the camera node its `camera` names, or flat without one. Without a camera
    /// node the view is orthographic, with its origin at the top left and its image centred on the camera's axis. A
    /// camera node's own transform is its pose; its `projection` text and `fov`, `near` and `far` numbers choose the
    /// projection and clip planes. Four `fov_*` angles that are not all zero make an off-axis frustum of a
    /// `perspective` camera. A `stereo` camera is two eyes side by side, `ipd` metres apart (in the camera's own
    /// `metres`), the frustum being the left eye's. A frustum or an eye separation that cannot be drawn is
    /// [`ViewRig::Blank`].
    pub(super) fn view_rig(&self, view: NodeId, size: Size) -> ViewRig {
        let camera = self
            .scene
            .camera(view)
            .filter(|camera| self.scene.node(*camera).is_some());
        let Some(camera) = camera else {
            return ViewRig::Mono(ViewCamera {
                position: Vec3 {
                    x: f64::from(size.width) / 2.0,
                    y: f64::from(size.height) / 2.0,
                    z: VIEW_ORTHO_DEPTH,
                },
                rotation_degrees: Vec3::default(),
                projection: ViewProjection::Orthographic,
                near: VIEW_NEAR,
                far: VIEW_FAR,
            });
        };
        let (position, rotation_degrees) = self
            .scene
            .transform(camera)
            .map_or((Vec3::default(), Vec3::default()), |transform| {
                (transform.base.offset, transform.base.rotation_degrees)
            });
        let near = self.scene.number(camera, "near").unwrap_or(VIEW_NEAR);
        let far = self.scene.number(camera, "far").unwrap_or(VIEW_FAR);
        let vertical = self.scene.number(camera, "fov").unwrap_or(VIEW_FOV);
        let [left, right, up, down] =
            ["fov_left", "fov_right", "fov_up", "fov_down"].map(|key| self.scene.number(camera, key).unwrap_or(0.0));
        let off_axis = [left, right, up, down].iter().any(|angle| *angle != 0.0);
        let mono = |projection| {
            ViewRig::Mono(ViewCamera {
                position,
                rotation_degrees,
                projection,
                near,
                far,
            })
        };
        match self.scene.text(camera, "projection") {
            Some("perspective") if off_axis => Frustum::from_degrees(left, right, up, down)
                .map_or(ViewRig::Blank, |frustum| mono(frustum.projection())),
            Some("perspective") => mono(ViewProjection::Perspective {
                vertical_fov_degrees: vertical,
            }),
            Some("stereo") => {
                let [eye, _] = eye_sizes(size);
                let frustum = if off_axis {
                    Frustum::from_degrees(left, right, up, down)
                } else {
                    Frustum::symmetric(vertical, f64::from(eye.width) / f64::from(eye.height))
                };
                let metres = self.scene.number(camera, "metres").unwrap_or(VIEW_METRES);
                let ipd = self.scene.number(camera, "ipd").unwrap_or(VIEW_IPD);
                match frustum {
                    Some(left) if metres.is_finite() && metres > 0.0 => ViewRig::Stereo(StereoRig {
                        position,
                        rotation_degrees,
                        eye_separation: ipd / metres,
                        left,
                        near,
                        far,
                    }),
                    _ => ViewRig::Blank,
                }
            }
            _ => mono(ViewProjection::Orthographic),
        }
    }

    /// Every View node in the scene, found from the roots down.
    pub(super) fn view_nodes(&self) -> BTreeSet<NodeId> {
        let mut views = BTreeSet::new();
        let mut stack: Vec<NodeId> = self.scene.root_ids().to_vec();
        while let Some(node) = stack.pop() {
            if self.scene.class(node) == Some("view") {
                views.insert(node);
            }
            stack.extend(self.scene.children(node).unwrap_or(&[]));
        }
        views
    }

    /// Every node a visible View shows, so a window there is on screen though on no monitor.
    #[must_use]
    pub fn view_shown_nodes(&self) -> BTreeSet<NodeId> {
        self.scene.view_shown_nodes()
    }

    /// Every View node in the scene, with the size it is drawn at. A capture source names a
    /// View by its node, so the capture side reads each View's current size from here.
    #[must_use]
    pub fn views(&self) -> Vec<(NodeId, Size)> {
        self.view_nodes()
            .into_iter()
            .filter_map(|view| {
                let bounds = self.scene.bounds(view)?;
                Some((view, Size::new(bounds.width, bounds.height)))
            })
            .collect()
    }

    /// The size a View node is drawn at, or `None` when `node` is not a View of the scene.
    #[must_use]
    pub fn view_size(&self, node: NodeId) -> Option<Size> {
        self.views()
            .into_iter()
            .find_map(|(view, size)| (view == node).then_some(size))
    }

    /// The picture each View's capture reads, keyed by its node. A View with no source has no picture and is left
    /// out. Capture copies a View from here.
    ///
    /// It is the picture the View drew the last time it was rendered, unless a node in its source is `secure`: then it
    /// is the same subtree with those nodes replaced as an output capture replaces a secure layer, so no capture of a
    /// View, whether the shell's still, a `view`-role session or a stream, ever holds a secure node's pixels.
    #[must_use]
    pub fn view_pictures(&self) -> Vec<(NodeId, Arc<Image>)> {
        self.views
            .iter()
            .filter_map(|(view, picture)| {
                let capture = picture.redacted.as_ref().or(picture.image.as_ref())?;
                Some((*view, Arc::clone(capture)))
            })
            .collect()
    }

    /// Whether `view` is a View the renderer draws on the GPU and the composer has a scene for: its picture is a
    /// texture, and a consumer that needs bytes reads it back.
    #[must_use]
    pub fn has_view_scene(&self, view: NodeId) -> bool {
        self.views.get(&view).is_some_and(|picture| picture.scene.is_some())
    }

    /// The scene each View on the GPU path was last drawn from, keyed by its node. A renderer that draws a View's
    /// subtree draws the scene into a texture, and a consumer that needs the pixels reads that texture back
    /// (`Backend::read_view`). Empty unless [`Composer::set_gpu_views`] turned the path on.
    #[must_use]
    pub fn view_scenes(&self) -> Vec<(NodeId, Arc<ViewScene>)> {
        self.views
            .iter()
            .filter_map(|(view, picture)| Some((*view, Arc::clone(picture.scene.as_ref()?))))
            .collect()
    }

    /// Tells the composer whether the active renderer draws a View's subtree into a texture on the GPU. The compositor
    /// sets it once, from the backend, before any frame; a View drawn the other way is drawn again.
    pub fn set_gpu_views(&mut self, on: bool) {
        if self.gpu_views == on {
            return;
        }
        self.gpu_views = on;
        // A picture made the other way would keep a form the renderer cannot show.
        self.views.clear();
    }

    /// Whether a ray that hits `node` goes on into the node's source: a View that takes input in `forward` mode and
    /// has a source that exists.
    pub(super) fn forwards_input(&self, node: NodeId) -> bool {
        self.forwarded_source(node).is_some()
    }

    /// The source a ray that hits `view` goes into, or `None` when it does not go anywhere: the node is not a View, its
    /// `input_mode` is `capture`, or it has no source.
    pub(super) fn forwarded_source(&self, view: NodeId) -> Option<NodeId> {
        if self.scene.class(view) != Some("view") || self.scene.text(view, "input_mode") == Some("capture") {
            return None;
        }
        self.scene
            .source(view)
            .filter(|source| self.scene.node(*source).is_some())
    }

    /// Where a ray that hit `node` at `local` (in the node's own plane) lands, once the Views it passes into have had
    /// it. `depth` bounds how many Views a ray may pass through, so a ring of Views that show each other ends.
    pub(super) fn landing(&self, node: NodeId, local: PointF, depth: u8) -> Landing {
        if let Some(window) = self.window_of(node) {
            return Landing::Window(window, local);
        }
        let Some(source) = self.forwarded_source(node) else {
            return Landing::Node(node, local);
        };
        let Some(depth) = depth.checked_sub(1) else {
            return Landing::Nowhere;
        };
        match self.through_view(node, source, local) {
            Some((inner, inner_local)) => self.landing(inner, inner_local, depth),
            None => Landing::Nowhere,
        }
    }

    /// The node of `source` that `view`'s picture shows at `local`, a point of the View's own plane, with the point in
    /// that node's plane: the pixel of the picture is the point less the View's origin, as the picture is drawn at the
    /// View's size. A stereo View's picture is two eyes side by side and none of it is a pointer's, so a ray lands on
    /// nothing there, as it does in a picture that cannot be drawn.
    pub(super) fn through_view(&self, view: NodeId, source: NodeId, local: PointF) -> Option<(NodeId, PointF)> {
        let bounds = self.scene.bounds(view)?;
        let size = Size::new(bounds.width, bounds.height);
        if self.is_within(view, source) {
            return None;
        }
        let ViewRig::Mono(camera) = self.view_rig(view, size) else {
            return None;
        };
        let pixel = PointF::new(local.x - f64::from(bounds.x), local.y - f64::from(bounds.y));
        pick_view(&self.scene, source, &camera, size, pixel, &|node| {
            self.window_of(node).is_some() || !self.unpublished(node)
        })
    }
}
