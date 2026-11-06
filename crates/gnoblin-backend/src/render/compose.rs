//! The session's scene, and the drawing of one output's frame from it.
//!
//! This is the OutputFrame path of `design/rust-compositor.md` between the window
//! manager and a renderer: the window manager decides where a window is, the scene
//! holds what each window looks like and which pixels need redrawing, and
//! [`Composer::compose`] turns that into a [`OutputFrame`] a renderer can draw.
//!
//! It lives in this crate because it is the one place a protocol engine, a backend
//! and the pure scene crate are allowed to meet
//! (`design/rust-compositor.md`, "Dependency rules"): `gnoblin-wayland` may not
//! name `gnoblin-render`, so the scene has to sit on the backend side of the
//! contract.
//!
//! # What a composer owns
//!
//! - **One node per drawn window.** A window's node exists exactly while it is
//!   visible and has committed content, because a node is what "these pixels are on
//!   screen" means: a client that attaches no buffer has no node, is never drawn and
//!   damages nothing.
//! - **The stacking order**, bottom first, and each node's `z_order` so the scene's
//!   own hit test and the frame agree about what is on top.
//! - **The windows' visual transforms** (opacity, tint), kept here so a node can be
//!   recreated at a new content size without losing the animation playing on it.
//! - **The scene's damage**, taken once per iteration.
//!
//! # Coordinates
//!
//! A window's node is placed by the window manager's geometry, which is in global
//! logical coordinates; the scene's output rectangles are in the same space. A
//! frame is output-local, so [`Composer::compose`] subtracts the output's origin.
//! That is why one scene serves every output and why moving an output moves its
//! windows without re-describing them.
//!
//! # Failure modes
//!
//! 1. **A client that attaches no buffer is drawn anyway.** A window with no content
//!    has no node, and a window with no buffer contributes no layer:
//!    `a_window_without_a_buffer_is_never_drawn`.
//! 2. **A closed window keeps its pixels on screen.** Removing a node damages the
//!    rectangle it occupied: `closing_a_window_damages_the_pixels_it_held`.
//! 3. **A move leaves the old pixels behind.** The scene damages a node's projected
//!    bounds before and after a transform change:
//!    `moving_a_window_damages_both_rectangles`.
//! 4. **Damage misses a window whose client drew into the same buffer.** A commit
//!    damages the window's rectangle even though the node did not move:
//!    `a_commit_damages_the_window_it_drew_into`.
//! 5. **A rotated window is drawn as its bounding box.** A rotation is refused with
//!    a log line and no layer rather than drawn at the wrong angle:
//!    `a_rotated_window_is_not_drawn_as_a_bounding_box`.
//! 6. **An output resize keeps the old pixels.** A changed or new output rectangle is
//!    damaged in full, so the first frame on the new buffer repaints everything:
//!    `an_output_resize_damages_the_whole_output`.
//! 7. **A layer is drawn in global coordinates.** A frame is output-local while the
//!    scene is global: `a_second_output_damages_its_own_coordinates`.
//! 8. **A hidden window's move repaints the output.** A window with no node has no
//!    pixels to move: `a_window_that_is_not_visible_damages_nothing_when_it_moves`.

use std::collections::BTreeMap;

use gnoblin_log::debug;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::utils::Transform;

use gnoblin_render::{
    Color, Damage, Easing, Effects, Mat4, NodeId, NodeKind, NodeSpec, Projection, PropertyValue, Scene, TrackId,
    TrackSpec, TrackStart, TweenPlayer, VisualProperty, VisualTransform,
};
use gnoblin_types::{OutputId, Rect, RectF, Size, Vec3, WindowId};

use crate::render::frame::{Layer, Model, OutputFrame, Surface};

/// How long a placement, move or other visual change takes to reach its target.
const MOTION_MS: u32 = 200;

/// A window's committed content: the crop to draw and its logical size.
///
/// It is the part of a commit the scene needs, and it deliberately excludes the
/// buffer: node placement, damage and hit testing are the same whether the pixels
/// come from shared memory or a DMA buffer, and a test can describe content without
/// a Wayland client.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Content {
    /// The crop to draw, in the buffer's own pixels.
    pub src: RectF,
    /// The crop's logical size on the output: a viewport can scale it.
    pub size: Size,
    /// The buffer's `wl_surface.set_buffer_transform` orientation.
    pub transform: Transform,
}

/// One managed window's rendering state.
#[derive(Clone, Debug)]
struct WindowNode {
    /// The scene node, while the window is drawn.
    node: Option<NodeId>,
    /// The window's frame rectangle in global logical coordinates.
    geometry: Rect,
    /// Its visual transform, kept across node recreation.
    transform: VisualTransform,
    /// The client's `wp_alpha_modifier_v1` factor for its surface, one when it set none.
    /// It multiplies the window's group opacity when the window is drawn.
    surface_alpha: f32,
    /// Its effect chain, kept for the same reason and pushed to the node.
    effects: Effects,
    /// Whether the window is on screen at all (mapped, on the active workspace).
    visible: bool,
    /// The client's content, when it has committed one.
    content: Option<Content>,
    /// The buffer the content is read from, when the client has attached one.
    buffer: Option<WlBuffer>,
}

/// The session's scene and the windows it draws.
#[derive(Debug)]
pub struct Composer {
    scene: Scene,
    windows: BTreeMap<WindowId, WindowNode>,
    /// Paint order, bottom first. It holds every window the composer knows, so a
    /// window that is not drawn yet still has a place for when it is.
    order: Vec<WindowId>,
    background: Color,
    next_node: u64,
    /// The damage of the iteration being drawn, taken once by
    /// [`Composer::begin_frame`] and consumed output by output.
    damage: Option<Damage>,
    /// Animations of window properties, driven by [`Composer::advance_motion`].
    motion: TweenPlayer,
    /// The live track for each window property, so a retarget can supersede it.
    tracks: BTreeMap<(WindowId, VisualProperty), TrackId>,
    /// When set, a change lands at once instead of being animated.
    reduced_motion: bool,
    /// Where the cursor was last drawn, in global layout coordinates, or `None` when hidden.
    cursor: Option<Rect>,
}

impl Default for Composer {
    fn default() -> Self {
        Self::new()
    }
}

impl Composer {
    /// An empty scene with an opaque black background.
    #[must_use]
    pub fn new() -> Self {
        Self {
            scene: Scene::new(),
            windows: BTreeMap::new(),
            order: Vec::new(),
            background: Color::BLACK,
            next_node: 1,
            damage: None,
            motion: TweenPlayer::new(),
            tracks: BTreeMap::new(),
            reduced_motion: false,
            cursor: None,
        }
    }

    /// The colour behind every window.
    #[must_use]
    pub fn background(&self) -> Color {
        self.background
    }

    /// Sets the colour behind every window and damages every output.
    pub fn set_background(&mut self, colour: Color) {
        if !colour.is_finite() || colour == self.background {
            return;
        }
        self.background = colour;
        let outputs: Vec<OutputId> = self.scene.outputs().keys().copied().collect();
        for output in outputs {
            self.scene.damage_output(output);
        }
    }

    /// Declares or replaces an output's rectangle.
    ///
    /// A new output has never been drawn and a moved or resized one has no valid
    /// pixels yet, so both are damaged in full: the renderer's buffer for it is
    /// either new or stale.
    pub fn set_output(&mut self, output: OutputId, rect: Rect) {
        let previous = self.scene.outputs().get(&output).copied();
        self.scene.set_output(output, rect);
        if previous != Some(rect) {
            self.scene.damage_output(output);
        }
    }

    /// Removes an output and its damage.
    pub fn remove_output(&mut self, output: OutputId) {
        self.scene.remove_output(output);
    }

    /// Damages an output in full, so its next frame repaints every pixel.
    ///
    /// A caller that drew something the scene does not know about — a session lock
    /// cover, and the reveal that takes it down — needs the output painted again
    /// even though no window moved.
    pub fn damage_output(&mut self, output: OutputId) {
        self.scene.damage_output(output);
    }

    /// Sets where the cursor is drawn, in global layout coordinates, or hides it with
    /// `None`. The old and new rectangles are damaged when they differ. `redraw` damages
    /// them even when the rectangle did not move, for a change of shape, which paints the
    /// same place with other pixels.
    pub fn set_cursor(&mut self, cursor: Option<Rect>, redraw: bool) {
        if cursor == self.cursor && !redraw {
            return;
        }
        for rect in [self.cursor, cursor].into_iter().flatten() {
            self.scene.damage_rect(rect);
        }
        self.cursor = cursor;
    }

    /// Takes every output's damage for this iteration.
    ///
    /// It is taken once, not per output: the backend renders outputs one call at a
    /// time, and a second `take_damage` would find an empty scene and leave every
    /// output after the first stale.
    pub fn begin_frame(&mut self) {
        self.damage = Some(self.scene.take_damage());
    }

    /// Drops the damage of the iteration that just ended.
    pub fn end_frame(&mut self) {
        self.damage = None;
    }

    /// Starts drawing a window.
    pub fn insert_window(&mut self, window: WindowId, geometry: Rect) {
        if self.windows.contains_key(&window) {
            self.place_window(window, geometry);
            return;
        }
        self.windows.insert(
            window,
            WindowNode {
                // Attached when content arrives, so a window's node identity never
                // changes for the rest of its life.
                node: None,
                geometry,
                transform: VisualTransform::IDENTITY,
                effects: Effects::new(),
                visible: true,
                content: None,
                surface_alpha: 1.0,
                buffer: None,
            },
        );
        self.order.push(window);
        self.sync(window, false);
    }

    /// Stops drawing a window, damaging what it held.
    pub fn remove_window(&mut self, window: WindowId) {
        let Some(entry) = self.windows.remove(&window) else {
            return;
        };
        self.cancel_tracks(window);
        if let Some(node) = entry.node {
            let _ = self.scene.remove(node);
        }
        self.order.retain(|candidate| *candidate != window);
    }

    /// Moves a window's frame rectangle.
    pub fn place_window(&mut self, window: WindowId, geometry: Rect) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        if entry.geometry == geometry {
            return;
        }
        entry.geometry = geometry;
        self.sync(window, false);
    }

    /// Shows or hides a window without forgetting where it is.
    pub fn set_window_visible(&mut self, window: WindowId, visible: bool) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        if entry.visible == visible {
            return;
        }
        entry.visible = visible;
        self.sync(window, false);
    }

    /// Sets a window's group opacity.
    ///
    /// A value outside 0 to 1 or a non-finite one is clamped, because the animation
    /// player can overshoot and a `NaN` opacity would poison the blend.
    pub fn set_window_opacity(&mut self, window: WindowId, opacity: f32) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        let opacity = if opacity.is_finite() {
            opacity.clamp(0.0, 1.0)
        } else {
            1.0
        };
        if entry.transform.opacity == opacity {
            return;
        }
        entry.transform.opacity = opacity;
        self.sync(window, true);
    }

    /// Sets a window's `wp_alpha_modifier_v1` factor, which multiplies its group opacity.
    ///
    /// A factor outside zero to one, or not finite, is clamped the same way as the group
    /// opacity, so a bad value cannot poison the blend.
    pub fn set_window_surface_alpha(&mut self, window: WindowId, factor: f32) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        let factor = if factor.is_finite() {
            factor.clamp(0.0, 1.0)
        } else {
            1.0
        };
        if entry.surface_alpha == factor {
            return;
        }
        entry.surface_alpha = factor;
        self.sync(window, true);
    }

    /// Sets a window's multiplicative tint.
    pub fn set_window_tint(&mut self, window: WindowId, tint: Color) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        if !tint.is_finite() || entry.transform.tint == tint {
            return;
        }
        entry.transform.tint = tint;
        self.sync(window, true);
    }

    /// Sets a window's corner radius, in logical pixels.
    ///
    /// The radius is the scene's own property (`VisualTransform::corner_radius`), so a
    /// tween can animate it and the frame carries it to whichever renderer draws the
    /// window. A negative or non-finite radius is ignored rather than clamped: a corner
    /// the caller cannot express is a caller bug, and drawing a square window would
    /// hide it.
    pub fn set_window_corner_radius(&mut self, window: WindowId, radius: f32) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        if !radius.is_finite() || radius < 0.0 || entry.transform.corner_radius == radius {
            return;
        }
        entry.transform.corner_radius = radius;
        self.sync(window, true);
    }

    /// Replaces a window's effect chain.
    ///
    /// The chain is the scene's own [`Effects`], already validated pass by pass, so the
    /// values a GPU shader reads are the values the script that set them was checked
    /// against. The old and new chain damage nothing on their own: the chain only draws
    /// inside or around the window, which [`Composer::sync`] re-damages.
    pub fn set_window_effects(&mut self, window: WindowId, effects: Effects) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        if entry.effects == effects {
            return;
        }
        entry.effects = effects;
        self.sync(window, true);
    }

    /// Sets a window's rotation about the depth axis, the 2D rotation.
    ///
    /// It is recorded so the damage model keeps the rotated bounds and refused when a
    /// frame is drawn: see [`Composer::compose`].
    pub fn set_window_rotation(&mut self, window: WindowId, degrees: f64) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        if !degrees.is_finite() || entry.transform.base.rotation_degrees.z == degrees {
            return;
        }
        entry.transform.base.rotation_degrees.z = degrees;
        self.sync(window, true);
    }

    /// Places a window's frame and base transform together: its rotation, scale and depth
    /// offset. A window already drawn tweens from where it is to the new placement; a new
    /// window takes it at once.
    ///
    /// The x and y offset always come from `geometry`, so the base transform's offset is
    /// only its depth.
    pub fn place_window_animated(&mut self, window: WindowId, geometry: Rect, base: gnoblin_types::Transform) {
        if !self.windows.contains_key(&window) {
            self.insert_window(window, geometry);
        }
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        entry.geometry = geometry;
        entry.transform.base = base;
        self.sync_animated(window);
    }

    /// Sets whether property changes land at once (`true`) or are animated (`false`).
    ///
    /// It applies to changes started after the call; a track already running finishes.
    pub fn set_reduced_motion(&mut self, reduced: bool) {
        self.reduced_motion = reduced;
    }

    /// Advances every running animation by `elapsed_ms` and forgets the ones that ended.
    pub fn advance_motion(&mut self, elapsed_ms: u32) {
        self.motion.advance(&mut self.scene, elapsed_ms);
        for end in self.motion.take_ended() {
            self.tracks.retain(|_, track| *track != end.track);
        }
    }

    /// A window's node's world matrix, as the scene currently holds it, or `None` when it is not drawn.
    #[must_use]
    pub fn window_matrix(&self, window: WindowId) -> Option<Mat4> {
        let node = self.windows.get(&window)?.node?;
        self.scene.world_matrix(node)
    }

    /// A window's node's bounds projected to the output plane, or `None` when it is not drawn.
    #[must_use]
    pub fn window_projected_bounds(&self, window: WindowId) -> Option<Rect> {
        let node = self.windows.get(&window)?.node?;
        self.scene.absolute_bounds(node)
    }

    /// Replaces a window's content, or clears it when the client unmaps.
    ///
    /// A commit always damages the window's rectangle: the client may have drawn
    /// into the same buffer, and nothing in the handle says whether its pixels
    /// changed.
    pub fn set_window_content(&mut self, window: WindowId, content: Option<Content>) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        entry.content = content;
        self.sync(window, true);
    }

    /// Records the buffer a window's content is read from.
    ///
    /// The scene does not look inside it, so it carries no damage of its own; the
    /// commit that attached it went through [`Composer::set_window_content`].
    pub fn set_window_buffer(&mut self, window: WindowId, buffer: Option<WlBuffer>) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        entry.buffer = buffer;
    }

    /// Sets the paint order, topmost first as `gnoblin-wm` reports it.
    pub fn set_stacking(&mut self, order: &[WindowId]) {
        let mut kept: Vec<WindowId> = Vec::with_capacity(self.order.len());
        for window in order.iter().rev() {
            if self.windows.contains_key(window) && !kept.contains(window) {
                kept.push(*window);
            }
        }
        // Windows the policy did not name keep their place, above the ones it did, so
        // a half-applied order cannot make a window disappear.
        for window in &self.order {
            if !kept.contains(window) {
                kept.push(*window);
            }
        }
        if kept == self.order {
            return;
        }
        self.order = kept;
        let windows: Vec<WindowId> = self.order.clone();
        for window in windows {
            self.sync(window, false);
        }
    }

    /// Draws one output's frame from the damage [`Composer::begin_frame`] took.
    ///
    /// Returns whether anything has to be drawn: `false` means the output did not
    /// change and its previous pixels still stand.
    pub fn compose(&self, output: OutputId, frame: &mut OutputFrame) -> bool {
        frame.clear();
        frame.set_background(self.background);
        let Some(rect) = self.scene.outputs().get(&output).copied() else {
            return false;
        };
        let Some(damage) = self.damage.as_ref().and_then(|damage| damage.for_output(output)) else {
            return false;
        };
        for damaged in &damage.rects {
            frame.push_damage(local(*damaged, rect));
        }
        if frame.is_empty() {
            return false;
        }
        let mut spatial = false;
        for window in &self.order {
            let Some(entry) = self.windows.get(window) else {
                continue;
            };
            if entry.node.is_none() {
                continue;
            }
            let (Some(content), Some(buffer)) = (entry.content, entry.buffer.as_ref()) else {
                // A window whose client has committed content but no buffer has no
                // pixels to draw.
                continue;
            };
            let dst = Rect::new(
                entry.geometry.x.saturating_sub(rect.x),
                entry.geometry.y.saturating_sub(rect.y),
                content.size.width,
                content.size.height,
            );
            let model = entry.node.and_then(|node| self.model(node, rect));
            let on_output = match model {
                // A projected window's flat rectangle says nothing about where it lands.
                Some(_) => entry
                    .node
                    .and_then(|node| self.scene.absolute_bounds(node))
                    .is_some_and(|bounds| bounds.intersection(rect).is_some()),
                None => dst.intersection(rect).is_some(),
            };
            if !on_output {
                // The window is on another output.
                continue;
            }
            if model.is_some() && !spatial {
                // A tilted window's pixels can land anywhere on the output, so while one is on
                // it the whole output is repainted rather than its flat rectangle.
                frame.push_damage(Rect::new(0, 0, rect.width, rect.height));
                spatial = true;
            }
            if entry.transform.base.rotation_degrees.z != 0.0 {
                // Pixman's frame API rotates the whole destination rather than one
                // layer, so a rotated window would appear as its bounding box. The
                // scene's damage already covers the rotated bounds, so skipping the
                // layer repaints those pixels with the background rather than leaving
                // stale ones.
                debug!(%window, "[render] a rotated window is not drawn by the software renderer");
                continue;
            }
            frame.push_layer(Layer::Surface(Surface {
                buffer: buffer.clone(),
                src: content.src,
                dst,
                transform: content.transform,
                opacity: entry.transform.opacity * entry.surface_alpha,
                tint: entry.transform.tint,
                corner_radius: entry.transform.corner_radius,
                effects: entry.effects.clone(),
                model,
            }));
        }
        !frame.is_empty()
    }

    /// How a node is placed on `output`: `None` for a flat rectangle drawn in paint order, which
    /// is every node in a planar scene under the orthographic projection.
    fn model(&self, node: NodeId, output: Rect) -> Option<Model> {
        let world = self.scene.world_matrix(node)?;
        if world.is_planar() && matches!(self.scene.projection_for(node)?, Projection::Orthographic) {
            return None;
        }
        Some(Model {
            matrix: Mat4::translation(-f64::from(output.x), -f64::from(output.y), 0.0).mul(&world),
            opaque: false,
        })
    }

    /// The transform a window's node should reach: its base transform placed at its geometry.
    fn target(entry: &WindowNode) -> VisualTransform {
        let mut transform = entry.transform;
        transform.base.offset = Vec3::new(
            f64::from(entry.geometry.x),
            f64::from(entry.geometry.y),
            transform.base.offset.z,
        );
        transform
    }

    /// Starts an animation towards each property's target that differs from the node's value,
    /// when `animate` is set and motion is not reduced. A property starts from its current
    /// value, so a superseded animation continues from where it is. Otherwise the target is
    /// written at once.
    fn retarget(&mut self, window: WindowId, animate: bool) {
        let Some(entry) = self.windows.get(&window) else {
            return;
        };
        let Some(node) = entry.node else {
            return;
        };
        let immediate = !animate || self.reduced_motion;
        let mut landing = Vec::new();
        let target = Self::target(entry);
        for property in VisualProperty::ALL {
            if property == VisualProperty::Scale {
                continue;
            }
            let value = property.get(&target);
            self.cancel_track(window, property);
            if self.scene.property(node, property).ok() == Some(value) {
                continue;
            }
            if immediate {
                landing.push((property, value));
                continue;
            }
            let spec = TrackSpec {
                from: TrackStart::Current,
                duration_ms: MOTION_MS,
                easing: Easing::EaseOutCubic,
                ..TrackSpec::new(node, property, value)
            };
            if let Ok(track) = self.motion.start(&mut self.scene, spec) {
                self.tracks.insert((window, property), track);
            }
        }
        self.land(node, &landing);
    }

    /// Writes several properties of one node as a single transform change.
    ///
    /// The scene damages the node's before and after bounds on every write, so writing
    /// the properties one at a time would damage each intermediate pose: a move from
    /// (20, 30) to (200, 300) would also damage (200, 30). A property a value refuses is
    /// left as it was, as `Scene::set_property` leaves it.
    fn land(&mut self, node: NodeId, changes: &[(VisualProperty, PropertyValue)]) {
        let Some(mut candidate) = self.scene.transform(node).copied() else {
            return;
        };
        let mut changed = false;
        for &(property, value) in changes {
            let mut next = candidate;
            if property.apply(&mut next, value).is_ok() {
                candidate = next;
                changed = true;
            }
        }
        if changed {
            let _ = self.scene.set_transform(node, candidate);
        }
    }

    /// Cancels a window property's animation, leaving the value where it is.
    fn cancel_track(&mut self, window: WindowId, property: VisualProperty) {
        if let Some(track) = self.tracks.remove(&(window, property)) {
            let _ = self.motion.cancel(track);
        }
    }

    /// Cancels every animation of a window.
    fn cancel_tracks(&mut self, window: WindowId) {
        let keys: Vec<(WindowId, VisualProperty)> = self.tracks.keys().filter(|key| key.0 == window).copied().collect();
        for (owner, property) in keys {
            self.cancel_track(owner, property);
        }
    }

    /// Makes a window's node match its state, landing every change at once.
    ///
    /// `force` re-damages the node even when nothing it holds changed, which is what
    /// a commit means: the buffer handle is the same, its pixels are not.
    fn sync(&mut self, window: WindowId, force: bool) {
        self.sync_with(window, force, false);
    }

    /// Makes a window's node match its state, animating its transform towards a placement.
    fn sync_animated(&mut self, window: WindowId) {
        self.sync_with(window, false, true);
    }

    /// The body of [`Self::sync`]: `animate` tweens the transform rather than landing it.
    fn sync_with(&mut self, window: WindowId, force: bool, animate: bool) {
        let Some(entry) = self.windows.get(&window) else {
            return;
        };
        let content = if entry.visible { entry.content } else { None };
        let node = entry.node;
        let transform = Self::target(entry);
        let effects = entry.effects.clone();
        let z_order = self.z_order(window);
        let Some(content) = content else {
            if let Some(node) = node {
                self.cancel_tracks(window);
                let _ = self.scene.remove(node);
            }
            if let Some(entry) = self.windows.get_mut(&window) {
                entry.node = None;
            }
            return;
        };
        let bounds = Rect::from_size(content.size);
        match node {
            Some(node) if self.scene.bounds(node) == Some(bounds) => {
                if force && let Some(current) = self.scene.transform(node).copied() {
                    let _ = self.scene.set_transform(node, current);
                }
                if self.scene.z_order(node) != Some(z_order) {
                    let _ = self.scene.set_z_order(node, z_order);
                }
                if self.scene.effects(node) != Some(&effects) {
                    let _ = self.scene.set_effects(node, effects.clone());
                }
                self.retarget(window, animate);
            }
            existing => {
                // The content changed size, or there was no node yet: the node is
                // replaced so its bounds are the content's, and both the old and the
                // new rectangle are damaged. A new node takes its target at once.
                if let Some(node) = existing {
                    self.cancel_tracks(window);
                    let _ = self.scene.remove(node);
                }
                let id = NodeId::new(self.next_node);
                self.next_node = self.next_node.saturating_add(1);
                let spec = NodeSpec {
                    parent: None,
                    z_order,
                    bounds,
                    transform,
                    effects,
                    kind: NodeKind::Visual,
                };
                if self.scene.insert(id, spec).is_ok()
                    && let Some(entry) = self.windows.get_mut(&window)
                {
                    entry.node = Some(id);
                }
            }
        }
    }

    /// A node's paint position, bottom first.
    fn z_order(&self, window: WindowId) -> i32 {
        match self.order.iter().position(|candidate| *candidate == window) {
            Some(index) => i32::try_from(index).unwrap_or(i32::MAX),
            None => i32::try_from(self.order.len()).unwrap_or(i32::MAX),
        }
    }
}

/// A global rectangle in an output's own coordinates.
fn local(rect: Rect, output: Rect) -> Rect {
    rect.translate(-output.x, -output.y)
}

#[cfg(test)]
mod tests {
    use gnoblin_types::{Point, WindowId};

    use super::*;

    fn content() -> Content {
        Content {
            src: RectF::new(0.0, 0.0, 40.0, 50.0),
            size: Size::new(40, 50),
            transform: Transform::Normal,
        }
    }

    fn screen() -> (Composer, OutputId) {
        let mut composer = Composer::new();
        let output = OutputId::new(0);
        composer.set_output(output, Rect::new(0, 0, 800, 600));
        // The output's first frame is not this test's subject.
        composer.begin_frame();
        composer.end_frame();
        (composer, output)
    }

    /// The damage the next frame for `output` carries, or `None` when nothing
    /// changed.
    fn damage_of(composer: &mut Composer, output: OutputId) -> Option<Vec<Rect>> {
        composer.begin_frame();
        let mut frame = OutputFrame::new();
        let changed = composer.compose(output, &mut frame);
        composer.end_frame();
        changed.then(|| frame.damage().to_vec())
    }

    #[test]
    fn a_window_without_a_buffer_is_never_drawn() {
        let (mut composer, output) = screen();
        composer.insert_window(WindowId::new(1), Rect::new(10, 10, 100, 100));
        composer.place_window(WindowId::new(1), Rect::new(50, 50, 100, 100));
        assert_eq!(
            damage_of(&mut composer, output),
            None,
            "a window with no committed content has no pixels to damage"
        );
    }

    #[test]
    fn a_window_with_content_but_no_buffer_has_no_layer() {
        let (mut composer, output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(10, 10, 100, 100));
        composer.set_window_content(window, Some(content()));
        composer.begin_frame();
        let mut frame = OutputFrame::new();
        assert!(composer.compose(output, &mut frame));
        composer.end_frame();
        assert!(
            frame.layers().is_empty(),
            "the node exists and damages its rectangle, but nothing can be drawn"
        );
    }

    #[test]
    fn an_output_resize_damages_the_whole_output() {
        let (mut composer, output) = screen();
        composer.set_output(output, Rect::new(0, 0, 1024, 768));
        assert_eq!(
            damage_of(&mut composer, output),
            Some(vec![Rect::new(0, 0, 1024, 768)]),
            "the new buffer holds no pixels, so everything is redrawn"
        );
    }

    #[test]
    fn an_output_that_did_not_change_damages_nothing() {
        let (mut composer, output) = screen();
        composer.set_output(output, Rect::new(0, 0, 800, 600));
        assert_eq!(damage_of(&mut composer, output), None);
    }

    #[test]
    fn closing_a_window_damages_the_pixels_it_held() {
        let (mut composer, output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        damage_of(&mut composer, output);
        composer.set_window_content(window, None);
        let rects = damage_of(&mut composer, output).expect("the rectangle it held");
        assert!(
            rects.iter().any(|rect| rect.contains(Point::new(20, 30))),
            "the rectangle the window held is damaged: {rects:?}"
        );
    }

    #[test]
    fn moving_a_window_damages_both_rectangles() {
        let (mut composer, output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        damage_of(&mut composer, output);
        composer.place_window(window, Rect::new(200, 300, 40, 50));
        let rects = damage_of(&mut composer, output).expect("both rectangles");
        assert_eq!(rects.len(), 2, "the old and the new rectangle: {rects:?}");
        assert!(rects.contains(&Rect::new(20, 30, 40, 50)));
        assert!(rects.contains(&Rect::new(200, 300, 40, 50)));
    }

    #[test]
    fn moving_a_translucent_rounded_window_damages_both_rectangles() {
        let (mut composer, output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.set_window_opacity(window, 0.5);
        composer.set_window_corner_radius(window, 4.0);
        damage_of(&mut composer, output);
        composer.place_window(window, Rect::new(200, 300, 40, 50));
        let rects = damage_of(&mut composer, output).expect("both rectangles");
        assert_eq!(rects.len(), 2, "the old and the new rectangle: {rects:?}");
        assert!(rects.contains(&Rect::new(20, 30, 40, 50)));
        assert!(rects.contains(&Rect::new(200, 300, 40, 50)));
    }

    #[test]
    fn a_commit_damages_the_window_it_drew_into() {
        let (mut composer, output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        damage_of(&mut composer, output);
        // The same content again: the node does not move, the client still drew.
        composer.set_window_content(window, Some(content()));
        let rects = damage_of(&mut composer, output).expect("a commit damages the window");
        assert!(
            rects.iter().any(|rect| rect.contains(Point::new(30, 40))),
            "a commit damages the window's rectangle: {rects:?}"
        );
    }

    #[test]
    fn a_second_output_damages_its_own_coordinates() {
        let mut composer = Composer::new();
        let first = OutputId::new(0);
        let second = OutputId::new(1);
        composer.set_output(first, Rect::new(0, 0, 800, 600));
        composer.set_output(second, Rect::new(800, 0, 800, 600));
        composer.begin_frame();
        composer.end_frame();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(810, 20, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.begin_frame();
        let mut frame = OutputFrame::new();
        assert!(composer.compose(second, &mut frame));
        assert_eq!(
            frame.damage(),
            &[Rect::new(10, 20, 40, 50)],
            "a frame is output-local, so the output's origin is subtracted"
        );
        let mut first_frame = OutputFrame::new();
        assert!(
            !composer.compose(first, &mut first_frame),
            "the first output did not change"
        );
        composer.end_frame();
    }

    #[test]
    fn a_hidden_window_keeps_no_pixels_to_move() {
        let (mut composer, output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.set_window_visible(window, false);
        damage_of(&mut composer, output);
        composer.place_window(window, Rect::new(200, 300, 40, 50));
        assert_eq!(
            damage_of(&mut composer, output),
            None,
            "a hidden window has no pixels to move"
        );
    }

    #[test]
    fn a_rotated_window_is_not_drawn_as_a_bounding_box() {
        let (mut composer, output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.set_window_rotation(window, 90.0);
        composer.begin_frame();
        let mut frame = OutputFrame::new();
        assert!(composer.compose(output, &mut frame));
        assert!(
            frame.layers().is_empty(),
            "the rotation is refused rather than drawn at the wrong angle"
        );
        assert!(
            frame.damage().iter().any(|rect| rect.contains(Point::new(30, 40))),
            "the scene still damages the rotated bounds"
        );
        composer.end_frame();
    }

    #[test]
    fn opacity_is_clamped_rather_than_poisoned() {
        let (mut composer, _output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(0, 0, 10, 10));
        composer.set_window_opacity(window, f32::NAN);
        assert_eq!(composer.windows[&window].transform.opacity, 1.0);
        composer.set_window_opacity(window, 4.0);
        assert_eq!(composer.windows[&window].transform.opacity, 1.0);
        composer.set_window_opacity(window, -1.0);
        assert_eq!(composer.windows[&window].transform.opacity, 0.0);
    }

    #[test]
    fn a_stacking_change_damages_only_the_windows_that_moved_in_the_order() {
        let (mut composer, output) = screen();
        let first = WindowId::new(1);
        let second = WindowId::new(2);
        composer.insert_window(first, Rect::new(0, 0, 10, 10));
        composer.set_window_content(first, Some(content()));
        composer.insert_window(second, Rect::new(5, 5, 10, 10));
        composer.set_window_content(second, Some(content()));
        damage_of(&mut composer, output);
        // The order already matches the insertion order, so nothing changes.
        composer.set_stacking(&[second, first]);
        assert_eq!(
            damage_of(&mut composer, output),
            None,
            "an unchanged stacking order damages nothing"
        );
        composer.set_stacking(&[first, second]);
        assert!(
            damage_of(&mut composer, output).is_some(),
            "a changed order damages the windows whose overlap changed"
        );
    }

    #[test]
    fn a_frame_with_no_damage_is_not_drawn() {
        let (mut composer, output) = screen();
        composer.begin_frame();
        let mut frame = OutputFrame::new();
        frame.push_damage(Rect::new(0, 0, 4, 4));
        assert!(!composer.compose(output, &mut frame));
        composer.end_frame();
        assert!(frame.is_empty(), "a frame with no damage keeps no stale description");
    }

    #[test]
    fn damage_is_taken_once_per_iteration() {
        let (mut composer, output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(0, 0, 10, 10));
        composer.set_window_content(window, Some(content()));
        composer.begin_frame();
        let mut first = OutputFrame::new();
        let mut second = OutputFrame::new();
        assert!(composer.compose(output, &mut first));
        assert!(
            composer.compose(output, &mut second),
            "a second output's call in the same iteration still sees its damage"
        );
        composer.end_frame();
        let mut third = OutputFrame::new();
        assert!(!composer.compose(output, &mut third), "the next iteration starts empty");
    }

    /// Whether some damage rectangle covers `rect` in full.
    fn covers(damage: &[Rect], rect: Rect) -> bool {
        let right = i64::from(rect.x) + i64::from(rect.width);
        let bottom = i64::from(rect.y) + i64::from(rect.height);
        damage.iter().any(|outer| {
            let outer_right = i64::from(outer.x) + i64::from(outer.width);
            let outer_bottom = i64::from(outer.y) + i64::from(outer.height);
            outer.x <= rect.x && outer.y <= rect.y && outer_right >= right && outer_bottom >= bottom
        })
    }

    #[test]
    fn moving_the_cursor_damages_both_rectangles() {
        let (mut composer, output) = screen();
        let before = Rect::new(10, 10, 24, 24);
        let after = Rect::new(100, 40, 24, 24);
        composer.set_cursor(Some(before), false);
        assert!(
            damage_of(&mut composer, output).is_some(),
            "a cursor that appears is drawn"
        );
        composer.set_cursor(Some(after), false);
        let damage = damage_of(&mut composer, output).expect("a moved cursor is drawn");
        assert!(covers(&damage, before), "the old cursor is erased");
        assert!(covers(&damage, after), "the new cursor is painted");
    }

    #[test]
    fn a_cursor_that_stays_put_damages_nothing() {
        let (mut composer, output) = screen();
        composer.set_cursor(Some(Rect::new(10, 10, 24, 24)), false);
        assert!(damage_of(&mut composer, output).is_some());
        composer.set_cursor(Some(Rect::new(10, 10, 24, 24)), false);
        assert!(damage_of(&mut composer, output).is_none(), "no motion, no damage");
    }

    #[test]
    fn a_shape_change_at_the_same_place_redraws_the_cursor() {
        let (mut composer, output) = screen();
        let at = Rect::new(10, 10, 24, 24);
        composer.set_cursor(Some(at), false);
        assert!(damage_of(&mut composer, output).is_some());
        composer.set_cursor(Some(at), true);
        let damage = damage_of(&mut composer, output).expect("a new shape is drawn");
        assert!(covers(&damage, at));
    }

    #[test]
    fn a_global_rectangle_becomes_local() {
        assert_eq!(
            local(Rect::new(810, 20, 40, 50), Rect::new(800, 0, 800, 600)),
            Rect::new(10, 20, 40, 50)
        );
        assert_eq!(
            local(Rect::new(-30, -10, 40, 50), Rect::new(-100, 0, 800, 600)),
            Rect::new(70, -10, 40, 50)
        );
    }
}
