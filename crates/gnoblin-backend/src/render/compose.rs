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
//! 9. **A closed window vanishes at once.** A window the client destroyed fades its last
//!    picture out, copied from its buffer before the client can free it:
//!    `a_closing_window_is_drawn_from_its_picture`.

use std::collections::BTreeMap;
use std::sync::Arc;

use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::utils::Transform;

use gnoblin_animation::{Easing, PropertyValue, VisualProperty, VisualTransform};
use gnoblin_render::{
    Damage, Effects, Mat4, NodeKind, NodeSpec, Projection, Scene, TrackSpec, TrackStart, TweenPlayer,
};
use gnoblin_types::{Color, NodeId, OutputId, Rect, RectF, Size, TrackId, Vec3, WindowId};

use gnoblin_render_frame::frame::{Layer, Model, OutputFrame, Solid, Surface, SurfaceSource};
use gnoblin_render_frame::image::Image;
use gnoblin_render_frame::shm;
use gnoblin_wm::WorkspaceDirection;

use crate::log;

/// How long a placement, move or other visual change takes to reach its target.
const MOTION_MS: u32 = 200;

/// How long a visual bell's flash takes to fade out, in milliseconds.
const FLASH_MS: u32 = 150;

/// The alpha a visual bell's flash starts at, before it fades.
const FLASH_PEAK: f32 = 0.5;

/// One visual bell flash in progress.
///
/// It is a white rectangle drawn over everything on the output, fading out over
/// [`FLASH_MS`]. The composer keeps no pixels for it: it is described afresh in every
/// frame until it ends, so the damage it leaves behind is what the next frame paints.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Flash {
    /// The rectangle flashed, in global logical coordinates, or `None` for every output
    /// in full.
    area: Option<Rect>,
    /// How long the flash has been running.
    elapsed_ms: u32,
    /// Whether motion is reduced: the flash is drawn once at its peak and then ends with the frame
    /// that draws it, instead of fading.
    reduced: bool,
}

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

/// What a closing window becomes once its fade ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloseEnd {
    /// The window was destroyed: it is forgotten, and leaves the stacking order.
    Remove,
    /// The client unmapped it: its node goes but the window stays known, so a later
    /// re-map can draw it again.
    Hide,
}

/// A window that is closing: the picture it kept and the crop it was drawn with.
#[derive(Clone, Debug)]
struct Closing {
    /// The buffer's pixels, read back when the window was closed.
    image: Arc<Image>,
    /// The crop and transform the window was drawn with, so the picture matches it.
    content: Content,
    /// What the window becomes when the fade ends.
    on_end: CloseEnd,
}

/// The horizontal offsets a workspace switch moves windows along, in global logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
struct WorkspaceSlide {
    /// The offset a window leaving the workspace slides to.
    outgoing: f64,
    /// The offset a window entering the workspace slides in from.
    incoming: f64,
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
    /// The opacity a matched window rule sets, one when none does. It multiplies the group
    /// opacity beside the placement opacity, which the layout rewrites on every placement.
    rule_opacity: f32,
    /// Its effect chain, kept for the same reason and pushed to the node.
    effects: Effects,
    /// Whether the window is on screen at all (mapped, on the active workspace).
    visible: bool,
    /// Whether the window's surface is secure, so a capture masks it. The window
    /// manager sets it from a matched `secure` rule; the layer registry records the same
    /// flag for the window's surface record.
    secure: bool,
    /// The client's content, when it has committed one.
    content: Option<Content>,
    /// The buffer the content is read from, when the client has attached one.
    buffer: Option<WlBuffer>,
    /// The window's last picture, once it is closing: drawn in place of its buffer until its
    /// fade ends, when the window is removed.
    closing: Option<Closing>,
    /// The offset added to the window's geometry along the workspace slide. Zero when no slide
    /// is running.
    slide: f64,
    /// Whether the window is sliding off the workspace it left: it is hidden, still drawn while
    /// it slides, and loses its node when the slide ends.
    leaving: bool,
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
    /// The visual bell's flash in progress, if any. Only one runs at a time: a bell rung
    /// during a flash restarts it.
    flash: Option<Flash>,
    /// The workspace switch in progress, while the compositor is between its begin and end
    /// calls. `None` when no switch is running or motion is reduced.
    workspace_slide: Option<WorkspaceSlide>,
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
            flash: None,
            workspace_slide: None,
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
        // A reduced-motion flash is drawn by the one frame that began after its bell, so it
        // ends here rather than on a timer that might pass before any frame draws it.
        if self.flash.is_some_and(|flash| flash.reduced) {
            self.flash = None;
        }
    }

    /// Starts drawing a window.
    pub fn insert_window(&mut self, window: WindowId, geometry: Rect) {
        if self.windows.contains_key(&window) {
            // A window shown again while its unmap fades (an X11 window mapping again) keeps its
            // node and stops fading, as a Wayland window's commit does.
            self.cancel_hide_fade(window);
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
                secure: false,
                content: None,
                surface_alpha: 1.0,
                rule_opacity: 1.0,
                buffer: None,
                closing: None,
                slide: 0.0,
                leaving: false,
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

    /// Stops drawing a window the client has destroyed, fading its last picture out.
    ///
    /// The picture is read from the window's buffer now, because the client may free that
    /// buffer once it has destroyed the window. A window with no picture to read, or with
    /// motion reduced, is removed at once, as [`Composer::remove_window`] does. A window that is
    /// already fading for an unmap keeps its fade, and is removed when the fade ends.
    pub fn close_window(&mut self, window: WindowId) {
        if let Some(closing) = self.windows.get_mut(&window).and_then(|entry| entry.closing.as_mut()) {
            closing.on_end = CloseEnd::Remove;
            return;
        }
        let closing = self.snapshot(window, CloseEnd::Remove);
        self.close_with_snapshot(window, closing);
    }

    /// Hides a window the client has unmapped, by committing no buffer: its last picture fades
    /// out, then its node goes, but the window stays known so a later map draws it again.
    ///
    /// The picture is read the same way [`Composer::close_window`] reads it. An unmap of a window
    /// that is already fading changes nothing.
    pub fn unmap_window(&mut self, window: WindowId) {
        if self.is_fading(window) {
            return;
        }
        let closing = self.snapshot(window, CloseEnd::Hide);
        self.unmap_with_snapshot(window, closing);
    }

    /// Reads the picture a window keeps when it leaves: its buffer's pixels as it was drawn.
    /// `None` when there is no picture to read, or when motion is reduced, so the window is
    /// hidden or removed at once.
    fn snapshot(&self, window: WindowId, on_end: CloseEnd) -> Option<Closing> {
        if self.reduced_motion {
            return None;
        }
        let entry = self.windows.get(&window)?;
        let buffer = entry.buffer.as_ref()?;
        let content = entry.content?;
        let image = shm::readback(buffer).ok()?;
        Some(Closing {
            image: Arc::new(image),
            content,
            on_end,
        })
    }

    /// The body of [`Composer::close_window`]: fades the picture `closing` keeps, or removes
    /// the window when there is none.
    fn close_with_snapshot(&mut self, window: WindowId, closing: Option<Closing>) {
        let node = self.windows.get(&window).and_then(|entry| entry.node);
        match (closing, node) {
            (Some(closing), Some(node)) => self.fade_out(window, node, closing),
            _ => self.remove_window(window),
        }
    }

    /// The body of [`Composer::unmap_window`]: fades the picture `closing` keeps, or drops the
    /// window's content so its node goes at once.
    fn unmap_with_snapshot(&mut self, window: WindowId, closing: Option<Closing>) {
        if self.is_fading(window) {
            return;
        }
        let node = self.windows.get(&window).and_then(|entry| entry.node);
        match (closing, node) {
            (Some(closing), Some(node)) => self.fade_out(window, node, closing),
            _ => {
                if let Some(entry) = self.windows.get_mut(&window) {
                    entry.content = None;
                    entry.buffer = None;
                }
                self.sync(window, true);
            }
        }
    }

    /// Starts an unmap or destroy fade. The content and buffer are dropped, so the picture is the
    /// only pixels kept; the client commits again before the window is drawn from a buffer.
    fn fade_out(&mut self, window: WindowId, node: NodeId, closing: Closing) {
        if let Some(entry) = self.windows.get_mut(&window) {
            entry.content = None;
            entry.buffer = None;
        }
        self.start_fade(window, node, closing);
    }

    /// Starts a window's fade. Its node keeps drawing `closing`'s picture while its opacity
    /// tweens to zero; the fade's end is applied by [`Composer::finish_fade`].
    ///
    /// The fade is an opacity track to zero on the window's node, so the scene damages the
    /// rectangle on every step and the frames repaint it until the track ends. The window's
    /// content is left as it is: a minimised window keeps it to draw when it is shown again.
    fn start_fade(&mut self, window: WindowId, node: NodeId, closing: Closing) {
        self.cancel_tracks(window);
        let on_end = closing.on_end;
        if let Some(entry) = self.windows.get_mut(&window) {
            entry.closing = Some(closing);
            entry.leaving = false;
            entry.slide = 0.0;
        }
        let spec = TrackSpec {
            from: TrackStart::Current,
            duration_ms: MOTION_MS,
            easing: Easing::EaseOutCubic,
            ..TrackSpec::new(node, VisualProperty::Opacity, PropertyValue::number(0.0))
        };
        match self.motion.start(&mut self.scene, spec) {
            Ok(track) => {
                self.tracks.insert((window, VisualProperty::Opacity), track);
            }
            Err(_) => self.finish_fade(window, on_end),
        }
    }

    /// Applies a window's closing once its fade has ended: a destroyed window is forgotten, and
    /// an unmapped one loses its node but stays known.
    fn finish_fade(&mut self, window: WindowId, on_end: CloseEnd) {
        match on_end {
            CloseEnd::Remove => self.remove_window(window),
            CloseEnd::Hide => {
                if let Some(entry) = self.windows.get_mut(&window) {
                    entry.closing = None;
                }
                self.sync(window, false);
            }
        }
    }

    /// Whether a window is fading out after it was closed or unmapped.
    fn is_fading(&self, window: WindowId) -> bool {
        self.windows.get(&window).is_some_and(|entry| entry.closing.is_some())
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
    ///
    /// Hiding fades the window's last picture out, as an unmap does, so a minimise is seen rather
    /// than cut. Showing it again while that fade runs eases it back to opaque on the same node.
    /// The window's content is kept either way, so the next show draws what the client last
    /// committed.
    ///
    /// During a workspace switch ([`Composer::begin_workspace_slide`]) the window slides instead:
    /// a hidden window slides off the side the new workspace is not on, and a shown one slides in
    /// from that side.
    pub fn set_window_visible(&mut self, window: WindowId, visible: bool) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        if entry.visible == visible {
            return;
        }
        if visible {
            entry.visible = true;
            if entry.leaving {
                // Shown again before its slide landed: it slides back from where it is.
                entry.leaving = false;
                entry.slide = 0.0;
                self.sync_with(window, false, true);
                return;
            }
            if self.cancel_hide_fade(window) {
                return;
            }
            match self.workspace_slide {
                Some(slide) => self.show_sliding(window, slide),
                None => self.sync(window, false),
            }
            return;
        }
        if let Some(slide) = self.workspace_slide
            && self.slides_off(window)
        {
            self.slide_off(window, slide);
            return;
        }
        let closing = if self.is_fading(window) {
            None
        } else {
            self.snapshot(window, CloseEnd::Hide)
        };
        self.hide_with_snapshot(window, closing);
    }

    /// The body of [`Composer::set_window_visible`] when hiding: records the window as hidden,
    /// then fades its picture out if there is one, or drops its node at once.
    ///
    /// A window that is already fading for an unmap or a destroy keeps that fade, which ends in
    /// a node that stays gone while the window is hidden.
    fn hide_with_snapshot(&mut self, window: WindowId, closing: Option<Closing>) {
        if let Some(entry) = self.windows.get_mut(&window) {
            entry.visible = false;
        }
        if self.is_fading(window) {
            return;
        }
        let node = self.windows.get(&window).and_then(|entry| entry.node);
        match (closing, node) {
            (Some(closing), Some(node)) => self.start_fade(window, node, closing),
            _ => self.sync(window, false),
        }
    }

    /// Shows a window that a workspace switch brings in. A window with no node is drawn at the
    /// side it enters from and eases to its place. A window still drawn has no node to slide, so
    /// it stays where it is.
    fn show_sliding(&mut self, window: WindowId, slide: WorkspaceSlide) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        if self.reduced_motion || entry.node.is_some() {
            self.sync(window, false);
            return;
        }
        entry.slide = slide.incoming;
        self.sync_with(window, false, false);
        if let Some(entry) = self.windows.get_mut(&window) {
            entry.slide = 0.0;
        }
        self.retarget(window, true);
    }

    /// Whether hiding a window starts a workspace slide: a drawn window that is not fading away.
    fn slides_off(&self, window: WindowId) -> bool {
        !self.reduced_motion
            && !self.is_fading(window)
            && self.windows.get(&window).is_some_and(|entry| entry.node.is_some())
    }

    /// Hides a window by sliding it off the side the workspace switch leaves. It keeps its node
    /// and its picture while it moves, and loses the node when the slide lands.
    fn slide_off(&mut self, window: WindowId, slide: WorkspaceSlide) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        entry.visible = false;
        entry.leaving = true;
        entry.slide = slide.outgoing;
        self.sync_with(window, false, true);
        if !self.tracks.contains_key(&(window, VisualProperty::OffsetX)) {
            self.settle_leaving(window);
        }
    }

    /// Ends a window's slide off the workspace it left: it is no longer kept, so its node goes.
    fn settle_leaving(&mut self, window: WindowId) {
        if let Some(entry) = self.windows.get_mut(&window) {
            entry.leaving = false;
            entry.slide = 0.0;
        }
        self.sync(window, false);
    }

    /// Starts a workspace switch. Until [`Composer::end_workspace_slide`], the windows that
    /// [`Composer::set_window_visible`] hides slide off the side the new workspace is not on, and
    /// the windows it shows slide in from that side, each by one output width.
    /// `Later` is the switch to the next workspace, which lies to the right: the windows being left
    /// slide left and the new ones come in from the right. Reduced motion starts no slide: the
    /// windows change at once.
    pub fn begin_workspace_slide(&mut self, direction: WorkspaceDirection) {
        if self.reduced_motion {
            self.workspace_slide = None;
            return;
        }
        let distance = self
            .scene
            .outputs()
            .values()
            .map(|rect| f64::from(rect.width))
            .fold(0.0, f64::max);
        let (outgoing, incoming) = match direction {
            WorkspaceDirection::Later => (-distance, distance),
            WorkspaceDirection::Earlier => (distance, -distance),
        };
        self.workspace_slide = Some(WorkspaceSlide { outgoing, incoming });
    }

    /// Ends the workspace switch begun by [`Composer::begin_workspace_slide`]. Slides already
    /// running finish on their own.
    pub fn end_workspace_slide(&mut self) {
        self.workspace_slide = None;
    }

    /// Marks a window's surface secure, or clears the mark.
    ///
    /// A secure surface is masked in a capture, and the frame that a capture reads back
    /// is kept only when it has a secure layer. The mark changes what a capture shows
    /// without changing the pixels on screen, so the window is damaged with force: the
    /// frame is redrawn and the capture reads the new mark rather than a stale frame.
    pub fn set_window_secure(&mut self, window: WindowId, secure: bool) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        if entry.secure == secure {
            return;
        }
        entry.secure = secure;
        self.sync(window, true);
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

    /// Sets a window's opacity from a matched window rule. It multiplies into the group
    /// opacity beside the placement opacity and the client's surface factor.
    ///
    /// Placement writes `transform.opacity` on every layout, so a rule keeps its own factor:
    /// writing the rule's value there would be overwritten by the next placement.
    pub fn set_window_rule_opacity(&mut self, window: WindowId, opacity: f32) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        let opacity = if opacity.is_finite() {
            opacity.clamp(0.0, 1.0)
        } else {
            1.0
        };
        if entry.rule_opacity == opacity {
            return;
        }
        entry.rule_opacity = opacity;
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
        let mut base = base;
        if !is_drawable_scale(base.scale) {
            log::warn!(area = "render", %window, "a window's scale is refused: invalid-scale; keeping its previous scale");
            base.scale = entry.transform.base.scale;
        }
        entry.transform.base = base;
        self.sync_animated(window);
    }

    /// Sets whether property changes land at once (`true`) or are animated (`false`).
    ///
    /// It applies to changes started after the call; a track already running finishes.
    pub fn set_reduced_motion(&mut self, reduced: bool) {
        self.reduced_motion = reduced;
    }

    /// Starts a visual bell's flash over `window`, or over every output when `window` is `None`
    /// or not one the composer holds.
    ///
    /// A flash already running is replaced. With reduced motion the flash is drawn once at its
    /// peak and then ends with the frame that draws it; otherwise it fades out over [`FLASH_MS`].
    pub fn flash_window(&mut self, window: Option<WindowId>) {
        let area = window
            .and_then(|window| self.windows.get(&window))
            .map(|entry| entry.geometry);
        let flash = Flash {
            area,
            elapsed_ms: 0,
            reduced: self.reduced_motion,
        };
        if let Some(old) = self.flash.replace(flash) {
            self.damage_flash(old);
        }
        self.damage_flash(flash);
    }

    /// Advances every running animation by `elapsed_ms` and forgets the ones that ended.
    ///
    /// It also moves a visual bell's flash on, so the next frame shows its fade.
    pub fn advance_motion(&mut self, elapsed_ms: u32) {
        self.motion.advance(&mut self.scene, elapsed_ms);
        for end in self.motion.take_ended() {
            let owner = self
                .tracks
                .iter()
                .find(|(_, track)| **track == end.track)
                .map(|(key, _)| *key);
            self.tracks.retain(|_, track| *track != end.track);
            if let Some((window, VisualProperty::Opacity)) = owner
                && let Some(on_end) = self
                    .windows
                    .get(&window)
                    .and_then(|entry| entry.closing.as_ref())
                    .map(|closing| closing.on_end)
            {
                self.finish_fade(window, on_end);
            }
            if let Some((window, VisualProperty::OffsetX)) = owner
                && self.windows.get(&window).is_some_and(|entry| entry.leaving)
            {
                self.settle_leaving(window);
            }
        }
        self.advance_flash(elapsed_ms);
    }

    /// Whether a visual bell's flash is still on screen, so the caller keeps frames coming.
    #[must_use]
    pub fn flash_running(&self) -> bool {
        self.flash.is_some()
    }

    /// Moves the flash on by `elapsed_ms`, damaging the area it covers so the next frame repaints
    /// it. A reduced-motion flash does not move: it is ended by the frame that draws it, in
    /// [`Composer::end_frame`].
    fn advance_flash(&mut self, elapsed_ms: u32) {
        let Some(mut flash) = self.flash else {
            return;
        };
        if flash.reduced {
            return;
        }
        self.damage_flash(flash);
        flash.elapsed_ms = flash.elapsed_ms.saturating_add(elapsed_ms);
        if flash.elapsed_ms >= FLASH_MS {
            self.flash = None;
        } else {
            self.flash = Some(flash);
        }
    }

    /// Damages the area a flash covers: every output in full when it has no area.
    fn damage_flash(&mut self, flash: Flash) {
        match flash.area {
            Some(area) => self.scene.damage_rect(area),
            None => {
                let outputs: Vec<OutputId> = self.scene.outputs().keys().copied().collect();
                for output in outputs {
                    self.scene.damage_output(output);
                }
            }
        }
    }

    /// The alpha a flash is drawn at: its peak, fading to nothing over [`FLASH_MS`] unless motion
    /// is reduced.
    fn flash_alpha(flash: Flash) -> f32 {
        if flash.reduced {
            return FLASH_PEAK;
        }
        let remaining = FLASH_MS.saturating_sub(flash.elapsed_ms);
        FLASH_PEAK * (remaining as f32 / FLASH_MS as f32)
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

    /// Replaces a window's content, or clears it.
    ///
    /// A commit always damages the window's rectangle: the client may have drawn
    /// into the same buffer, and nothing in the handle says whether its pixels
    /// changed. A commit of content to a window that is fading out after an unmap cancels the
    /// fade: the window eases back to opaque on the node it kept, rather than starting over
    /// with a new one. A window being destroyed is not revived.
    pub fn set_window_content(&mut self, window: WindowId, content: Option<Content>) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        entry.content = content;
        if content.is_some() && self.cancel_hide_fade(window) {
            return;
        }
        self.sync(window, true);
    }

    /// Stops a window's hide fade because the window is shown again, so its node takes the content
    /// it is given rather than fading out. Returns whether a fade was stopped.
    ///
    /// Only a visible window stops its fade: a window the workspace hid keeps fading out while
    /// its client commits or maps again, so it is not revived against the workspace's wish. A
    /// destroy fade is not stopped: the client cannot show a window it has destroyed. The fade's
    /// picture is dropped here, so the node is re-synced to the window's state.
    fn cancel_hide_fade(&mut self, window: WindowId) -> bool {
        let Some(entry) = self.windows.get(&window) else {
            return false;
        };
        let hide_fading = entry.visible
            && entry
                .closing
                .as_ref()
                .is_some_and(|closing| closing.on_end == CloseEnd::Hide);
        if !hide_fading {
            return false;
        }
        if let Some(entry) = self.windows.get_mut(&window) {
            entry.closing = None;
        }
        self.cancel_tracks(window);
        self.sync_with(window, true, true);
        true
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
            let (source, content) = if let Some(closing) = &entry.closing {
                (SurfaceSource::Snapshot(Arc::clone(&closing.image)), closing.content)
            } else {
                let (Some(content), Some(buffer)) = (entry.content, entry.buffer.as_ref()) else {
                    // A window whose client has committed content but no buffer has no
                    // pixels to draw.
                    continue;
                };
                (SurfaceSource::Buffer(buffer.clone()), content)
            };
            // The window's placed rectangle is the scene's, mid-tween, not its target.
            let Some(pose) = self.pose(*window) else {
                continue;
            };
            let dst = Rect::new(
                pose.bounds.x.saturating_sub(rect.x),
                pose.bounds.y.saturating_sub(rect.y),
                pose.bounds.width,
                pose.bounds.height,
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
            if pose.transform.base.rotation_degrees.z != 0.0 {
                // Pixman's frame API rotates the whole destination rather than one
                // layer, so a rotated window would appear as its bounding box. The
                // scene's damage already covers the rotated bounds, so skipping the
                // layer repaints those pixels with the background rather than leaving
                // stale ones.
                log::debug!(area = "render", %window, "a rotated window is not drawn by the software renderer");
                continue;
            }
            frame.push_layer(Layer::Surface(Surface {
                source,
                src: content.src,
                dst,
                transform: content.transform,
                opacity: pose.transform.opacity * entry.surface_alpha * entry.rule_opacity,
                tint: pose.transform.tint,
                corner_radius: pose.transform.corner_radius,
                effects: entry.effects.clone(),
                model,
                secure: entry.secure,
            }));
        }
        if let Some(flash) = self.flash {
            // The bell's flash is drawn over every window, on the part of this output it covers.
            let covered = match flash.area {
                Some(area) => area.intersection(rect),
                None => Some(rect),
            };
            if let Some(covered) = covered {
                let colour = Color::new(1.0, 1.0, 1.0, Self::flash_alpha(flash));
                frame.push_layer(Layer::Solid(Solid::new(local(covered, rect), colour)));
            }
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
            f64::from(entry.geometry.x) + entry.slide,
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

        if entry.leaving && !animate {
            // A leaving window's slide is a track already running. Landing the target now would
            // cancel it and leave the window at its outgoing offset with no track to end the leave.
            return;
        }
        let immediate = !animate || self.reduced_motion;
        let mut landing = Vec::new();
        let target = Self::target(entry);
        for property in VisualProperty::ALL {
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
        if entry.closing.is_some() {
            // A closing window keeps its node and picture until its fade ends. Only its
            // stacking position still follows the policy.
            let node = entry.node;
            let z_order = self.z_order(window);
            if let Some(node) = node
                && self.scene.z_order(node) != Some(z_order)
            {
                let _ = self.scene.set_z_order(node, z_order);
            }
            return;
        }

        if entry.leaving && !entry.visible {
            // A leaving window keeps its node only while its slide runs. Without content of the
            // node's size the node cannot be kept, so the slide ends at once and the node goes.
            let fits = entry.content.is_some_and(|content| {
                entry
                    .node
                    .is_some_and(|node| self.scene.bounds(node) == Some(Rect::from_size(content.size)))
            });
            if !fits {
                self.settle_leaving(window);
                return;
            }
        }
        let content = if entry.visible || entry.leaving {
            entry.content
        } else {
            None
        };
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
                // new rectangle are damaged. A new node takes its target at once,
                // except that a window which maps fades in from transparent.
                if let Some(node) = existing {
                    self.cancel_tracks(window);
                    let _ = self.scene.remove(node);
                }
                let fades_in = existing.is_none() && !self.reduced_motion;
                let mut spec_transform = transform;
                if fades_in {
                    spec_transform.opacity = 0.0;
                }
                let id = NodeId::new(self.next_node);
                self.next_node = self.next_node.saturating_add(1);
                let spec = NodeSpec {
                    parent: None,
                    z_order,
                    bounds,
                    transform: spec_transform,
                    effects,
                    kind: NodeKind::Visual,
                };
                if self.scene.insert(id, spec).is_ok() {
                    if let Some(entry) = self.windows.get_mut(&window) {
                        entry.node = Some(id);
                    }
                    if fades_in {
                        self.retarget(window, true);
                    }
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

    /// A window's node as the scene holds it now: its placed rectangle and its transform,
    /// both mid-tween while a property animates. `None` when the window has no node.
    fn pose(&self, window: WindowId) -> Option<Pose> {
        let node = self.windows.get(&window)?.node?;
        let bounds = self.scene.content_bounds(node)?.to_pixel();
        let transform = *self.scene.transform(node)?;
        Some(Pose { bounds, transform })
    }
}

/// A global rectangle in an output's own coordinates.
fn local(rect: Rect, output: Rect) -> Rect {
    rect.translate(-output.x, -output.y)
}

/// A window's node as the scene holds it this frame: the rectangle it is placed at on the
/// output plane and its transform, both of which a running tween moves.
struct Pose {
    bounds: Rect,
    transform: VisualTransform,
}

/// Whether a scale can be drawn at all. The renderer draws a window at its scaled bounds, so
/// a zero component collapses it, a non-finite one has no bounds, and a negative one would
/// mirror content that the software renderer does not flip. Each is refused as `invalid-scale`.
fn is_drawable_scale(scale: Vec3) -> bool {
    [scale.x, scale.y, scale.z]
        .iter()
        .all(|component| component.is_finite() && *component > 0.0)
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

    // A secure mark redraws the window, so a capture reads the new mark rather than a
    // frame drawn before it. Setting the same mark again changes nothing and damages nothing.
    #[test]
    fn a_secure_mark_damages_the_window_only_when_it_changes() {
        let (mut composer, output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(10, 10, 100, 100));
        composer.set_window_content(window, Some(content()));
        assert!(damage_of(&mut composer, output).is_some(), "the new node is drawn once");
        assert_eq!(
            damage_of(&mut composer, output),
            None,
            "a steady window has nothing to draw"
        );

        composer.set_window_secure(window, true);
        assert!(
            damage_of(&mut composer, output).is_some(),
            "marking secure redraws the window"
        );
        composer.set_window_secure(window, true);
        assert_eq!(damage_of(&mut composer, output), None, "the same mark damages nothing");

        composer.set_window_secure(window, false);
        assert!(
            damage_of(&mut composer, output).is_some(),
            "clearing the mark redraws the window"
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

    /// A picture the size of `content()`, for a closing window to keep.
    fn picture() -> Arc<Image> {
        let bytes = vec![0xff; 40 * 50 * 4];
        Arc::new(Image::new(Size::new(40, 50), 40 * 4, bytes).expect("a picture the size of its window"))
    }

    #[test]
    fn a_window_with_no_picture_to_keep_is_removed_at_once() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.close_window(window);
        assert!(
            !composer.windows.contains_key(&window),
            "a window with no buffer to read has nothing to fade"
        );
    }

    #[test]
    fn a_closing_window_is_drawn_from_its_picture() {
        let (mut composer, output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.advance_motion(MOTION_MS);
        composer.close_with_snapshot(
            window,
            Some(Closing {
                image: picture(),
                content: content(),
                on_end: CloseEnd::Remove,
            }),
        );
        composer.begin_frame();
        let mut frame = OutputFrame::new();
        assert!(composer.compose(output, &mut frame));
        composer.end_frame();
        let drawn = frame.layers().iter().any(|layer| {
            matches!(
                layer,
                Layer::Surface(Surface {
                    source: SurfaceSource::Snapshot(_),
                    ..
                })
            )
        });
        assert!(
            drawn,
            "the closing window is drawn from the picture it kept, not a buffer it no longer has"
        );
    }

    #[test]
    fn a_closing_window_is_repainted_as_its_fade_runs() {
        let (mut composer, output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.advance_motion(MOTION_MS);
        damage_of(&mut composer, output);
        composer.close_with_snapshot(
            window,
            Some(Closing {
                image: picture(),
                content: content(),
                on_end: CloseEnd::Remove,
            }),
        );
        damage_of(&mut composer, output);
        composer.advance_motion(MOTION_MS / 2);
        let rects = damage_of(&mut composer, output).expect("the fade repaints the pixels it covers");
        assert!(
            rects.iter().any(|rect| rect.contains(Point::new(20, 30))),
            "the fading window's rectangle is repainted: {rects:?}"
        );
    }

    #[test]
    fn a_closing_window_is_removed_once_its_fade_ends() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.advance_motion(MOTION_MS);
        composer.close_with_snapshot(
            window,
            Some(Closing {
                image: picture(),
                content: content(),
                on_end: CloseEnd::Remove,
            }),
        );
        composer.advance_motion(MOTION_MS / 2);
        assert!(composer.windows.contains_key(&window), "the window is still fading");
        composer.advance_motion(MOTION_MS);
        assert!(
            !composer.windows.contains_key(&window),
            "the window is gone once its fade ends"
        );
    }

    // A client that unmaps (commits a null buffer) keeps its window. Its last picture fades
    // out, then only the node goes: the window stays known, so a re-map needs no new identity.
    #[test]
    fn an_unmapped_window_fades_then_loses_its_node() {
        let (mut composer, output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.advance_motion(MOTION_MS);
        composer.unmap_with_snapshot(
            window,
            Some(Closing {
                image: picture(),
                content: content(),
                on_end: CloseEnd::Hide,
            }),
        );
        composer.begin_frame();
        let mut frame = OutputFrame::new();
        assert!(composer.compose(output, &mut frame));
        composer.end_frame();
        assert!(
            frame.layers().iter().any(|layer| matches!(
                layer,
                Layer::Surface(Surface {
                    source: SurfaceSource::Snapshot(_),
                    ..
                })
            )),
            "the unmapped window fades from the picture it kept"
        );
        composer.advance_motion(MOTION_MS / 2);
        assert!(composer.window_matrix(window).is_some(), "still fading");
        composer.advance_motion(MOTION_MS);
        assert!(composer.windows.contains_key(&window), "the window is still known");
        assert!(
            composer.window_matrix(window).is_none(),
            "its node is gone once the fade ends"
        );
        assert!(composer.tracks.is_empty(), "no track outlives the fade");
    }

    #[test]
    fn an_unmapped_window_with_no_picture_is_hidden_at_once() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.unmap_with_snapshot(window, None);
        assert!(composer.window_matrix(window).is_none(), "nothing to fade, so no node");
        assert!(composer.windows.contains_key(&window));
    }

    // A minimised window fades out like an unmapped one, but keeps its picture and content so
    // that restoring it can ease back. Its node goes when the fade ends; the window stays known.
    #[test]
    fn a_minimised_window_fades_out_then_loses_its_node_and_keeps_its_content() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.advance_motion(MOTION_MS);
        composer.hide_with_snapshot(
            window,
            Some(Closing {
                image: picture(),
                content: content(),
                on_end: CloseEnd::Hide,
            }),
        );
        assert!(
            !composer.windows[&window].visible,
            "the window is hidden from the start"
        );
        composer.advance_motion(MOTION_MS / 2);
        assert!(composer.window_matrix(window).is_some(), "still fading");
        composer.advance_motion(MOTION_MS * 2);
        assert!(composer.windows.contains_key(&window), "the window is still known");
        assert!(
            composer.window_matrix(window).is_none(),
            "its node is gone once the fade ends"
        );
        assert!(
            composer.windows[&window].content.is_some(),
            "its content is kept to restore"
        );
        assert!(composer.tracks.is_empty(), "no track outlives the fade");
    }

    // Showing a minimised window mid-fade stops the fade on the node it kept: the node eases
    // back to full opacity and is not removed when the old fade would have ended.
    #[test]
    fn a_minimised_window_restored_mid_fade_eases_back_to_opaque() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.advance_motion(MOTION_MS);
        let node = composer.windows[&window].node.expect("a node");
        composer.hide_with_snapshot(
            window,
            Some(Closing {
                image: picture(),
                content: content(),
                on_end: CloseEnd::Hide,
            }),
        );
        composer.advance_motion(MOTION_MS / 2);
        composer.set_window_visible(window, true);
        assert!(composer.windows[&window].closing.is_none(), "the fade is stopped");
        assert_eq!(composer.windows[&window].node, Some(node), "the same node");
        composer.advance_motion(MOTION_MS * 2);
        assert!(composer.windows.contains_key(&window));
        assert_eq!(
            composer.windows[&window].node,
            Some(node),
            "the node survives the old fade's end"
        );
        assert!(composer.tracks.is_empty(), "the stopped fade left no track");
        assert_eq!(
            composer.scene.transform(node).map(|transform| transform.opacity),
            Some(1.0),
            "the window is opaque again"
        );
    }

    // With motion reduced there is no picture to fade from, so the window is hidden at once.
    #[test]
    fn a_minimised_window_with_reduced_motion_is_hidden_at_once() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.set_reduced_motion(true);
        composer.set_window_visible(window, false);
        assert!(composer.window_matrix(window).is_none(), "no node once hidden");
        assert!(composer.windows[&window].content.is_some(), "the content is kept");
    }

    // A client that keeps committing while its window is minimised must not revive it: only a
    // window the workspace shows again can stop its fade.
    #[test]
    fn a_commit_during_a_minimise_fade_does_not_revive_the_window() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.advance_motion(MOTION_MS);
        composer.hide_with_snapshot(
            window,
            Some(Closing {
                image: picture(),
                content: content(),
                on_end: CloseEnd::Hide,
            }),
        );
        composer.advance_motion(MOTION_MS / 2);
        composer.set_window_content(window, Some(content()));
        assert!(composer.windows[&window].closing.is_some(), "the fade continues");
        composer.advance_motion(MOTION_MS * 2);
        assert!(composer.window_matrix(window).is_none(), "the window stays hidden");
    }

    // A window hidden while it is being destroyed still goes: the destroy's end removes it.
    #[test]
    fn a_destroy_during_a_minimise_fade_removes_the_window_when_it_ends() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.advance_motion(MOTION_MS);
        composer.hide_with_snapshot(
            window,
            Some(Closing {
                image: picture(),
                content: content(),
                on_end: CloseEnd::Hide,
            }),
        );
        composer.close_window(window);
        composer.advance_motion(MOTION_MS * 2);
        assert!(
            !composer.windows.contains_key(&window),
            "the destroyed window is forgotten"
        );
    }

    // A window that changes size while minimised is drawn at its new size once shown again.
    #[test]
    fn a_window_resized_while_minimised_is_drawn_at_its_new_size_when_restored() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.advance_motion(MOTION_MS);
        composer.hide_with_snapshot(
            window,
            Some(Closing {
                image: picture(),
                content: content(),
                on_end: CloseEnd::Hide,
            }),
        );
        let bigger = Content {
            src: RectF::new(0.0, 0.0, 80.0, 90.0),
            size: Size::new(80, 90),
            transform: Transform::Normal,
        };
        composer.set_window_content(window, Some(bigger));
        composer.advance_motion(MOTION_MS * 2);
        assert!(composer.window_matrix(window).is_none(), "still hidden");
        composer.set_window_visible(window, true);
        assert_eq!(
            drawn_pose(&composer, window).0,
            Rect::new(20, 30, 80, 90),
            "the new size"
        );
    }

    // Mapping again during the fade cancels it: the node is the same one, it eases back to
    // full opacity, and the fade's end does not remove the window.
    #[test]
    fn a_window_that_maps_again_while_fading_cancels_the_fade() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.advance_motion(MOTION_MS);
        let node = composer.windows[&window].node.expect("a node");
        composer.unmap_with_snapshot(
            window,
            Some(Closing {
                image: picture(),
                content: content(),
                on_end: CloseEnd::Hide,
            }),
        );
        composer.advance_motion(MOTION_MS / 2);
        composer.set_window_content(window, Some(content()));
        assert!(composer.windows[&window].closing.is_none(), "the picture is dropped");
        assert_eq!(composer.windows[&window].node, Some(node), "the same node");
        composer.advance_motion(MOTION_MS * 2);
        assert!(
            composer.windows.contains_key(&window),
            "the old fade's end keeps the window"
        );
        assert_eq!(composer.windows[&window].node, Some(node));
        assert!(composer.tracks.is_empty(), "the cancelled fade left no track");
        assert_eq!(
            composer.scene.transform(node).map(|transform| transform.opacity),
            Some(1.0),
            "the window is opaque again"
        );
    }

    // A destroy that arrives during an unmap fade must not cut the fade short: the picture is
    // still on screen, so it finishes and then the window is removed.
    #[test]
    fn a_destroy_during_an_unmap_fade_finishes_the_fade_then_removes() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.advance_motion(MOTION_MS);
        composer.unmap_with_snapshot(
            window,
            Some(Closing {
                image: picture(),
                content: content(),
                on_end: CloseEnd::Hide,
            }),
        );
        composer.advance_motion(MOTION_MS / 2);
        composer.close_window(window);
        assert!(composer.window_matrix(window).is_some(), "the fade is not cut short");
        composer.advance_motion(MOTION_MS);
        assert!(!composer.windows.contains_key(&window), "removed once the fade ends");
        assert!(composer.tracks.is_empty());
    }

    // Unmapped with nothing to fade, then destroyed: the entry is removed, not kept forever.
    #[test]
    fn an_unmapped_window_destroyed_later_is_forgotten() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.unmap_with_snapshot(window, None);
        composer.close_window(window);
        assert!(
            !composer.windows.contains_key(&window),
            "a destroyed window is forgotten"
        );
        assert!(composer.order.is_empty(), "and it leaves the stacking order");
    }

    // An X11 window is inserted again while its unmap fade runs. The fade stops, and since the
    // window has no picture yet it keeps no node, so nothing is drawn as a ghost.
    #[test]
    fn an_unmapped_window_inserted_again_while_fading_stops_the_fade() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.advance_motion(MOTION_MS);
        composer.unmap_with_snapshot(
            window,
            Some(Closing {
                image: picture(),
                content: content(),
                on_end: CloseEnd::Hide,
            }),
        );
        composer.advance_motion(MOTION_MS / 2);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        assert!(!composer.is_fading(window), "the fade stops");
        assert!(composer.tracks.is_empty(), "the stopped fade left no track");
        assert_eq!(composer.windows[&window].node, None, "no picture, so no node");
        composer.advance_motion(MOTION_MS * 2);
        assert!(composer.windows.contains_key(&window), "the window is still known");
        assert_eq!(composer.windows[&window].node, None, "and still drawn as nothing");
    }
    // A client that unmaps a window with no picture to keep hides it through the public entry
    // point, and an X11 window that maps again reuses the same entry: one slot, one entry, no
    // leak from the repeated inserts.
    #[test]
    fn unmap_and_remap_without_a_picture_keeps_one_entry() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.unmap_window(window);
        assert!(composer.windows.contains_key(&window), "the window stays known");
        assert!(!composer.is_fading(window), "there was no picture to fade");
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        assert_eq!(composer.order, vec![window], "one stacking slot");
        assert_eq!(composer.windows.len(), 1, "one entry");
    }

    // An unmap that arrives after a destroy must not turn the destroy into a hide: the window
    // still leaves when its fade ends.
    #[test]
    fn an_unmap_during_a_destroy_fade_does_not_hide_the_window() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.advance_motion(MOTION_MS);
        composer.close_with_snapshot(
            window,
            Some(Closing {
                image: picture(),
                content: content(),
                on_end: CloseEnd::Remove,
            }),
        );
        composer.unmap_window(window);
        assert_eq!(
            composer.windows[&window].closing.as_ref().map(|closing| closing.on_end),
            Some(CloseEnd::Remove),
            "the destroy still decides the end"
        );
        composer.advance_motion(MOTION_MS * 2);
        assert!(
            !composer.windows.contains_key(&window),
            "the destroyed window is removed"
        );
    }

    // A commit that lands after a destroy fades the old picture and does not bring the window
    // back: the client cannot show a window it has destroyed.
    #[test]
    fn a_commit_during_a_destroy_fade_does_not_revive_the_window() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.advance_motion(MOTION_MS);
        composer.close_with_snapshot(
            window,
            Some(Closing {
                image: picture(),
                content: content(),
                on_end: CloseEnd::Remove,
            }),
        );
        composer.set_window_content(window, Some(content()));
        assert_eq!(
            composer.windows[&window].closing.as_ref().map(|closing| closing.on_end),
            Some(CloseEnd::Remove),
            "the fade is not cancelled"
        );
        composer.advance_motion(MOTION_MS * 2);
        assert!(
            !composer.windows.contains_key(&window),
            "the window is removed at the fade's end"
        );
        assert!(composer.tracks.is_empty(), "no track outlives its window");
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

    // A workspace switch slides the windows it hides off the side the new workspace is not on,
    // and slides the windows it shows in from that side. The slide lasts as long as a fade.
    #[test]
    fn a_window_leaving_a_workspace_slides_off_then_loses_its_node() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.advance_motion(MOTION_MS);
        composer.begin_workspace_slide(WorkspaceDirection::Later);
        composer.set_window_visible(window, false);
        composer.end_workspace_slide();
        assert!(!composer.windows[&window].visible, "the window is hidden");
        assert!(
            composer.window_matrix(window).is_some(),
            "it is still drawn while it slides off"
        );
        composer.advance_motion(MOTION_MS / 2);
        let (bounds, _) = drawn_pose(&composer, window);
        assert!(
            bounds.x < 20,
            "it moves off to the left, away from the workspace shown: {bounds:?}"
        );
        composer.advance_motion(MOTION_MS * 2);
        assert!(
            composer.window_matrix(window).is_none(),
            "its node goes once it has slid off"
        );
        assert!(composer.windows.contains_key(&window), "the window is still known");
        assert!(!composer.windows[&window].leaving, "no leave outlives its slide");
        assert!(composer.tracks.is_empty(), "no track outlives the slide");
    }

    #[test]
    fn a_window_entering_a_workspace_slides_in_from_the_side_it_lies_on() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.advance_motion(MOTION_MS);
        composer.set_window_visible(window, false);
        composer.advance_motion(MOTION_MS * 2);
        assert!(composer.window_matrix(window).is_none(), "a hidden window has no node");
        composer.begin_workspace_slide(WorkspaceDirection::Later);
        composer.set_window_visible(window, true);
        composer.end_workspace_slide();
        assert_eq!(
            drawn_pose(&composer, window).0.x,
            20 + 800,
            "it starts one output width to the right"
        );
        composer.advance_motion(MOTION_MS / 2);
        let middle = drawn_pose(&composer, window).0.x;
        assert!(middle > 20 && middle < 820, "it slides in across the output: {middle}");
        composer.advance_motion(MOTION_MS);
        assert_eq!(
            drawn_pose(&composer, window).0.x,
            20,
            "it lands where its geometry places it"
        );
        assert!(composer.tracks.is_empty(), "no track outlives the slide");
    }

    // Switching again before a slide lands reverses each window from where it is, so neither
    // window jumps, and the window hidden last has no node once the slide ends.
    #[test]
    fn a_quick_double_switch_neither_jumps_nor_leaks_a_node() {
        let (mut composer, _) = screen();
        let leaving = WindowId::new(1);
        let entering = WindowId::new(2);
        for window in [leaving, entering] {
            composer.insert_window(window, Rect::new(20, 30, 40, 50));
            composer.set_window_content(window, Some(content()));
        }
        composer.advance_motion(MOTION_MS);
        composer.set_window_visible(entering, false);
        composer.advance_motion(MOTION_MS * 2);
        composer.begin_workspace_slide(WorkspaceDirection::Later);
        composer.set_window_visible(leaving, false);
        composer.set_window_visible(entering, true);
        composer.end_workspace_slide();
        composer.advance_motion(MOTION_MS / 2);
        let before = [drawn_pose(&composer, leaving).0.x, drawn_pose(&composer, entering).0.x];
        composer.begin_workspace_slide(WorkspaceDirection::Earlier);
        composer.set_window_visible(leaving, true);
        composer.set_window_visible(entering, false);
        composer.end_workspace_slide();
        let after = [drawn_pose(&composer, leaving).0.x, drawn_pose(&composer, entering).0.x];
        assert_eq!(before, after, "reversing mid-slide does not jump");
        composer.advance_motion(MOTION_MS * 2);
        assert_eq!(
            drawn_pose(&composer, leaving).0.x,
            20,
            "the window shown again lands at rest"
        );
        assert!(
            composer.window_matrix(entering).is_none(),
            "the window hidden again has no node"
        );
        assert!(!composer.windows[&entering].leaving, "no leave outlives its slide");
        assert!(composer.tracks.is_empty(), "no track outlives the slides");
    }

    #[test]
    fn a_workspace_switch_with_reduced_motion_does_not_slide() {
        let (mut composer, _) = screen();
        composer.set_reduced_motion(true);
        let leaving = WindowId::new(1);
        let entering = WindowId::new(2);
        for window in [leaving, entering] {
            composer.insert_window(window, Rect::new(20, 30, 40, 50));
            composer.set_window_content(window, Some(content()));
        }
        composer.set_window_visible(entering, false);
        composer.begin_workspace_slide(WorkspaceDirection::Later);
        composer.set_window_visible(leaving, false);
        composer.set_window_visible(entering, true);
        composer.end_workspace_slide();
        assert!(
            composer.window_matrix(leaving).is_none(),
            "a window leaves at once with motion reduced"
        );
        assert_eq!(drawn_pose(&composer, entering).0.x, 20, "and enters at once");
        assert!(composer.tracks.is_empty(), "no slide is started");
    }

    /// The rectangle and opacity `window` is drawn at. Compose builds each surface's destination
    /// from `pose`, so this is what the frame draws. A unit test cannot attach a `WlBuffer`, so
    /// no frame layer exists to inspect; asserting the pose asserts the drawn pose.
    fn drawn_pose(composer: &Composer, window: WindowId) -> (Rect, f32) {
        let pose = composer.pose(window).expect("a window with content has a node");
        (pose.bounds, pose.transform.opacity)
    }

    /// Starts a linear track of `property` on `window`'s node, lasting `duration_ms`.
    fn track_window(
        composer: &mut Composer,
        window: WindowId,
        property: VisualProperty,
        to: PropertyValue,
        duration_ms: u32,
    ) {
        let node = composer.windows[&window].node.expect("a drawn window has a node");
        let spec = TrackSpec {
            from: TrackStart::Current,
            duration_ms,
            easing: Easing::Linear,
            ..TrackSpec::new(node, property, to)
        };
        composer
            .motion
            .start(&mut composer.scene, spec)
            .expect("the track starts");
    }

    #[test]
    fn a_tweened_opacity_is_the_opacity_the_frame_draws() {
        let (mut composer, _output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        // A mapped window fades in from transparent; let that fade land so the tween
        // under test starts from an opaque window.
        composer.advance_motion(200);
        track_window(
            &mut composer,
            window,
            VisualProperty::Opacity,
            PropertyValue::Number(0.5),
            200,
        );
        composer.advance_motion(100);
        assert_eq!(
            drawn_pose(&composer, window),
            (Rect::new(20, 30, 40, 50), 0.75),
            "the frame draws the tween's pose, halfway from 1.0 to 0.5, not its target"
        );
    }

    #[test]
    fn a_window_that_maps_fades_in_from_transparent() {
        let (mut composer, _output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        assert_eq!(
            drawn_pose(&composer, window),
            (Rect::new(20, 30, 40, 50), 0.0),
            "a newly mapped window starts transparent"
        );
        composer.advance_motion(100);
        let (_, halfway) = drawn_pose(&composer, window);
        assert!(
            halfway > 0.0 && halfway < 1.0,
            "the fade is part way through at half its duration, got {halfway}"
        );
        composer.advance_motion(100);
        assert_eq!(
            drawn_pose(&composer, window).1,
            1.0,
            "the fade lands on the window's opacity exactly"
        );
    }

    #[test]
    fn a_window_that_maps_with_reduced_motion_is_opaque_at_once() {
        let (mut composer, _output) = screen();
        composer.set_reduced_motion(true);
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        assert_eq!(
            drawn_pose(&composer, window),
            (Rect::new(20, 30, 40, 50), 1.0),
            "reduced motion writes the target at once"
        );
    }

    #[test]
    fn a_tweened_offset_moves_the_drawn_rectangle() {
        let (mut composer, _output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        track_window(
            &mut composer,
            window,
            VisualProperty::OffsetX,
            PropertyValue::Number(220.0),
            200,
        );
        assert_eq!(
            drawn_pose(&composer, window).0.x,
            20,
            "before the tween the window is at its placement"
        );
        composer.advance_motion(100);
        assert_eq!(
            drawn_pose(&composer, window).0,
            Rect::new(120, 30, 40, 50),
            "halfway along the tween the window is drawn halfway from its placement to its target"
        );
        composer.advance_motion(100);
        assert_eq!(
            drawn_pose(&composer, window).0,
            Rect::new(220, 30, 40, 50),
            "the tween lands on its target"
        );
    }

    #[test]
    fn a_scaled_window_is_drawn_at_its_scaled_bounds() {
        let (mut composer, _output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(100, 100, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.set_reduced_motion(true);
        let mut base = gnoblin_types::Transform::IDENTITY;
        base.scale = Vec3::new(2.0, 2.0, 1.0);
        composer.place_window_animated(window, Rect::new(100, 100, 40, 50), base);
        assert_eq!(
            drawn_pose(&composer, window),
            (Rect::new(80, 75, 80, 100), 1.0),
            "a scale of two is about the window's centre, so the rectangle grows by a quarter each side"
        );
    }

    #[test]
    fn a_scale_tween_passes_through_intermediate_sizes() {
        let (mut composer, _output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(100, 100, 40, 50));
        composer.set_window_content(window, Some(content()));
        let mut base = gnoblin_types::Transform::IDENTITY;
        base.scale = Vec3::new(2.0, 2.0, 1.0);
        composer.place_window_animated(window, Rect::new(100, 100, 40, 50), base);
        composer.advance_motion(100);
        let (dst, _) = drawn_pose(&composer, window);
        assert!(
            dst.width > 40 && dst.width < 80,
            "mid-tween the window is between its two sizes, not at either: {dst:?}"
        );
        let centre_x = dst.x + i32::try_from(dst.width / 2).expect("a width fits in i32");
        assert!(
            (centre_x - 120).abs() <= 1,
            "the tween scales about the centre, which stays at x 120: {dst:?}"
        );
        composer.advance_motion(10_000);
        assert_eq!(
            drawn_pose(&composer, window).0,
            Rect::new(80, 75, 80, 100),
            "the tween lands on the scaled rectangle"
        );
    }

    #[test]
    fn a_zero_or_negative_or_non_finite_scale_is_refused_and_the_window_keeps_its_size() {
        for bad in [
            Vec3::new(0.0, 2.0, 1.0),
            Vec3::new(f64::NAN, 1.0, 1.0),
            Vec3::new(-1.0, 1.0, 1.0),
        ] {
            let (mut composer, _output) = screen();
            let window = WindowId::new(1);
            composer.insert_window(window, Rect::new(100, 100, 40, 50));
            composer.set_window_content(window, Some(content()));
            composer.set_reduced_motion(true);
            let mut base = gnoblin_types::Transform::IDENTITY;
            base.scale = bad;
            composer.place_window_animated(window, Rect::new(100, 100, 40, 50), base);
            assert_eq!(
                drawn_pose(&composer, window),
                (Rect::new(100, 100, 40, 50), 1.0),
                "a scale the scene cannot draw is refused, so the window keeps its size"
            );
            let kept = composer.windows[&window].transform.base.scale;
            assert_eq!((kept.x, kept.y), (1.0, 1.0), "the previous scale is kept");
        }
    }

    #[test]
    fn an_open_tween_draws_monotone_frames_from_transparent_to_opaque() {
        let (mut composer, _output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.set_window_opacity(window, 0.0);
        track_window(
            &mut composer,
            window,
            VisualProperty::Opacity,
            PropertyValue::Number(1.0),
            200,
        );
        let mut opacities = Vec::new();
        for _ in 0..5 {
            opacities.push(drawn_pose(&composer, window).1);
            composer.advance_motion(50);
        }
        assert_eq!(
            opacities,
            vec![0.0, 0.25, 0.5, 0.75, 1.0],
            "a linear open tween steps evenly"
        );
        assert!(
            opacities.windows(2).all(|pair| pair[0] <= pair[1]),
            "every frame is at least as opaque as the one before"
        );
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

    /// The solid layers `output`'s next frame draws, as their rectangle and alpha.
    fn solids_of(composer: &mut Composer, output: OutputId) -> Vec<(Rect, f32)> {
        composer.begin_frame();
        let mut frame = OutputFrame::new();
        composer.compose(output, &mut frame);
        composer.end_frame();
        frame
            .layers()
            .iter()
            .filter_map(|layer| match layer {
                Layer::Solid(solid) => Some((solid.dst, solid.colour.a)),
                Layer::Surface(_) => None,
                Layer::Overlay(_) => None,
            })
            .collect()
    }

    #[test]
    fn a_flash_fades_out_over_its_duration() {
        let (mut composer, output) = screen();
        composer.flash_window(None);
        let first = solids_of(&mut composer, output);
        assert_eq!(first, vec![(Rect::new(0, 0, 800, 600), FLASH_PEAK)]);

        composer.advance_motion(FLASH_MS / 2);
        let halfway = solids_of(&mut composer, output);
        assert_eq!(halfway.len(), 1);
        assert!(
            (halfway[0].1 - FLASH_PEAK / 2.0).abs() < 1e-4,
            "half the peak after half the time"
        );
        assert!(composer.flash_running());

        composer.advance_motion(FLASH_MS);
        assert!(!composer.flash_running(), "the flash ends once its duration has passed");
        assert!(
            solids_of(&mut composer, output).is_empty(),
            "an ended flash draws nothing"
        );
    }

    #[test]
    fn a_flash_damages_every_output_it_covers() {
        let (mut composer, output) = screen();
        composer.flash_window(None);
        let damage = damage_of(&mut composer, output).expect("a flash changes the output");
        assert!(
            covers(&damage, Rect::new(0, 0, 800, 600)),
            "the whole output is repainted"
        );
    }

    #[test]
    fn a_flash_over_a_window_covers_only_that_window() {
        let (mut composer, output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(100, 80, 200, 150));
        composer.flash_window(Some(window));
        assert_eq!(
            solids_of(&mut composer, output),
            vec![(Rect::new(100, 80, 200, 150), FLASH_PEAK)],
            "the flash is drawn over the window's rectangle alone"
        );
        composer.advance_motion(1);
        let damage = damage_of(&mut composer, output).expect("the fade repaints the window");
        assert!(covers(&damage, Rect::new(100, 80, 200, 150)));
        assert!(
            !covers(&damage, Rect::new(0, 0, 800, 600)),
            "the rest of the output is untouched"
        );
    }

    #[test]
    fn a_flash_over_an_unknown_window_falls_back_to_every_output() {
        let (mut composer, output) = screen();
        composer.flash_window(Some(WindowId::new(99)));
        assert_eq!(
            solids_of(&mut composer, output),
            vec![(Rect::new(0, 0, 800, 600), FLASH_PEAK)]
        );
    }

    #[test]
    fn a_reduced_motion_flash_is_drawn_once_at_its_peak() {
        let (mut composer, output) = screen();
        composer.set_reduced_motion(true);
        composer.flash_window(None);
        composer.advance_motion(FLASH_MS * 2);
        assert!(
            composer.flash_running(),
            "a reduced flash waits for the frame that draws it"
        );
        assert_eq!(
            solids_of(&mut composer, output),
            vec![(Rect::new(0, 0, 800, 600), FLASH_PEAK)],
            "reduced motion does not fade the flash"
        );
        assert!(!composer.flash_running(), "the frame that drew it ends it");
        assert!(solids_of(&mut composer, output).is_empty());
    }

    #[test]
    fn a_new_flash_replaces_a_running_one() {
        let (mut composer, output) = screen();
        composer.flash_window(None);
        composer.advance_motion(FLASH_MS / 2);
        composer.flash_window(None);
        let restarted = solids_of(&mut composer, output);
        assert_eq!(
            restarted,
            vec![(Rect::new(0, 0, 800, 600), FLASH_PEAK)],
            "a bell restarts the fade"
        );
    }
}
