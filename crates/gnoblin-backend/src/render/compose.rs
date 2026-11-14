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
//! - **One node per drawn surface of that window.** A window's subsurfaces and popups are nodes under its node
//!   (class `surface`, `role` `subsurface` or `popup`), each placed by its own offset in its parent's plane, so
//!   they follow the window's transform and opacity and are drawn, damaged, captured and hit-tested through the
//!   scene like the window. The window's own node stays the toplevel surface alone: its size, its effects and
//!   its corner radius never reach a popup. See [`Composer::set_window_surfaces`].
//! - **The stacking order**, bottom first: the scene's sibling order, which is also
//!   what its hit test and the frame use, so they agree about what is on top.
//! - **The windows' visual transforms** (opacity, tint), kept here so a node can be
//!   recreated at a new content size without losing the animation playing on it. The
//!   client's and the rule's opacity factors fold into the opacity, so a frame draws each
//!   window's node alone.
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
//! 10. **A popup or a subsurface is not drawn.** The window's surface tree is nodes in the scene, drawn above (or,
//!     for a subsurface placed below its parent, behind) the toplevel surface in the order the client set:
//!     `a_popup_past_the_window_edge_is_drawn_at_its_offset_above_the_window`,
//!     `a_subsurface_below_its_parent_is_drawn_behind_it`.
//! 11. **A secure window shows through its popups.** The window's mark is on every surface of its tree:
//!     `a_popup_of_a_secure_window_is_secure`.
//! 12. **A secure node is judged by what reads it.** Every path asks the one set `Composer::find_secure` makes for the
//!     frame (a marked window, a node whose `secure` is true, and everything below either), so a View of a node below a
//!     secure one, a View or a window below one, and a window's fade are all secure, and a View's capture never reads
//!     the picture drawn before a mark: `a_source_below_a_secure_ancestor_is_redacted_in_the_views_capture_and_the_view_is_a_secure_layer`
//!     and the cases beside it in `view_secure_tests.rs`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use smithay::backend::allocator::Buffer as _;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::utils::Transform;

use gnoblin_animation::{Easing, PropertyValue, SampledCurve, VisualProperty, VisualTransform};
use gnoblin_paint::{SharedPainter, decode};
use gnoblin_render::scene_ids::NodeIdAllocator;
use gnoblin_render::transaction::{Animation, Motion, TreeChange, TreeValue, land_at_once, visual_property};
use gnoblin_render::{
    Damage, Effects, Mat4, NodeKind, NodeSpec, Pass, Projection, Scene, TrackSpec, TrackStart, TweenPlayer,
};
use gnoblin_types::{Color, NodeId, OutputId, Point, PointF, Rect, RectF, Scale, Size, TrackId, Vec3, WindowId};

use gnoblin_render_frame::frame::{Layer, Model, OutputFrame, SecureTreatment, Solid, Surface, SurfaceSource};
use gnoblin_render_frame::image::Image;
use gnoblin_render_frame::shm;
use gnoblin_render_frame::view_scene::{ClipPlane, ViewEye, ViewOp, ViewScene, ViewTexture, size_fits};
use gnoblin_render_software::{
    Frustum, StereoRig, ViewCamera, ViewProjection, ViewRig, eye_pixel_matrix, eye_sizes, render_rig,
};

use crate::log;

/// Reads a client's GPU buffer into a picture, or answers `None` when the backend cannot. A View of a window that
/// shows a GPU buffer draws what this returns: see [`Composer::begin_frame_reading`].
pub type ReadDmabuf<'a> = dyn FnMut(&smithay::backend::allocator::dmabuf::Dmabuf) -> Option<Image> + 'a;

/// How long a placement, move or other visual change takes to reach its target.
const MOTION_MS: u32 = 200;

/// How long a visual bell's flash takes to fade out, in milliseconds.
const FLASH_MS: u32 = 150;

/// The alpha a visual bell's flash starts at, before it fades.
const FLASH_PEAK: f32 = 0.5;

/// The smallest and largest uniform scale the shell may set on a window, as `set_scale` documents.
const SHELL_SCALE_MIN: f32 = 0.01;
const SHELL_SCALE_MAX: f32 = 10.0;

/// The largest magnitude, in logical pixels, of each component the shell may offset a window by.
const SHELL_OFFSET_LIMIT: f64 = 100_000.0;

/// Why a blur or shadow on a node placed in 3D is not drawn: the text after the kind in an entry of the node's
/// read-only `skipped` list, and the code the GLES renderer logs when it meets such a layer.
const SKIPPED_ON_3D: &str = "unsupported-effect-on-3d";
/// The reason a node's effect is skipped when the active renderer draws no effect of that kind.
const SKIPPED_BY_RENDERER: &str = "unsupported-by-renderer";

/// The orthographic camera's distance from the image plane, in logical pixels.
const VIEW_ORTHO_DEPTH: f64 = 100.0;
/// The near and far eye depths a View draws between when its camera sets neither.
const VIEW_NEAR: f64 = 0.1;
const VIEW_FAR: f64 = 10_000.0;
/// The vertical field of view of a perspective camera that sets none, in degrees.
const VIEW_FOV: f64 = 60.0;
/// The scene unit of a camera that sets no `metres`, in metres: a millimetre, as the schema's default.
const VIEW_METRES: f64 = 0.001;
/// The distance between a stereo camera's eyes when it sets none, in metres.
const VIEW_IPD: f64 = 0.063;

/// A View node's rendered picture: the size it was drawn at, the image of its source
/// subtree (or `None` when it has no source or the source cannot be drawn), and the view
/// clock's reading when it was drawn, which `fps` paces the next draw from.
///
/// The image is what the display shows. A capture never reads it when the source holds a `secure` node: it reads
/// `redacted`, the same subtree drawn with each secure node replaced the way an output capture replaces a secure
/// layer ([`SecureTreatment`]). `secure` names those nodes as they were when the picture was drawn, so a node that
/// becomes secure, or stops being, draws the View again at once.
#[derive(Debug)]
struct ViewPicture {
    size: Size,
    /// The picture on the CPU, when the composer drew it there.
    image: Option<Arc<Image>>,
    /// The picture a capture reads when the source holds a secure node, or `None` when it holds none and `image` is
    /// the capture's picture too.
    redacted: Option<Arc<Image>>,
    /// The scene the GPU draws the picture from, when the active renderer draws a View's subtree itself. Then
    /// `image` and `redacted` are `None`: the picture is a texture, and the pixels reach the CPU only when a capture
    /// reads them (`Backend::read_view`).
    scene: Option<Arc<ViewScene>>,
    /// The nodes of the source subtree a capture may not show: each node marked secure, and everything below it.
    secure: BTreeSet<NodeId>,
    drawn_at_ms: u64,
}

/// A Canvas node's rendered picture: what it was painted from, and the image, or `None` when its content paints
/// nothing (empty, a zero-size node, or too large to draw).
///
/// The picture is kept until the node's `content` is written again, its size changes, or the scale of the output it
/// is on does. `revision` is the scene's count of byte writes to the node, so a rewrite of equal bytes repaints too:
/// comparing bytes every frame would cost more than the rare repeated write.
#[derive(Debug)]
struct CanvasPicture {
    revision: u64,
    /// The node's size in logical pixels when it was painted.
    size: Size,
    /// Physical pixels per logical pixel the picture was painted at. The image is `size * scale` pixels, and the
    /// renderer draws it over the node's logical rectangle.
    scale: Scale,
    image: Option<Arc<Image>>,
}

/// Why the composer refused a shell visual value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellVisualError {
    /// The value is not finite or lies outside the range the window methods document.
    OutOfRange,
}

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

/// What a surface of a window's tree is, besides the window's own toplevel surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeSurfaceRole {
    /// A `wl_subsurface` of the toplevel surface, of a popup or of another subsurface.
    Subsurface,
    /// An `xdg_popup`.
    Popup,
}

impl TreeSurfaceRole {
    /// The `role` the surface's node publishes to the tree.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Subsurface => "subsurface",
            Self::Popup => "popup",
        }
    }
}

/// One surface of a window's tree below its toplevel surface: what it is, where it sits and what it shows.
///
/// The list a window is given ([`Composer::set_window_surfaces`]) is in tree order: a surface comes after its
/// parent, and the surfaces that share a parent are in the order they paint, bottom first.
#[derive(Clone, Debug)]
pub struct TreeSurface {
    /// Names the surface among the others of the window. It is stable for the surface's life, so the surface keeps
    /// its node across lists.
    pub key: u64,
    /// The `key` of the surface this one is placed against, or `None` for the window's toplevel surface.
    pub parent: Option<u64>,
    /// What the surface is.
    pub role: TreeSurfaceRole,
    /// Whether the surface paints before its parent's own pixels, which is a subsurface placed below its parent.
    pub behind: bool,
    /// The surface's top left corner in its parent's plane, in logical pixels.
    pub offset: Point,
    /// The `wp_alpha_modifier_v1` factor the surface committed, one when it set none. It multiplies the opacity
    /// of everything under the surface too.
    pub alpha: f32,
    /// What the surface committed.
    pub content: Content,
    /// Where its pixels come from.
    pub source: SurfaceSource,
    /// Whether the surface committed since the last list, so its pixels may have changed under the same buffer.
    pub damaged: bool,
}

/// A surface of a window's tree that has a scene node.
#[derive(Clone, Copy, Debug)]
struct TreeNode {
    node: NodeId,
    /// The parent surface the node was made under, so a surface that moves to another parent is made again.
    parent: Option<u64>,
    role: TreeSurfaceRole,
    /// The position and size last published to the tree, as `[x, y, width, height]`; NaN before the first.
    published: [f64; 4],
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
    /// The pixels the window keeps: a copy of its buffer, read back when the window was closed, or a client's GPU
    /// buffer, which the handle keeps alive with its own plane descriptors and a GPU renderer draws as it drew the
    /// window.
    source: SurfaceSource,
    /// The crop and transform the window was drawn with, so the picture matches it.
    content: Content,
    /// What the window becomes when the fade ends.
    on_end: CloseEnd,
}

/// One managed window's rendering state.
#[derive(Clone, Debug)]
struct WindowNode {
    /// The scene node, while the window is drawn.
    node: Option<NodeId>,
    /// The window's frame rectangle in global logical coordinates.
    geometry: Rect,
    /// Where the window geometry the client set (`xdg_surface.set_window_geometry`) starts inside its surface, in
    /// surface-local pixels; zero when it set none. `geometry` is where that window geometry sits, so the surface
    /// itself is drawn at `geometry` less this offset: a client that draws a shadow around its window has the
    /// window placed, and hit, by the part inside the shadow.
    surface_offset: (i32, i32),
    /// Its visual transform, kept across node recreation.
    transform: VisualTransform,
    /// The client's `wp_alpha_modifier_v1` factor for its surface, one when it set none.
    /// `target` folds it into the node's opacity.
    surface_alpha: f32,
    /// The opacity a matched window rule sets, one when none does. `target` folds it into the
    /// node's opacity beside the placement's, which the layout rewrites on every placement.
    rule_opacity: f32,
    /// The shell's opacity for the window, one when it set none. It multiplies the group
    /// opacity in the target, beside the placement's, so a placement write never clobbers it.
    shell_opacity: f32,
    /// The shell's uniform scale about the window's centre, one when it set none.
    shell_scale: f32,
    /// The shell's offset in logical pixels, zero when it set none. It rides on the placement.
    shell_offset: (f64, f64),
    /// Whether the window is on screen at all (mapped, on the active workspace).
    visible: bool,
    /// Whether the window's surface is secure, so a capture masks it. The shell sets it by writing `secure` on the
    /// window's node.
    secure: bool,
    /// The client's content, when it has committed one.
    content: Option<Content>,
    /// The pixels the content is read from, when the client has attached a buffer.
    buffer: Option<SurfaceSource>,
    /// The window's last picture, once it is closing: drawn in place of its buffer until its
    /// fade ends, when the window is removed.
    closing: Option<Closing>,
    /// The client's title and app id, stored on the window's node for the tree to read.
    title: String,
    app_id: String,
    /// The content size the node's bounds were last set from. The bounds follow the content only
    /// when it changes, so a width the shell wrote survives the client's commits.
    applied_size: Option<Size>,
    /// The position and size last published to the node, as `[x, y, width, height]`.
    published_geometry: Option<[f64; 4]>,
    /// The window state last published to the node: focused, fullscreen, maximised, minimised.
    published_state: Option<[bool; 4]>,
    /// Whether `unresponsive` was last published to the node as true. A node starts false.
    published_unresponsive: bool,
    /// The window this one is a dialog of (`xdg_toplevel.set_parent`, xdg-foreign), by identity: the parent may map
    /// after the dialog, so the node it names is looked up when the relation is published.
    parent: Option<WindowId>,
    /// The parent's node last published to this node's `parent_window`. `None` is a window with no parent.
    published_parent: Option<NodeId>,
    /// Whether the window opened on a token a launcher minted, as the shell reads it in `from_launcher`.
    from_launcher: bool,
    /// Whether `from_launcher` was last published to the node as true. A node starts false.
    published_from_launcher: bool,
    /// The rectangle the window's node takes hits over beyond its own bounds, in surface-local pixels: the surface
    /// tree's, when a subsurface reaches past the root surface.
    hit_extent: Option<Rect>,
    /// The surfaces of the window's tree below its toplevel surface, in tree order. See [`TreeSurface`].
    tree: Vec<TreeSurface>,
    /// The scene node of each surface in `tree` that is drawn, by the surface's key.
    tree_nodes: BTreeMap<u64, TreeNode>,
    /// The key of each node in `tree_nodes`.
    tree_keys: BTreeMap<NodeId, u64>,
}

/// The window management policy the main seat publishes for the stock shell to read.
pub struct MainSeatPolicy {
    pub edge_tiling: bool,
    pub drag_constraint: &'static str,
    pub focus_mode: &'static str,
    pub focus_new_windows: &'static str,
    pub raise_on_click: bool,
    pub auto_raise: bool,
    pub auto_raise_delay: i64,
    pub focus_change_on_pointer_rest: bool,
    pub double_click: &'static str,
    pub middle_click: &'static str,
    pub right_click: &'static str,
    /// The declared workspaces, one `id:name` item each. Empty when the configuration declares none.
    pub workspaces: Vec<String>,
}

/// What an output is, beyond its rectangle: the values the compositor publishes on the output's screen node.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenInfo {
    pub make: String,
    pub model: String,
    /// The mode being driven, as `WxH@Hz`.
    pub mode: String,
    pub scale: f64,
    /// Whether the layout's primary logical monitor is on this output.
    pub primary: bool,
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
    /// The ids of the nodes this composer creates, all in the compositor's range.
    node_ids: NodeIdAllocator,
    /// The damage of the iteration being drawn, taken once by
    /// [`Composer::begin_frame`] and consumed output by output.
    damage: Option<Damage>,
    /// Animations of window properties, driven by [`Composer::advance_motion`].
    motion: TweenPlayer,
    /// The live track for each window property, so a retarget can supersede it.
    tracks: BTreeMap<(WindowId, VisualProperty), TrackId>,
    /// The live track of each property the shell animated with a timed `set`, so a later write to
    /// the same property ends or supersedes it.
    shell_tracks: BTreeMap<(NodeId, VisualProperty), TrackId>,
    /// When set, a change lands at once instead of being animated.
    reduced_motion: bool,
    /// Where the cursor was last drawn, in global layout coordinates, or `None` when hidden.
    cursor: Option<Rect>,
    /// The visual bell's flash in progress, if any. Only one runs at a time: a bell rung
    /// during a flash restarts it.
    flash: Option<Flash>,
    /// The tree changes made since the frame last took them: a window's node created or
    /// destroyed, published in the next batch.
    tree_changes: Vec<TreeChange>,
    /// The `skipped` list last published for each node that had one, so only a change is sent.
    skipped_published: BTreeMap<NodeId, Vec<String>>,
    /// Window nodes created since the last take, announced to the shell as ready.
    tree_ready: Vec<NodeId>,
    /// Lock nodes whose session lock went `pending` since the last take, announced to the shell
    /// as `requested`.
    lock_requested: Vec<NodeId>,
    /// The compositor's seat node for the main wl_seat, once added. It holds the pointer and focus.
    main_seat: Option<NodeId>,
    /// The compositor's lock node, once added. Its `state` is the session lock's state, so a
    /// shell that attaches while the session is locked reads it in the snapshot.
    lock_node: Option<NodeId>,
    /// Whether the canvases under the lock node changed since [`Composer::take_lock_ui_changed`].
    lock_ui_changed: bool,
    /// The focus node the compositor last wrote to the main seat. A `focus` the scene holds
    /// that differs from this was written by the shell.
    published_focus: Option<NodeId>,
    /// The node the shell holds the main seat's focus on, when that node is not a window. The
    /// compositor's "no window has focus" does not take it back, so the shell's keys still reach it.
    shell_focus: Option<NodeId>,
    /// The screen node of each output, created with the output and removed with it.
    screens: BTreeMap<OutputId, NodeId>,
    /// Whether the shell held the tree at the last placement check. While it does, the composer
    /// leaves the node's x and y to the shell.
    placing: bool,
    /// The pictures of the View nodes, each rendered from its source subtree and kept until
    /// that subtree, its camera or the view itself changes.
    views: BTreeMap<NodeId, ViewPicture>,
    /// Whether the active renderer draws a View's subtree into a texture on the GPU. When it does, a View is drawn
    /// as a [`ViewScene`] the renderer draws; when it does not, the composer draws the picture on the CPU. The
    /// compositor sets it once from the backend before any frame.
    gpu_views: bool,
    /// The count of View draws, which numbers each [`ViewScene`] so a renderer that holds the texture of a revision
    /// draws nothing for it.
    view_revision: u64,
    /// The milliseconds the loop has advanced the composer by ([`Composer::advance_motion`]), which is the clock a
    /// View's `fps` is paced on. A test moves it the way the loop does.
    view_clock_ms: u64,
    /// The Views whose source changed inside their `fps` slot: each is drawn at the slot's end, from the state it is
    /// in then, so any number of changes in a slot cost one draw.
    paced_views: BTreeSet<NodeId>,
    /// What a View's capture picture draws in place of a secure node, the same choice an output capture makes for a
    /// secure layer. It is fixed for the session by the `capture.secure-treatment` setting.
    secure_treatment: SecureTreatment,
    /// The nodes that are secure in the frame being drawn: see [`Composer::find_secure`]. Taken once per frame by
    /// [`Composer::begin_frame_reading`], which damages the nodes whose answer changed, and read by every path that
    /// shows or captures a node: the layers a frame marks secure and the redaction of each View's capture.
    secure_scope: BTreeSet<NodeId>,
    /// The effect kinds ([`Pass::name`]) the active renderer draws. A node's effect of any other kind is named in its
    /// `skipped` list. The compositor sets it once from the backend before any frame; until then every kind is
    /// taken as drawn.
    effect_kinds: &'static [&'static str],
    /// The pictures of the Canvas nodes, each painted from its `content` and kept until that or the node's size
    /// changes.
    canvases: BTreeMap<NodeId, CanvasPicture>,
    /// The scale each output composes at, which a canvas on it is painted at so its text and strokes are sharp.
    output_scales: BTreeMap<OutputId, Scale>,
    /// Paints the canvases. The tree answers `measure` from the same painter, so a measured string is drawn in the
    /// extent it was measured at.
    painter: SharedPainter,
}

impl Default for Composer {
    fn default() -> Self {
        Self::new()
    }
}

impl Composer {
    /// An empty scene with an opaque black background, painting canvases with the embedded font alone.
    #[must_use]
    pub fn new() -> Self {
        Self::with_painter(SharedPainter::embedded())
    }

    /// An empty scene that paints `Canvas` nodes with `painter`.
    #[must_use]
    pub fn with_painter(painter: SharedPainter) -> Self {
        let mut node_ids = NodeIdAllocator::new();
        let mut scene = Scene::new();
        // The shell's root takes the allocator's first id, so it is the same for the session.
        let root = node_ids.allocate().expect("a fresh allocator has an id for the root");
        scene
            .insert(root, NodeSpec::new(Rect::new(0, 0, 100, 100)))
            .expect("the root is a fresh node");
        scene.set_root(Some(root)).expect("the root was just inserted");
        // The root is the generic `node` of the tree schema: the shell's stock classes resolve it by name.
        scene
            .set_class(root, Some("node".to_owned()))
            .expect("the root was just inserted");
        Self {
            scene,
            windows: BTreeMap::new(),
            order: Vec::new(),
            background: Color::BLACK,
            node_ids,
            damage: None,
            motion: TweenPlayer::new(),
            tracks: BTreeMap::new(),
            shell_tracks: BTreeMap::new(),
            reduced_motion: false,
            cursor: None,
            flash: None,
            tree_changes: Vec::new(),
            skipped_published: BTreeMap::new(),
            tree_ready: Vec::new(),
            lock_requested: Vec::new(),
            main_seat: None,
            lock_node: None,
            lock_ui_changed: false,
            published_focus: None,
            shell_focus: None,
            screens: BTreeMap::new(),
            placing: false,
            views: BTreeMap::new(),
            gpu_views: false,
            view_revision: 0,
            view_clock_ms: 0,
            paced_views: BTreeSet::new(),
            secure_treatment: SecureTreatment::default(),
            secure_scope: BTreeSet::new(),
            effect_kinds: Pass::NAMES,
            canvases: BTreeMap::new(),
            output_scales: BTreeMap::new(),
            painter,
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
        self.sync_screen(output, rect);
    }

    /// Declares the scale an output composes at. A canvas on it is painted at this scale on the next frame.
    pub fn set_output_scale(&mut self, output: OutputId, scale: Scale) {
        if self.output_scales.insert(output, scale) != Some(scale) {
            self.scene.damage_output(output);
        }
    }

    /// Removes an output, its damage and its screen node.
    pub fn remove_output(&mut self, output: OutputId) {
        self.scene.remove_output(output);
        self.output_scales.remove(&output);
        if let Some(node) = self.screens.remove(&output) {
            self.drop_node(node);
        }
        self.publish_seat_screen();
    }

    /// The screen node of an output, once the composer has declared it.
    #[must_use]
    pub fn screen_node(&self, output: OutputId) -> Option<NodeId> {
        self.screens.get(&output).copied()
    }

    /// Creates the output's screen node under the root on first sight, and publishes its
    /// geometry on every change. The name is the output's id, `output-N`.
    fn sync_screen(&mut self, output: OutputId, rect: Rect) {
        let geometry = [
            ("x", f64::from(rect.x)),
            ("y", f64::from(rect.y)),
            ("width", f64::from(rect.width)),
            ("height", f64::from(rect.height)),
        ];
        let node = match self.screens.get(&output).copied() {
            Some(node) => node,
            None => {
                let Some(id) = self.node_ids.allocate() else {
                    return;
                };
                let Some(root) = self.scene.root_ids().first().copied() else {
                    return;
                };
                let mut spec = NodeSpec::new(rect);
                spec.parent = Some(root);
                spec.input = false;
                if self.scene.insert(id, spec).is_err() {
                    return;
                }
                let _ = self.scene.set_class(id, Some("screen".to_owned()));
                let _ = self.scene.set_text(id, "name", format!("output-{}", output.get()));
                let index = self.scene.root_ids().iter().position(|root| *root == id).unwrap_or(0);
                self.tree_changes.push(TreeChange::Created {
                    node: id,
                    class: "screen".to_owned(),
                    index,
                });
                self.screens.insert(output, id);
                id
            }
        };
        for (key, value) in geometry {
            if self.scene.number(node, key) == Some(value) {
                continue;
            }
            let _ = self.scene.set_number(node, key, value);
            self.tree_changes.push(TreeChange::Set {
                node,
                key: key.to_owned(),
                value: TreeValue::Double(value),
            });
        }
        self.publish_seat_screen();
    }

    /// Publishes what an output is beyond its rectangle to its screen node: the maker and model, the mode as
    /// `WxH@Hz`, the scale, the transform and whether it is the primary output. Each value that differs from what
    /// the node holds is stored and announced as a `set`; an output with no screen node yet publishes nothing.
    pub fn set_output_info(&mut self, output: OutputId, info: &ScreenInfo) {
        let Some(node) = self.screens.get(&output).copied() else {
            return;
        };
        let values = [
            ("make", TreeValue::Text(info.make.clone())),
            ("model", TreeValue::Text(info.model.clone())),
            ("mode", TreeValue::Text(info.mode.clone())),
            ("output_scale", TreeValue::Double(info.scale)),
            ("transform", TreeValue::Text("normal".to_owned())),
            ("primary", TreeValue::Bool(info.primary)),
            ("enabled", TreeValue::Bool(true)),
        ];
        for (key, value) in values {
            if self.scene.value(node, key) == Some(&value) {
                continue;
            }
            let _ = self.scene.set_value(node, key, value.clone());
            self.tree_changes.push(TreeChange::Set {
                node,
                key: key.to_owned(),
                value,
            });
        }
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
    ///
    /// A window that shows a GPU buffer has no pixels a View can read, so a View of one is masked; see
    /// [`Composer::begin_frame_reading`].
    pub fn begin_frame(&mut self) {
        self.begin_frame_reading(&mut |_| None);
    }

    /// [`Composer::begin_frame`] for a session whose backend can read a client's GPU buffer: a View that is redrawn
    /// this frame shows the pixels `reader` returns for each window that shows one. The read happens only for a View
    /// that is stale, and a buffer `reader` answers `None` for is masked.
    pub fn begin_frame_reading(&mut self, reader: &mut ReadDmabuf<'_>) {
        let dirty = self.scene.take_dirty();
        self.take_secure_scope();
        self.render_canvases();
        // A change under the lock node dirties the node itself, which is how the lock cover knows to be drawn again.
        if self.lock_node.is_some_and(|lock| dirty.contains(&lock)) {
            self.lock_ui_changed = true;
        }
        self.render_views(&dirty, reader);
        self.damage = Some(self.scene.take_damage());
    }

    /// Whether the lock node, or a canvas under it, changed since the last call. The lock cover redraws on it.
    pub fn take_lock_ui_changed(&mut self) -> bool {
        std::mem::take(&mut self.lock_ui_changed)
    }

    /// Paints each Canvas node whose picture is stale: it has none yet, its `content` was written since, or its size
    /// changed. The scene already damaged the node's rectangle when the content or the size changed, so a repaint
    /// adds no damage of its own.
    fn render_canvases(&mut self) {
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
            self.canvases.insert(
                canvas,
                CanvasPicture {
                    revision,
                    size,
                    scale,
                    image,
                },
            );
        }
    }

    /// The scale a canvas is painted at: the largest among the outputs it is on, so it is sharp on each and only
    /// shrunk on a lower-density neighbour. A canvas on no output is painted at one.
    fn canvas_scale(&self, canvas: NodeId) -> Scale {
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
    fn paint_canvas(&self, canvas: NodeId, size: Size, scale: Scale) -> Option<Arc<Image>> {
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
    fn canvas_nodes(&self) -> BTreeSet<NodeId> {
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
    fn render_views(&mut self, dirty: &BTreeSet<NodeId>, reader: &mut ReadDmabuf<'_>) {
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

    /// The nodes among `nodes` a capture may not show: those the frame's secure scope holds.
    fn secure_nodes(&self, nodes: impl IntoIterator<Item = NodeId>) -> BTreeSet<NodeId> {
        nodes
            .into_iter()
            .filter(|node| self.secure_scope.contains(node))
            .collect()
    }

    /// Every node that is secure: a window the shell or a rule marked, a node whose own `secure` value is true, and
    /// everything below either. A secure subtree is secure as a whole, wherever a reader stands in it: a View of a
    /// node that lies below a secure one reads secure content though the View's own source holds no mark, and a View
    /// that is itself below a secure node is a secure layer.
    ///
    /// This is the one answer to "is this node secure". The layers a frame marks (windows and their surfaces,
    /// canvases, Views) and the redaction of a View's capture, on the CPU and on the GPU, all take it from the scope
    /// [`Composer::take_secure_scope`] keeps, so no path decides it for itself.
    fn find_secure(&self) -> BTreeSet<NodeId> {
        let marked_windows = self
            .windows
            .values()
            .filter(|entry| entry.secure)
            .filter_map(|entry| entry.node);
        let marked_nodes = self
            .scene
            .node_ids()
            .filter(|node| matches!(self.scene.value(*node, "secure"), Some(TreeValue::Bool(true))));
        let mut scope = BTreeSet::new();
        for root in marked_windows.chain(marked_nodes) {
            // An ancestor that is marked has taken the subtree already.
            if !scope.contains(&root) {
                scope.extend(self.subtree(root));
            }
        }
        scope
    }

    /// Finds the frame's secure scope and damages the nodes whose answer changed since the last one, so the frame a
    /// capture reads is drawn again with the new mark: a mark changes what a capture shows and not the pixels on
    /// screen, so nothing else would redraw it.
    fn take_secure_scope(&mut self) {
        let scope = self.find_secure();
        let changed: Vec<NodeId> = scope.symmetric_difference(&self.secure_scope).copied().collect();
        for node in changed {
            if let Some(placed) = self.scene.absolute_bounds(node) {
                self.scene.damage_rect(placed);
            }
        }
        self.secure_scope = scope;
    }

    /// Brings the secure scope up to date, as [`Composer::take_secure_scope`] does, and answers the nodes it newly
    /// holds, so the frame's tree work can end a capture of one of them before the frame's replies go out.
    pub fn take_newly_secure(&mut self) -> BTreeSet<NodeId> {
        let newly = self.find_secure().difference(&self.secure_scope).copied().collect();
        self.take_secure_scope();
        newly
    }

    /// Whether `root`, a node below it, or the source a View at `root` draws, is among `nodes`.
    pub fn reaches(&self, root: NodeId, nodes: &BTreeSet<NodeId>) -> bool {
        std::iter::once(root)
            .chain(self.scene.source(root))
            .any(|node| self.subtree(node).iter().any(|below| nodes.contains(below)))
    }

    /// Whether `output` shows one of `nodes`: a node below its screen, or one whose bounds overlap the output's
    /// rectangle. A window is a root node, not a child of the screen, so the bounds are what places it on an output.
    pub fn reaches_output(&self, output: OutputId, nodes: &BTreeSet<NodeId>) -> bool {
        let Some(screen) = self.screen_node(output) else {
            return false;
        };
        if self.reaches(screen, nodes) {
            return true;
        }
        let Some(area) = self.scene.absolute_bounds(screen) else {
            return false;
        };
        nodes.iter().any(|node| {
            self.scene
                .absolute_bounds(*node)
                .is_some_and(|bounds| bounds.intersects(area))
        })
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
    fn draw_view(
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
    fn view_scene(
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
    fn emit_view_node(&self, emit: &ViewEmit<'_>, id: NodeId, parent_opacity: f64, ops: &mut Vec<ViewOp>) {
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

    /// The pixels each node of `nodes` draws from, with their size: a window's surface or a canvas. A window's buffer
    /// is shared, not read, so a GPU renderer samples the client's own memory and a GPU buffer needs no readback.
    fn view_sources(&self, nodes: &BTreeSet<NodeId>) -> BTreeMap<NodeId, ViewPixels> {
        let mut pictures = BTreeMap::new();
        for entry in self.windows.values() {
            let own = entry.node.zip(entry.buffer.as_ref());
            let below = entry
                .tree
                .iter()
                .filter_map(|surface| Some((entry.tree_nodes.get(&surface.key)?.node, &surface.source)));
            for (node, source) in own.into_iter().chain(below) {
                if let (true, Some(size)) = (nodes.contains(&node), source_size(source)) {
                    pictures.insert(
                        node,
                        ViewPixels {
                            source: source.clone(),
                            size,
                        },
                    );
                }
            }
        }
        for (node, picture) in &self.canvases {
            if let (true, Some(image)) = (nodes.contains(node), picture.image.as_ref()) {
                // A canvas picture is premultiplied, which is what a snapshot is.
                pictures.insert(
                    *node,
                    ViewPixels {
                        source: SurfaceSource::Snapshot(Arc::clone(image)),
                        size: image.size(),
                    },
                );
            }
        }
        pictures
    }

    /// The pixels of each window in `nodes` with a readable buffer, keyed by its node. A GPU buffer (a DMA-BUF) is read
    /// by `reader`, and when it cannot be read its window is drawn as the neutral grey a secure node is masked with,
    /// and the first one is logged: see [`source_picture`].
    fn window_pictures(&self, nodes: &BTreeSet<NodeId>, reader: &mut ReadDmabuf<'_>) -> BTreeMap<NodeId, Image> {
        let mut pictures: BTreeMap<NodeId, Image> = BTreeMap::new();
        for entry in self.windows.values() {
            let own = entry.node.zip(entry.buffer.as_ref());
            let below = entry
                .tree
                .iter()
                .filter_map(|surface| Some((entry.tree_nodes.get(&surface.key)?.node, &surface.source)));
            for (node, source) in own.into_iter().chain(below) {
                if let Some(image) = nodes.contains(&node).then(|| source_picture(source, reader)).flatten() {
                    pictures.insert(node, image);
                }
            }
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
    fn view_rig(&self, view: NodeId, size: Size) -> ViewRig {
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
    fn view_nodes(&self) -> BTreeSet<NodeId> {
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

    /// `root` and every node below it.
    fn subtree(&self, root: NodeId) -> Vec<NodeId> {
        let mut nodes = Vec::new();
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            nodes.push(node);
            stack.extend(self.scene.children(node).unwrap_or(&[]));
        }
        nodes
    }

    /// Runs `f` against the scene the composer draws from. The tree's frame work edits it
    /// here, before [`Composer::begin_frame`] takes the damage, so the edits draw this frame.
    pub fn with_scene<R>(&mut self, f: impl FnOnce(&mut Scene) -> R) -> R {
        f(&mut self.scene)
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
                surface_offset: (0, 0),
                transform: VisualTransform::IDENTITY,
                visible: true,
                secure: false,
                content: None,
                surface_alpha: 1.0,
                rule_opacity: 1.0,
                shell_opacity: 1.0,
                shell_scale: 1.0,
                shell_offset: (0.0, 0.0),
                buffer: None,
                closing: None,
                title: String::new(),
                app_id: String::new(),
                published_state: None,
                published_unresponsive: false,
                parent: None,
                published_parent: None,
                from_launcher: false,
                published_from_launcher: false,
                applied_size: None,
                published_geometry: None,
                hit_extent: None,
                tree: Vec::new(),
                tree_nodes: BTreeMap::new(),
                tree_keys: BTreeMap::new(),
            },
        );
        self.order.push(window);
        self.sync(window, false);
    }

    /// Sets the rectangle a window takes hits over beyond its own surface, in surface-local pixels, or clears it.
    /// It changes what the pointer and touch reach and draws nothing.
    pub fn set_window_hit_extent(&mut self, window: WindowId, extent: Option<Rect>) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        entry.hit_extent = extent;
        if let Some(node) = entry.node {
            let _ = self.scene.set_hit_extent(node, extent);
        }
    }

    /// Stops drawing a window, damaging what it held.
    pub fn remove_window(&mut self, window: WindowId) {
        let Some(entry) = self.windows.remove(&window) else {
            return;
        };
        self.cancel_tracks(window);
        if let Some(node) = entry.node {
            self.drop_node(node);
        }
        self.order.retain(|candidate| *candidate != window);
        // A dialog whose parent is gone has no parent: the shell is told, as it is of any other change.
        self.publish_dialogs_of(window);
    }

    /// Removes a window's node from the scene, recording its destruction for the tree.
    fn drop_node(&mut self, node: NodeId) {
        if let Ok(removed) = self.scene.remove(node) {
            self.tree_changes.push(TreeChange::Destroyed { node, removed });
        }
    }

    /// Publishes a window node's position and size to the tree, when they changed since the last
    /// publish. The size is the content size; the position is the placed frame's origin.
    ///
    /// While the shell holds the tree its placement stands, so the position is not published and
    /// the tree is treated as not knowing it: the next publish after the hold writes it.
    fn publish_geometry(&mut self, window: WindowId) {
        let holds = self.scene.shell_places();
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        let (Some(node), Some(content)) = (entry.node, entry.content) else {
            return;
        };
        let values = [
            f64::from(entry.geometry.x - entry.surface_offset.0),
            f64::from(entry.geometry.y - entry.surface_offset.1),
            f64::from(content.size.width),
            f64::from(content.size.height),
        ];
        let mut published = entry.published_geometry.unwrap_or([f64::NAN; 4]);
        if holds {
            published[0] = f64::NAN;
            published[1] = f64::NAN;
        }
        let mut next = published;
        for (index, (key, value)) in ["x", "y", "width", "height"].into_iter().zip(values).enumerate() {
            if holds && index < 2 {
                continue;
            }
            if published[index] == value {
                continue;
            }
            next[index] = value;
            self.tree_changes.push(TreeChange::Set {
                node,
                key: key.to_owned(),
                value: TreeValue::Double(value),
            });
        }
        if let Some(entry) = self.windows.get_mut(&window) {
            entry.published_geometry = Some(next);
        }
    }

    /// Publishes a window's focus and window-manager state (`focused`, `fullscreen`, `maximised`,
    /// `minimised`) to its node as a `set` when any of them changes. While a shell holds the tree
    /// only `focused` is published: the other three are the shell's to write, and sending its own
    /// values back would only make its next transaction stale.
    pub fn set_window_state(&mut self, window: WindowId, state: [bool; 4]) {
        let holds = self.scene.shell_places();
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        if entry.published_state == Some(state) {
            return;
        }
        entry.published_state = Some(state);
        let Some(node) = entry.node else {
            return;
        };
        for (index, (key, value)) in ["focused", "fullscreen", "maximised", "minimised"]
            .into_iter()
            .zip(state)
            .enumerate()
        {
            if holds && index > 0 {
                continue;
            }
            self.tree_changes.push(TreeChange::Set {
                node,
                key: key.to_owned(),
                value: TreeValue::Bool(value),
            });
        }
    }

    /// Publishes whether the window's client has stopped answering its liveness pings, as the node's `unresponsive`,
    /// when that changed. A window with no node yet is published once it has one: nothing is recorded until then, and
    /// a window that gets a node while marked reports it on the next call.
    pub fn set_window_unresponsive(&mut self, window: WindowId, unresponsive: bool) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        let Some(node) = entry.node else {
            return;
        };
        if entry.published_unresponsive == unresponsive {
            return;
        }
        entry.published_unresponsive = unresponsive;
        self.tree_changes.push(TreeChange::Set {
            node,
            key: "unresponsive".to_owned(),
            value: TreeValue::Bool(unresponsive),
        });
    }

    /// Records the window `window` is a dialog of (`xdg_toplevel.set_parent` or xdg-foreign), or that it has none, and
    /// publishes it as the node's `parent_window`. A parent that has no node yet is published when it gets one.
    pub fn set_window_parent(&mut self, window: WindowId, parent: Option<WindowId>) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        if entry.parent == parent {
            return;
        }
        entry.parent = parent;
        self.publish_relations(window);
    }

    /// Marks the window as opened on a token a launcher minted, and publishes it as the node's `from_launcher`. The
    /// mark stays for the life of the window.
    pub fn set_window_from_launcher(&mut self, window: WindowId) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        if entry.from_launcher {
            return;
        }
        entry.from_launcher = true;
        self.publish_relations(window);
    }

    /// Publishes the node references and marks a window's record holds: `parent_window` as the parent's node (none
    /// while the parent has no node) and `from_launcher`. Only a change is published, and a window with no node waits.
    fn publish_relations(&mut self, window: WindowId) {
        let parent_node = self
            .windows
            .get(&window)
            .and_then(|entry| entry.parent)
            .and_then(|parent| self.windows.get(&parent))
            .and_then(|parent| parent.node);
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        let Some(node) = entry.node else {
            return;
        };
        if entry.published_parent != parent_node {
            entry.published_parent = parent_node;
            let _ = self.scene.set_reference(node, "parent_window", parent_node);
            self.tree_changes.push(TreeChange::Set {
                node,
                key: "parent_window".to_owned(),
                value: TreeValue::Node(parent_node),
            });
        }
        if entry.from_launcher && !entry.published_from_launcher {
            entry.published_from_launcher = true;
            let _ = self.scene.set_value(node, "from_launcher", TreeValue::Bool(true));
            self.tree_changes.push(TreeChange::Set {
                node,
                key: "from_launcher".to_owned(),
                value: TreeValue::Bool(true),
            });
        }
    }

    /// Publishes the relation of every window that is a dialog of `parent`, now that its node exists or is gone.
    fn publish_dialogs_of(&mut self, parent: WindowId) {
        let dialogs: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(_, entry)| entry.parent == Some(parent))
            .map(|(window, _)| *window)
            .collect();
        for dialog in dialogs {
            self.publish_relations(dialog);
        }
    }

    /// The `[fullscreen, maximised, minimised]` flags the shell holds on a window's node, or
    /// `None` while no shell holds the tree or the window has no node. While the shell holds the
    /// tree these three are the shell's to write, so they are read from the node and never
    /// published over it.
    #[must_use]
    pub fn shell_window_flags(&self, window: WindowId) -> Option<[bool; 3]> {
        if !self.scene.shell_places() {
            return None;
        }
        let node = self.windows.get(&window)?.node?;
        let flag = |key: &str| matches!(self.scene.value(node, key), Some(TreeValue::Bool(true)));
        Some([flag("fullscreen"), flag("maximised"), flag("minimised")])
    }

    /// Whether the shell marked the window's node `tiled`. Only the shell writes it, so a session with no shell
    /// holds every window untiled.
    #[must_use]
    pub fn shell_window_tiled(&self, window: WindowId) -> bool {
        if !self.scene.shell_places() {
            return false;
        }
        let Some(node) = self.windows.get(&window).and_then(|entry| entry.node) else {
            return false;
        };
        matches!(self.scene.value(node, "tiled"), Some(TreeValue::Bool(true)))
    }

    /// The size of a window's node: the content size the client last committed, or the size the
    /// shell wrote since. `None` when the window has no node.
    #[must_use]
    pub fn window_node_size(&self, window: WindowId) -> Option<(u32, u32)> {
        let node = self.windows.get(&window)?.node?;
        let bounds = self.scene.bounds(node)?;
        Some((bounds.width, bounds.height))
    }

    /// Stores a window's title and app id for its node's read-only texts. Each one that changes
    /// while the window has a node is published to the tree as a `set`.
    pub fn set_window_text(&mut self, window: WindowId, title: &str, app_id: &str) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        let mut changed = Vec::new();
        if entry.title != title {
            entry.title = title.to_owned();
            changed.push(("title", entry.title.clone()));
        }
        if entry.app_id != app_id {
            entry.app_id = app_id.to_owned();
            changed.push(("app_id", entry.app_id.clone()));
        }
        let Some(node) = entry.node else {
            return;
        };
        for (key, value) in changed {
            let _ = self.scene.set_text(node, key, value.clone());
            self.tree_changes.push(TreeChange::Set {
                node,
                key: key.to_owned(),
                value: TreeValue::Text(value),
            });
        }
    }

    /// Takes the tree changes made since the last take, in the order they happened.
    pub fn take_tree_changes(&mut self) -> Vec<TreeChange> {
        std::mem::take(&mut self.tree_changes)
    }

    /// Takes the window nodes created since the last take. Each is announced to the shell once,
    /// when its node first exists with content.
    pub fn take_tree_ready(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.tree_ready)
    }

    /// Takes the lock nodes whose session lock went `pending` since the last take. Each is
    /// announced to the shell once per transition to `pending`.
    pub fn take_lock_requested(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.lock_requested)
    }

    /// Follows the shell's hold on the tree. When the hold changed since the last check, every
    /// window is resynced: placed by the composer again, or left to the shell.
    pub fn follow_placement(&mut self) {
        let holds = self.scene.shell_places();
        if holds == self.placing {
            return;
        }
        self.placing = holds;
        let order = self.order.clone();
        for window in order {
            self.sync_animated(window);
        }
    }

    /// The transform a window's node should reach. While the shell holds the tree, the node's
    /// x and y are the shell's, so the target keeps them where they are.
    fn placed_target(&self, entry: &WindowNode) -> VisualTransform {
        let mut transform = Self::target(entry);
        if self.scene.shell_places() {
            let (x, y) = entry
                .node
                .and_then(|node| self.scene.transform(node))
                .map_or((0.0, 0.0), |current| (current.base.offset.x, current.base.offset.y));
            transform.base.offset.x = x;
            transform.base.offset.y = y;
        }
        transform
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
        let source = entry.buffer.as_ref()?;
        let content = entry.content?;
        let source = match source {
            SurfaceSource::Buffer(buffer) => SurfaceSource::Snapshot(Arc::new(shm::readback(buffer).ok()?)),
            SurfaceSource::Snapshot(image) => SurfaceSource::Snapshot(Arc::clone(image)),
            // A GPU buffer has no CPU picture to copy, and needs none: the handle owns the plane descriptors, so it
            // outlives the client's `wl_buffer`, and the renderer that imported it draws it again for the fade.
            SurfaceSource::Dmabuf(dmabuf) => SurfaceSource::Dmabuf(dmabuf.clone()),
            // A window is never drawn from a View's texture.
            SurfaceSource::View(_) => return None,
        };
        Some(Closing {
            source,
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
        if on_end == CloseEnd::Remove {
            // The client is destroying the window, so the buffers of its popups and subsurfaces may go at any
            // moment: they leave with it rather than fading from pixels that may no longer be there.
            self.drop_tree_nodes(window);
        }
        if let Some(entry) = self.windows.get_mut(&window) {
            entry.closing = Some(closing);
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

    /// Sets where a window's geometry starts inside its surface, as the client's `xdg_surface.set_window_geometry`
    /// says: the surface is drawn that far up and left of the frame, so the frame's origin is the geometry's.
    pub fn set_window_surface_offset(&mut self, window: WindowId, x: i32, y: i32) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        if entry.surface_offset == (x, y) {
            return;
        }
        entry.surface_offset = (x, y);
        self.sync(window, false);
    }

    /// Shows or hides a window without forgetting where it is.
    ///
    /// Hiding fades the window's last picture out, as an unmap does, so a minimise is seen rather
    /// than cut. Showing it again while that fade runs eases it back to opaque on the same node.
    /// The window's content is kept either way, so the next show draws what the client last
    /// committed.
    pub fn set_window_visible(&mut self, window: WindowId, visible: bool) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        if entry.visible == visible {
            return;
        }
        if visible {
            entry.visible = true;
            if !self.cancel_hide_fade(window) {
                self.sync(window, false);
            }
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

    /// Applies the `secure` value the shell wrote on each window's node to the window's mark, so a window the shell
    /// marks is masked in a capture and one it clears is not. A window whose node holds no `secure` value keeps
    /// its mark. Called each frame after the tree's transactions apply.
    pub fn follow_shell_secure(&mut self) {
        let written: Vec<(WindowId, bool)> = self
            .windows
            .iter()
            .filter_map(|(window, entry)| match self.scene.value(entry.node?, "secure")? {
                TreeValue::Bool(secure) => Some((*window, *secure)),
                _ => None,
            })
            .collect();
        for (window, secure) in written {
            self.set_window_secure(window, secure);
        }
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

    /// Sets a window's `wp_alpha_modifier_v1` factor, which multiplies its node's opacity.
    ///
    /// A factor outside zero to one, or not finite, is clamped the same way as the group
    /// opacity, so a bad value cannot poison the blend. A closing window keeps the factor its
    /// fade began with, as its placement does.
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

    /// Sets a window's opacity from a matched window rule. It folds into the node's opacity
    /// beside the placement opacity and the client's surface factor.
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

    /// Sets the shell's opacity for a window, which multiplies its group opacity. A value
    /// outside 0 to 1, or not finite, is refused and the window keeps its current opacity.
    pub fn set_shell_opacity(&mut self, window: WindowId, opacity: f32) -> Result<(), ShellVisualError> {
        if !(0.0..=1.0).contains(&opacity) {
            return Err(ShellVisualError::OutOfRange);
        }
        let Some(entry) = self.windows.get_mut(&window) else {
            return Ok(());
        };
        if entry.shell_opacity != opacity {
            entry.shell_opacity = opacity;
            self.sync(window, true);
        }
        Ok(())
    }

    /// Sets the shell's uniform scale for a window, about its centre. A value outside 0.01 to
    /// 10, or not finite, is refused and the window keeps its current scale.
    pub fn set_shell_scale(&mut self, window: WindowId, scale: f32) -> Result<(), ShellVisualError> {
        if !(SHELL_SCALE_MIN..=SHELL_SCALE_MAX).contains(&scale) {
            return Err(ShellVisualError::OutOfRange);
        }
        let Some(entry) = self.windows.get_mut(&window) else {
            return Ok(());
        };
        if entry.shell_scale != scale {
            entry.shell_scale = scale;
            self.sync(window, true);
        }
        Ok(())
    }

    /// Sets the shell's offset for a window, in logical pixels. A component outside plus or
    /// minus 100000, or not finite, refuses the whole offset and the window keeps its current one.
    pub fn set_shell_offset(&mut self, window: WindowId, x: f64, y: f64) -> Result<(), ShellVisualError> {
        if !(x.abs() <= SHELL_OFFSET_LIMIT && y.abs() <= SHELL_OFFSET_LIMIT) {
            return Err(ShellVisualError::OutOfRange);
        }
        let Some(entry) = self.windows.get_mut(&window) else {
            return Ok(());
        };
        if entry.shell_offset != (x, y) {
            entry.shell_offset = (x, y);
            self.sync(window, true);
        }
        Ok(())
    }

    /// Puts a window's shell visuals back to their defaults: fully opaque, unscaled, unoffset.
    pub fn reset_shell_visuals(&mut self, window: WindowId) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        let defaults = (1.0, 1.0, (0.0, 0.0));
        if (entry.shell_opacity, entry.shell_scale, entry.shell_offset) == defaults {
            return;
        }
        entry.shell_opacity = 1.0;
        entry.shell_scale = 1.0;
        entry.shell_offset = (0.0, 0.0);
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
        self.view_clock_ms = self.view_clock_ms.saturating_add(u64::from(elapsed_ms));
        self.motion.advance(&mut self.scene, elapsed_ms);
        for end in self.motion.take_ended() {
            let owner = self
                .tracks
                .iter()
                .find(|(_, track)| **track == end.track)
                .map(|(key, _)| *key);
            self.tracks.retain(|_, track| *track != end.track);
            self.shell_tracks.retain(|_, track| *track != end.track);
            if let Some((window, VisualProperty::Opacity)) = owner
                && let Some(on_end) = self
                    .windows
                    .get(&window)
                    .and_then(|entry| entry.closing.as_ref())
                    .map(|closing| closing.on_end)
            {
                self.finish_fade(window, on_end);
            }
        }
        self.advance_flash(elapsed_ms);
    }

    /// Applies what the shell's writes to animated properties asked of the tracks, in the order the
    /// writes applied. A timed `set` plays a track to its target; with reduced motion on, or for a
    /// property the scene cannot tween (a width or height), the target lands at once instead, so a
    /// shell that forgot to shorten its durations still gets instant changes. A write to a
    /// property ends the track running on it either way.
    ///
    /// Reduced motion is read when the write is applied: a track already running when it turns on
    /// finishes, and the next write lands at once.
    pub fn apply_shell_motion(&mut self, motion: Vec<Motion>) {
        for item in motion {
            match item {
                Motion::Settle { node, key } => {
                    if let Some(property) = visual_property(&key) {
                        self.end_shell_track(node, property);
                    }
                }
                Motion::Play(animation) => self.play_shell_animation(&animation),
            }
        }
    }

    /// How many tracks the shell's timed writes have running.
    #[must_use]
    pub fn shell_tracks_running(&self) -> usize {
        self.shell_tracks.len()
    }

    /// Ends the track the shell's timed `set` started on `node`'s `property`, leaving the value where it is.
    fn end_shell_track(&mut self, node: NodeId, property: VisualProperty) {
        if let Some(track) = self.shell_tracks.remove(&(node, property)) {
            let _ = self.motion.cancel(track);
        }
    }

    /// Plays one timed `set`, or lands its target at once when it cannot be played.
    fn play_shell_animation(&mut self, animation: &Animation) {
        let property = visual_property(&animation.key);
        if let Some(property) = property {
            self.end_shell_track(animation.node, property);
        }
        if let Some(property) = property.filter(|_| !self.reduced_motion)
            && self.start_shell_track(animation, property)
        {
            return;
        }
        if let Err(code) = land_at_once(&mut self.scene, animation) {
            log::warn!(
                area = "render",
                node = %animation.node,
                key = %animation.key,
                code = code.as_str(),
                "an animated set could not land: the node keeps its value"
            );
        }
    }

    /// Starts the track for a timed `set`. `false` when the duration rounds to nothing or the scene
    /// refuses the track, so the caller lands the target at once.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn start_shell_track(&mut self, animation: &Animation, property: VisualProperty) -> bool {
        let TreeValue::Double(target) = animation.value else {
            return false;
        };
        let duration_ms = (animation.seconds * 1000.0).round() as u32;
        let Ok(curve) = SampledCurve::new(animation.samples.clone()) else {
            return false;
        };
        if duration_ms == 0 {
            return false;
        }
        let mut spec = TrackSpec::new(animation.node, property, PropertyValue::number(target as f32));
        spec.duration_ms = duration_ms;
        let Ok(track) = self.motion.start_sampled(&mut self.scene, spec, curve) else {
            return false;
        };
        // The node keeps the target as its value, so a snapshot read mid-track agrees with the change the
        // shell's mirror was told.
        let _ = self
            .scene
            .set_value(animation.node, &animation.key, animation.value.clone());
        self.shell_tracks.insert((animation.node, property), track);
        true
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

    /// The rectangle a window's node covers on the output plane, rounded to whole pixels: where a shell that holds
    /// the tree put the window. `None` when the window has no node yet or it is not drawn.
    #[must_use]
    pub fn window_frame(&self, window: WindowId) -> Option<Rect> {
        let node = self.windows.get(&window)?.node?;
        self.scene.content_bounds(node).map(gnoblin_types::RectF::to_pixel)
    }

    /// The window drawn under a point on the output plane, and the point in that window's local
    /// space, which is the surface point the pointer is over. The topmost window wins, and a window
    /// `accept` refuses passes the point through to what lies under it.
    #[must_use]
    pub fn window_at(&self, point: PointF, accept: &dyn Fn(WindowId) -> bool) -> Option<(WindowId, PointF)> {
        let hit = self
            .scene
            .hit_test_where(point, &|node| self.window_of(node).is_some_and(accept))?;
        Some((self.window_of(hit.node)?, hit.local))
    }

    /// The window whose node is `node`.
    fn window_of(&self, node: NodeId) -> Option<WindowId> {
        self.windows
            .iter()
            .find_map(|(window, entry)| (entry.node == Some(node)).then_some(*window))
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

    /// Sets what a View's capture picture draws in place of a secure node. It is the same choice an output capture
    /// makes, and the compositor sets it once, from the `capture.secure-treatment` setting, before any frame.
    pub fn set_secure_treatment(&mut self, treatment: SecureTreatment) {
        if self.secure_treatment == treatment {
            return;
        }
        self.secure_treatment = treatment;
        // A picture drawn under the other treatment would keep the wrong pixels in a secure node's place.
        self.views.clear();
    }

    /// The scene node of a window, once the composer has given it one.
    #[must_use]
    pub fn window_node(&self, window: WindowId) -> Option<NodeId> {
        self.windows.get(&window).and_then(|entry| entry.node)
    }

    /// Adds the compositor's seat node for the main wl_seat, a child of the root. The shell
    /// cannot destroy or move it. Its name is text only, so it is not a second wl_seat global.
    pub fn add_main_seat(&mut self, name: &str) -> Option<NodeId> {
        if let Some(seat) = self.main_seat {
            return Some(seat);
        }
        let id = self.node_ids.allocate()?;
        let root = *self.scene.root_ids().first()?;
        let mut spec = NodeSpec::new(Rect::new(0, 0, 0, 0));
        spec.parent = Some(root);
        spec.input = false;
        self.scene.insert(id, spec).ok()?;
        let _ = self.scene.set_class(id, Some("seat".to_owned()));
        let _ = self.scene.set_text(id, "name", name.to_owned());
        let _ = self.scene.set_number(id, "pointer_x", 0.0);
        let _ = self.scene.set_number(id, "pointer_y", 0.0);
        let index = self.scene.root_ids().iter().position(|root| *root == id).unwrap_or(0);
        self.tree_changes.push(TreeChange::Created {
            node: id,
            class: "seat".to_owned(),
            index,
        });
        self.main_seat = Some(id);
        self.publish_seat_screen();
        self.publish_effect_kinds();
        Some(id)
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

    /// Publishes the screen under the main seat's pointer as the seat's `screen`. When the
    /// pointer is over no screen, or none has been declared yet, the first screen is
    /// published, or none. Only a change is published.
    fn publish_seat_screen(&mut self) {
        let Some(seat) = self.main_seat else {
            return;
        };
        let x = self.scene.number(seat, "pointer_x").unwrap_or(0.0);
        let y = self.scene.number(seat, "pointer_y").unwrap_or(0.0);
        let contains = |node: &NodeId| {
            let (Some(nx), Some(ny), Some(w), Some(h)) = (
                self.scene.number(*node, "x"),
                self.scene.number(*node, "y"),
                self.scene.number(*node, "width"),
                self.scene.number(*node, "height"),
            ) else {
                return false;
            };
            x >= nx && x < nx + w && y >= ny && y < ny + h
        };
        let target = self
            .screens
            .values()
            .copied()
            .find(contains)
            .or_else(|| self.screens.values().next().copied());
        if self.scene.reference(seat, "screen") == target {
            return;
        }
        let _ = self.scene.set_reference(seat, "screen", target);
        self.tree_changes.push(TreeChange::Set {
            node: seat,
            key: "screen".to_owned(),
            value: TreeValue::Node(target),
        });
    }

    /// Publishes the pointer's layout position on the main seat. Only a change is published.
    pub fn set_main_seat_pointer(&mut self, x: f64, y: f64) {
        let Some(seat) = self.main_seat else {
            return;
        };
        for (key, value) in [("pointer_x", x), ("pointer_y", y)] {
            if self.scene.number(seat, key) == Some(value) {
                continue;
            }
            let _ = self.scene.set_number(seat, key, value);
            self.tree_changes.push(TreeChange::Set {
                node: seat,
                key: key.to_owned(),
                value: TreeValue::Double(value),
            });
        }
        self.publish_seat_screen();
    }

    /// Publishes the main seat's window management policy, the values the stock shell reads
    /// from the seat. Each key is published only when its value changes.
    pub fn publish_main_seat_policy(&mut self, policy: &MainSeatPolicy) {
        let Some(seat) = self.main_seat else {
            return;
        };
        let reduced_motion = self.reduced_motion;
        for (key, value) in [
            ("edge_tiling", TreeValue::Bool(policy.edge_tiling)),
            ("reduced_motion", TreeValue::Bool(reduced_motion)),
            ("drag_constraint", TreeValue::Text(policy.drag_constraint.to_owned())),
            ("focus_mode", TreeValue::Text(policy.focus_mode.to_owned())),
            (
                "focus_new_windows",
                TreeValue::Text(policy.focus_new_windows.to_owned()),
            ),
            ("raise_on_click", TreeValue::Bool(policy.raise_on_click)),
            ("auto_raise", TreeValue::Bool(policy.auto_raise)),
            ("auto_raise_delay", TreeValue::Int(policy.auto_raise_delay)),
            (
                "focus_change_on_pointer_rest",
                TreeValue::Bool(policy.focus_change_on_pointer_rest),
            ),
            ("double_click", TreeValue::Text(policy.double_click.to_owned())),
            ("middle_click", TreeValue::Text(policy.middle_click.to_owned())),
            ("right_click", TreeValue::Text(policy.right_click.to_owned())),
            ("workspaces", TreeValue::ListText(policy.workspaces.clone())),
        ] {
            if self.scene.value(seat, key) != Some(&value) {
                let _ = self.scene.set_value(seat, key, value.clone());
                self.tree_changes.push(TreeChange::Set {
                    node: seat,
                    key: key.to_owned(),
                    value,
                });
            }
        }
    }

    /// Publishes the input devices the main seat has, one `id:name:caps` item each in the order they
    /// arrived, on its read-only `device` property. Only a change is published, so a repeated call with the
    /// same list sends the shell nothing.
    pub fn publish_main_seat_devices(&mut self, devices: Vec<String>) {
        let Some(seat) = self.main_seat else {
            return;
        };
        let value = TreeValue::ListText(devices);
        if self.scene.value(seat, "device") != Some(&value) {
            let _ = self.scene.set_value(seat, "device", value.clone());
            self.tree_changes.push(TreeChange::Set {
                node: seat,
                key: "device".to_owned(),
                value,
            });
        }
    }

    /// Publishes the window that holds keyboard focus on the main seat, or none. Only a change
    /// is published. What is published is remembered, so a shell write can be told apart.
    pub fn set_main_seat_focus(&mut self, window: Option<WindowId>) {
        let Some(seat) = self.main_seat else {
            return;
        };
        let target = window.and_then(|window| self.window_node(window));
        if target.is_none() && self.shell_focus.is_some() && self.main_seat_focus() == self.shell_focus {
            // The shell holds the seat focus on one of its nodes; "no window" does not take it back.
            return;
        }
        self.shell_focus = None;
        self.published_focus = target;
        if self.scene.reference(seat, "focus") == target {
            return;
        }
        let _ = self.scene.set_reference(seat, "focus", target);
        self.tree_changes.push(TreeChange::Set {
            node: seat,
            key: "focus".to_owned(),
            value: TreeValue::Node(target),
        });
    }

    /// Records that the shell wrote the main seat's `focus` onto one of its own nodes, so the
    /// compositor's "no window" publishes do not take the focus back. The write is not republished.
    pub fn hold_seat_focus_for_shell(&mut self) {
        self.shell_focus = self.main_seat_focus();
        self.published_focus = None;
    }

    /// Gives the main seat's focus back to the compositor: a window or no window replaces the
    /// shell's node on the next publish.
    pub fn release_shell_focus(&mut self) {
        self.shell_focus = None;
    }

    /// The shell's node that holds the main seat's focus and takes input, the target of the keys
    /// the router does not consume. A node that was destroyed or stopped taking input is not one.
    #[must_use]
    pub fn shell_key_target(&self) -> Option<NodeId> {
        let node = self.shell_focus?;
        (self.main_seat_focus() == Some(node) && self.scene.input(node) == Some(true)).then_some(node)
    }

    /// The shell's node that takes the keys while the compositor's own lock covers the session: the same node
    /// [`Self::shell_key_target`] names, and only while the shell's lock screen is up (the `lock` node's `engaged` is
    /// true). The shell is the compositor's own trusted peer and the lock screen is what it draws, so the typed
    /// password reaches the screen that checks it; no client ever sees the keys.
    #[must_use]
    pub fn shell_lock_key_target(&self) -> Option<NodeId> {
        self.lock_ui_engaged().then(|| self.shell_key_target()).flatten()
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
        let (Some(lock), Some(rect)) = (self.lock_node, self.scene.outputs().get(&output).copied()) else {
            return false;
        };
        if !self.lock_ui_engaged() || !self.scene.shell_places() {
            return false;
        }
        let under_lock = |mut node: NodeId| {
            while let Some(parent) = self.scene.parent(node) {
                if parent == lock {
                    return true;
                }
                node = parent;
            }
            false
        };
        let ranks = self.paint_ranks();
        let mut canvases: Vec<(u32, NodeId)> = self
            .canvases
            .iter()
            .filter(|(node, picture)| picture.image.is_some() && self.scene.drawn(**node) && under_lock(**node))
            .filter_map(|(node, _)| Some((*ranks.get(node)?, *node)))
            .collect();
        canvases.sort_unstable();
        let mut spatial = false;
        for (_, canvas) in &canvases {
            self.push_canvas(*canvas, rect, frame, &mut spatial);
        }
        !canvases.is_empty()
    }

    /// The node the main seat's `focus` names, which the shell may have written.
    #[must_use]
    pub fn main_seat_focus(&self) -> Option<NodeId> {
        self.main_seat.and_then(|seat| self.scene.reference(seat, "focus"))
    }

    /// Whether the shell wrote the main seat's `focus` since the compositor last published it.
    #[must_use]
    pub fn main_seat_focus_written_by_shell(&self) -> bool {
        let focus = self.main_seat_focus();
        focus != self.published_focus && (self.shell_focus.is_none() || focus != self.shell_focus)
    }

    /// The window whose node the main seat's `focus` names.
    #[must_use]
    pub fn main_seat_focus_window(&self) -> Option<WindowId> {
        self.main_seat_focus().and_then(|node| self.window_of_node(node))
    }

    /// The managed window that is the scene node `node`, when one is.
    #[must_use]
    pub fn window_of_node(&self, node: NodeId) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|(_, entry)| entry.node == Some(node))
            .map(|(window, _)| *window)
    }

    /// The shell's node drawn topmost under a point on the output plane, with the point in that
    /// node's local space. A window above it wins: only a topmost node that is not a window is the
    /// shell's, so a shell node under a window is not reported. A node the compositor created since
    /// the last frame and has not yet published in a batch is skipped (F29): the shell has not
    /// heard of it, so it cannot be sent input, and the pick falls through to what lies under it.
    #[must_use]
    pub fn shell_node_at(&self, point: PointF) -> Option<(NodeId, PointF)> {
        let unpublished = |node: NodeId| {
            self.tree_changes
                .iter()
                .any(|change| matches!(change, TreeChange::Created { node: created, .. } if *created == node))
        };
        let hit = self
            .scene
            .hit_test_where(point, &|node| self.window_of(node).is_some() || !unpublished(node))?;
        self.window_of(hit.node).is_none().then_some((hit.node, hit.local))
    }

    /// Whether the shell holds the tree's placement, so window requests are the shell's to decide.
    #[must_use]
    pub fn shell_places(&self) -> bool {
        self.scene.shell_places()
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
        self.set_window_source(window, buffer.map(SurfaceSource::of_buffer));
    }

    /// [`Composer::set_window_buffer`] for pixels that are not a client's live buffer.
    pub fn set_window_source(&mut self, window: WindowId, source: Option<SurfaceSource>) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        let node = entry.node;
        entry.buffer = source;
        // A View that draws this window re-reads its pixels from the new buffer.
        if let Some(node) = node {
            let _ = self.scene.mark_dirty(node);
        }
    }

    /// Replaces the surfaces of a window's tree below its toplevel surface: its subsurfaces and its popups.
    ///
    /// Each surface is a node of its own under its parent's node, so it moves, fades and is captured with the
    /// window, and it is damaged exactly where it is drawn: a surface that is new, moved, resized or reordered
    /// damages its rectangle, one that is no longer in the list damages the rectangle it held, and one that
    /// committed since the last list ([`TreeSurface::damaged`]) damages its rectangle though nothing else changed.
    /// The window's own node keeps the toplevel surface alone. Nodes are made when the window has a node, so a list
    /// given to a window that is not drawn is kept and built when it is; a window that is closing keeps the tree it
    /// had until it is gone.
    pub fn set_window_surfaces(&mut self, window: WindowId, surfaces: Vec<TreeSurface>) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        entry.tree = surfaces;
        self.sync_tree(window);
    }

    /// Makes the scene nodes under a window's node match its tree: nodes for new surfaces, none for gone ones,
    /// each node's place, size and order, and the tree's view of all of it.
    fn sync_tree(&mut self, window: WindowId) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        let Some(window_node) = entry.node else {
            return;
        };
        if entry.closing.is_some() || (entry.tree.is_empty() && entry.tree_nodes.is_empty()) {
            return;
        }
        let mut tree = std::mem::take(&mut entry.tree);
        let mut nodes = std::mem::take(&mut entry.tree_nodes);
        // A surface counts once, and only under a parent that came before it.
        let mut seen = BTreeSet::new();
        tree.retain(|surface| surface.parent.is_none_or(|parent| seen.contains(&parent)) && seen.insert(surface.key));
        self.remove_stale_tree_nodes(&tree, &mut nodes);
        for surface in &tree {
            let parent = match surface.parent {
                None => window_node,
                Some(parent) => match nodes.get(&parent) {
                    Some(parent) => parent.node,
                    None => continue,
                },
            };
            let bounds = Rect::from_size(surface.content.size);
            let transform = Self::tree_transform(surface);
            if let Some(existing) = nodes.get(&surface.key) {
                self.update_tree_node(existing.node, bounds, transform, surface);
            } else if let Some(node) = self.create_tree_node(parent, bounds, transform, surface) {
                nodes.insert(
                    surface.key,
                    TreeNode {
                        node,
                        parent: surface.parent,
                        role: surface.role,
                        published: [f64::NAN; 4],
                    },
                );
            }
        }
        self.order_tree_nodes(window_node, &tree, &nodes);
        for surface in &tree {
            if let Some(node) = nodes.get_mut(&surface.key) {
                self.publish_tree_geometry(node, surface);
            }
        }
        for surface in &mut tree {
            surface.damaged = false;
        }
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        entry.tree_keys = nodes.iter().map(|(key, node)| (node.node, *key)).collect();
        entry.tree_nodes = nodes;
        entry.tree = tree;
    }

    /// Removes the nodes of surfaces that are gone from `tree`, or that now have another parent or role, along with
    /// everything under them: a surface still wanted under a removed node is made again by the caller.
    fn remove_stale_tree_nodes(&mut self, tree: &[TreeSurface], nodes: &mut BTreeMap<u64, TreeNode>) {
        let wanted: BTreeMap<u64, &TreeSurface> = tree.iter().map(|surface| (surface.key, surface)).collect();
        let mut stale: BTreeSet<u64> = nodes
            .iter()
            .filter(|(key, node)| {
                wanted
                    .get(key)
                    .is_none_or(|surface| surface.parent != node.parent || surface.role != node.role)
            })
            .map(|(key, _)| *key)
            .collect();
        loop {
            let below: Vec<u64> = nodes
                .iter()
                .filter(|(key, node)| !stale.contains(key) && node.parent.is_some_and(|parent| stale.contains(&parent)))
                .map(|(key, _)| *key)
                .collect();
            if below.is_empty() {
                break;
            }
            stale.extend(below);
        }
        let tops: Vec<NodeId> = stale
            .iter()
            .filter_map(|key| nodes.get(key))
            .filter(|node| node.parent.is_none_or(|parent| !stale.contains(&parent)))
            .map(|node| node.node)
            .collect();
        for node in tops {
            self.drop_node(node);
        }
        nodes.retain(|key, _| !stale.contains(key));
    }

    /// The transform a surface's node holds: its offset in its parent's plane and its alpha factor.
    fn tree_transform(surface: &TreeSurface) -> VisualTransform {
        let mut transform = VisualTransform::IDENTITY;
        transform.base.offset = Vec3::new(f64::from(surface.offset.x), f64::from(surface.offset.y), 0.0);
        transform.opacity = if surface.alpha.is_finite() {
            surface.alpha.clamp(0.0, 1.0)
        } else {
            1.0
        };
        transform
    }

    /// Makes the node of a surface that has none, under `parent`, and announces it to the tree as a `surface`.
    fn create_tree_node(
        &mut self,
        parent: NodeId,
        bounds: Rect,
        transform: VisualTransform,
        surface: &TreeSurface,
    ) -> Option<NodeId> {
        let Some(id) = self.node_ids.allocate() else {
            log::warn!(
                area = "render",
                "no compositor node id is left; a surface of a window is not drawn"
            );
            return None;
        };
        let mut spec = NodeSpec::new(bounds);
        spec.parent = Some(parent);
        spec.transform = transform;
        // The pointer reaches a popup or a subsurface through the protocol engine's own surface lookup, which is
        // what knows input regions, so the scene's pick passes through these nodes.
        spec.input = false;
        if let Err(error) = self.scene.insert(id, spec) {
            log::warn!(
                area = "render",
                ?error,
                "the node of a surface of a window is refused; it is not drawn"
            );
            return None;
        }
        let role = surface.role.name();
        let _ = self.scene.set_class(id, Some("surface".to_owned()));
        let _ = self.scene.set_text(id, "role", role.to_owned());
        let _ = self.scene.set_value(id, "mapped", TreeValue::Bool(true));
        let _ = self.scene.set_behind(id, surface.behind);
        let index = self.scene.root_ids().iter().position(|root| *root == id).unwrap_or(0);
        self.tree_changes.push(TreeChange::Created {
            node: id,
            class: "surface".to_owned(),
            index,
        });
        let position = self
            .scene
            .children(parent)
            .and_then(|children| children.iter().position(|child| *child == id))
            .unwrap_or(0);
        self.tree_changes.push(TreeChange::Parented {
            node: id,
            parent,
            index: position,
        });
        for (key, value) in [
            ("role", TreeValue::Text(role.to_owned())),
            ("mapped", TreeValue::Bool(true)),
        ] {
            self.tree_changes.push(TreeChange::Set {
                node: id,
                key: key.to_owned(),
                value,
            });
        }
        Some(id)
    }

    /// Brings a surface's node to its size, place and paint side, and damages it when the surface committed.
    fn update_tree_node(&mut self, node: NodeId, bounds: Rect, transform: VisualTransform, surface: &TreeSurface) {
        if self.scene.bounds(node) != Some(bounds) {
            let _ = self.scene.set_bounds(node, bounds);
        }
        // A write of the transform it already holds still damages the node, which is what a commit means: the
        // buffer's handle is the same and its pixels are not.
        if surface.damaged || self.scene.transform(node) != Some(&transform) {
            let _ = self.scene.set_transform(node, transform);
        }
        let _ = self.scene.set_behind(node, surface.behind);
    }

    /// Puts the children of the window's node, and of each surface's node, in paint order: the surfaces behind their
    /// parent first, then whatever else is there (the shell's own nodes keep their order among themselves), then
    /// the surfaces above, each group in the order the tree gave.
    fn order_tree_nodes(&mut self, window_node: NodeId, tree: &[TreeSurface], nodes: &BTreeMap<u64, TreeNode>) {
        let parents =
            std::iter::once((window_node, None)).chain(nodes.iter().map(|(key, node)| (node.node, Some(*key))));
        for (parent, key) in parents.collect::<Vec<_>>() {
            let mine: Vec<(NodeId, bool)> = tree
                .iter()
                .filter(|surface| surface.parent == key)
                .filter_map(|surface| Some((nodes.get(&surface.key)?.node, surface.behind)))
                .collect();
            if mine.is_empty() {
                continue;
            }
            let current = self.scene.children(parent).unwrap_or(&[]).to_vec();
            let owned: BTreeSet<NodeId> = mine.iter().map(|(node, _)| *node).collect();
            let mut order: Vec<NodeId> = mine
                .iter()
                .filter(|(_, behind)| *behind)
                .map(|(node, _)| *node)
                .collect();
            order.extend(current.iter().filter(|node| !owned.contains(node)));
            order.extend(mine.iter().filter(|(_, behind)| !*behind).map(|(node, _)| *node));
            if order != current {
                let _ = self.scene.set_order(Some(parent), &order);
            }
        }
    }

    /// Publishes a surface's position and size to the tree when they changed since the last publish.
    fn publish_tree_geometry(&mut self, node: &mut TreeNode, surface: &TreeSurface) {
        let values = [
            f64::from(surface.offset.x),
            f64::from(surface.offset.y),
            f64::from(surface.content.size.width),
            f64::from(surface.content.size.height),
        ];
        for (index, (key, value)) in ["x", "y", "width", "height"].into_iter().zip(values).enumerate() {
            if node.published[index] == value {
                continue;
            }
            node.published[index] = value;
            self.tree_changes.push(TreeChange::Set {
                node: node.node,
                key: key.to_owned(),
                value: TreeValue::Double(value),
            });
        }
    }

    /// Removes the nodes of a window's tree and forgets the tree, leaving the window's own node.
    fn drop_tree_nodes(&mut self, window: WindowId) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        let nodes = std::mem::take(&mut entry.tree_nodes);
        entry.tree_keys.clear();
        entry.tree.clear();
        let tops: Vec<NodeId> = nodes
            .values()
            .filter(|node| node.parent.is_none_or(|parent| !nodes.contains_key(&parent)))
            .map(|node| node.node)
            .collect();
        for node in tops {
            self.drop_node(node);
        }
    }

    /// Sets the paint order, topmost first.
    pub fn set_stacking(&mut self, order: &[WindowId]) {
        let mut kept: Vec<WindowId> = Vec::with_capacity(self.order.len());
        // Each window is kept once; a repeat later in the reversed order is dropped.
        let mut seen: BTreeSet<WindowId> = BTreeSet::new();
        for window in order.iter().rev() {
            if self.windows.contains_key(window) && seen.insert(*window) {
                kept.push(*window);
            }
        }
        // Windows the policy did not name keep their place, above the ones it did, so
        // a half-applied order cannot make a window disappear.
        for window in &self.order {
            if seen.insert(*window) {
                kept.push(*window);
            }
        }
        if kept == self.order {
            return;
        }
        self.order = kept;
        // The scene's sibling order is the paint order. Only windows with a node are
        // siblings in the scene, so the order is applied to those, bottom first.
        // The compositor's own roots (the shell's root) keep the bottom, below every window.
        let window_nodes: Vec<NodeId> = self
            .order
            .iter()
            .filter_map(|window| self.windows.get(window)?.node)
            .collect();
        let mut nodes: Vec<NodeId> = self
            .scene
            .root_ids()
            .iter()
            .copied()
            .filter(|id| !window_nodes.contains(id))
            .collect();
        nodes.extend(window_nodes);
        let _ = self.scene.set_order(None, &nodes);
        let windows: Vec<WindowId> = self.order.clone();
        for window in windows {
            self.sync(window, false);
        }
    }

    /// Every window, topmost first.
    ///
    /// With no shell the order is the one [`Composer::set_stacking`] keeps. While a shell holds the tree its windows
    /// sit under its own nodes, so the order is the scene's: a depth-first walk, children after their parent, each
    /// sibling after the one below it. A window that has no node yet is not drawn, and counts as the topmost.
    #[must_use]
    pub fn window_stack(&self) -> Vec<WindowId> {
        if !self.scene.shell_places() {
            return self.order.iter().rev().copied().collect();
        }
        let by_node: BTreeMap<NodeId, WindowId> = self
            .windows
            .iter()
            .filter_map(|(window, entry)| Some((entry.node?, *window)))
            .collect();
        let mut bottom_first: Vec<WindowId> = Vec::with_capacity(self.windows.len());
        let mut pending: Vec<NodeId> = self.scene.root_ids().iter().rev().copied().collect();
        while let Some(node) = pending.pop() {
            bottom_first.extend(by_node.get(&node));
            if let Some(children) = self.scene.children(node) {
                pending.extend(children.iter().rev());
            }
        }
        let placed: BTreeSet<WindowId> = bottom_first.iter().copied().collect();
        bottom_first.extend(self.order.iter().filter(|window| !placed.contains(window)));
        bottom_first.reverse();
        bottom_first
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
        // A canvas draws in the scene's paint order against the windows: above its parent and what is under that,
        // below what the tree puts after it. Windows keep the stacking order; canvases wait their turn.
        let ranks = if self.canvases.is_empty() {
            BTreeMap::new()
        } else {
            self.paint_ranks()
        };
        let mut canvases: Vec<(u32, NodeId)> = self
            .canvases
            .iter()
            .filter(|(node, picture)| picture.image.is_some() && self.scene.drawn(**node))
            .filter_map(|(node, _)| Some((*ranks.get(node)?, *node)))
            .collect();
        canvases.sort_unstable();
        let mut waiting = canvases.into_iter().peekable();
        for window in &self.order {
            let Some(entry) = self.windows.get(window) else {
                continue;
            };
            let Some(window_node) = entry.node else {
                continue;
            };
            if let Some(rank) = ranks.get(&window_node) {
                while let Some((_, canvas)) = waiting.next_if(|(canvas_rank, _)| canvas_rank < rank) {
                    self.push_canvas(canvas, rect, frame, &mut spatial);
                }
            }
            self.push_window(*window, entry, window_node, rect, frame, &mut spatial);
        }
        for (_, canvas) in waiting {
            self.push_canvas(canvas, rect, frame, &mut spatial);
        }
        // A View draws its source's picture at its own rectangle, above the windows it is
        // painted over and below the visual bell.
        //
        // On the GPU path the picture is a texture the renderer draws from the View's scene, so the frame carries the
        // scene and a layer that samples its texture. Every GPU View is named alive, shown here or not, so the
        // renderer keeps the texture of one that is off this output and gives back the texture of one that is gone.
        frame.set_live_views(
            self.views
                .iter()
                .filter(|(_, picture)| picture.scene.is_some())
                .map(|(view, _)| view.get())
                .collect(),
        );
        for view in self.view_nodes() {
            let Some(picture) = self.views.get(&view) else {
                continue;
            };
            let source = match (picture.image.as_ref(), picture.scene.as_ref()) {
                (Some(image), _) => SurfaceSource::Snapshot(Arc::clone(image)),
                (None, Some(_)) => SurfaceSource::View(ViewTexture { id: view.get() }),
                (None, None) => continue,
            };
            let Some(bounds) = self.scene.absolute_bounds(view) else {
                continue;
            };
            let placed = Rect::new(bounds.x, bounds.y, picture.size.width, picture.size.height);
            if placed.intersection(rect).is_none() {
                continue;
            }
            if let Some(scene) = &picture.scene {
                frame.push_view_scene(Arc::clone(scene));
            }
            frame.push_layer(Layer::Surface(Surface {
                source,
                src: RectF {
                    x: 0.0,
                    y: 0.0,
                    width: f64::from(picture.size.width),
                    height: f64::from(picture.size.height),
                },
                dst: local(placed, rect),
                transform: Transform::Normal,
                opacity: 1.0,
                tint: Color::new(1.0, 1.0, 1.0, 1.0),
                corner_radius: 0.0,
                effects: self.scene.effects(view).cloned().unwrap_or_default(),
                model: None,
                // A capture of the output replaces the picture of a View that holds a secure node, so the output
                // reveals nothing a View's own capture hides.
                secure: !picture.secure.is_empty() || self.secure_scope.contains(&view),
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

    /// The position of every node in the scene's paint order, bottom first: a depth-first walk that visits a node
    /// before its children and a sibling after the one below it.
    fn paint_ranks(&self) -> BTreeMap<NodeId, u32> {
        let mut ranks = BTreeMap::new();
        let mut pending: Vec<NodeId> = self.scene.root_ids().iter().rev().copied().collect();
        while let Some(node) = pending.pop() {
            let rank = u32::try_from(ranks.len()).unwrap_or(u32::MAX);
            ranks.insert(node, rank);
            pending.extend(self.scene.children(node).unwrap_or(&[]).iter().rev());
        }
        ranks
    }

    /// Adds the layer of a Canvas node's picture, where the scene places the node now and at the opacity and
    /// corner radius it composes to.
    fn push_canvas(&self, canvas: NodeId, output: Rect, frame: &mut OutputFrame, spatial: &mut bool) {
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
            corner_radius: transform.corner_radius,
            effects: self.scene.effects(canvas).cloned().unwrap_or_else(Effects::new),
            model,
            secure: self.secure_scope.contains(&canvas),
        }));
    }

    /// How a node is placed on `output`: `None` for a flat rectangle drawn in paint order, which
    /// is every node in a planar scene under the orthographic projection.
    fn model(&self, node: NodeId, output: Rect) -> Option<Model> {
        let world = self.spatial_world(node)?;
        Some(Model {
            matrix: Mat4::translation(-f64::from(output.x), -f64::from(output.y), 0.0).mul(&world),
            opaque: false,
        })
    }

    /// The node's world matrix when it is placed in 3D, `None` for a flat rectangle: a planar
    /// world matrix under the orthographic projection is drawn by the flat path.
    fn spatial_world(&self, node: NodeId) -> Option<Mat4> {
        let world = self.scene.world_matrix(node)?;
        if world.is_planar() && matches!(self.scene.projection_for(node)?, Projection::Orthographic) {
            return None;
        }
        Some(world)
    }

    /// The effects of `node` the renderer will not draw, one `<kind>: <reasons>` text each, in list order. A kind the
    /// renderer cannot draw at all is `unsupported-by-renderer`. A blur or a shadow on a node placed in 3D is
    /// `unsupported-effect-on-3d`: the renderers have no depth pass for a backdrop or a shadow, so they draw the layer
    /// without them. An effect with both reasons names both, joined by `, `.
    fn skipped_effects(&self, node: NodeId) -> Vec<String> {
        let Some(effects) = self.scene.effects(node) else {
            return Vec::new();
        };
        if effects.is_empty() {
            return Vec::new();
        }
        let in_3d = self.spatial_world(node).is_some();
        effects
            .passes()
            .iter()
            .filter_map(|pass| {
                let name = pass.name();
                let mut reasons = Vec::new();
                if !self.effect_kinds.contains(&name) {
                    reasons.push(SKIPPED_BY_RENDERER);
                }
                if in_3d {
                    reasons.push(SKIPPED_ON_3D);
                }
                (!reasons.is_empty()).then(|| format!("{name}: {}", reasons.join(", ")))
            })
            .collect()
    }

    /// Tells the composer which effect kinds the active renderer draws, and publishes them on the main seat's
    /// read-only `effect_kinds` so a shell can adapt before it sets an effect. The compositor calls it once, from the
    /// backend, before any frame; the next [`Self::publish_skipped_effects`] names what is not drawn.
    pub fn set_effect_kinds(&mut self, kinds: &'static [&'static str]) {
        self.effect_kinds = kinds;
        self.publish_effect_kinds();
    }

    /// Publishes the seat's `effect_kinds` when it differs from what the seat holds.
    fn publish_effect_kinds(&mut self) {
        let Some(seat) = self.main_seat else {
            return;
        };
        let value = TreeValue::ListText(self.effect_kinds.iter().map(|kind| (*kind).to_owned()).collect());
        if self.scene.value(seat, "effect_kinds") != Some(&value) {
            let _ = self.scene.set_value(seat, "effect_kinds", value.clone());
            self.tree_changes.push(TreeChange::Set {
                node: seat,
                key: "effect_kinds".to_owned(),
                value,
            });
        }
    }

    /// Publishes each node's read-only `skipped` list when it changes. A node whose effects the
    /// renderer cannot draw gets the list; a node that stops needing it gets the empty list back.
    /// Only a change is published, so a steady scene sends nothing. Called once per frame, after
    /// the tree's writes and the placement have settled.
    pub fn publish_skipped_effects(&mut self) {
        let mut now = BTreeMap::new();
        for node in self.scene.effect_nodes() {
            let skipped = self.skipped_effects(node);
            if !skipped.is_empty() {
                now.insert(node, skipped);
            }
        }
        let cleared: Vec<NodeId> = self
            .skipped_published
            .keys()
            .filter(|node| !now.contains_key(node))
            .copied()
            .collect();
        for node in cleared {
            self.skipped_published.remove(&node);
            self.write_skipped(node, Vec::new());
        }
        for (node, skipped) in now {
            if self.skipped_published.get(&node) != Some(&skipped) {
                self.skipped_published.insert(node, skipped.clone());
                self.write_skipped(node, skipped);
            }
        }
    }

    /// Stores a node's `skipped` list and queues it for the next change batch. A node that is
    /// gone has nothing to write: its destruction is already queued.
    fn write_skipped(&mut self, node: NodeId, skipped: Vec<String>) {
        let value = TreeValue::ListText(skipped);
        if self.scene.set_value(node, "skipped", value.clone()).is_ok() {
            self.tree_changes.push(TreeChange::Set {
                node,
                key: "skipped".to_owned(),
                value,
            });
        }
    }

    /// The transform a window's node should reach: its base transform placed at its geometry.
    fn target(entry: &WindowNode) -> VisualTransform {
        // The shell's visuals and the client's and rule's factors live in their own fields and
        // are folded in here, so a placement that rewrites the placement's own transform never
        // clobbers them.
        let mut transform = entry.transform;
        let (shell_x, shell_y) = entry.shell_offset;
        transform.base.offset = Vec3::new(
            f64::from(entry.geometry.x - entry.surface_offset.0) + shell_x,
            f64::from(entry.geometry.y - entry.surface_offset.1) + shell_y,
            transform.base.offset.z,
        );
        let scale = f64::from(entry.shell_scale);
        transform.base.scale.x *= scale;
        transform.base.scale.y *= scale;
        transform.opacity *= entry.shell_opacity * entry.surface_alpha * entry.rule_opacity;
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
        let target = self.placed_target(entry);
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
            // A closing window keeps its node and picture until its fade ends. Its
            // stacking position is the scene's sibling order, set by `set_stacking`.
            return;
        }

        let content = if entry.visible { entry.content } else { None };
        let node = entry.node;
        let transform = self.placed_target(entry);
        let Some(content) = content else {
            if let Some(node) = node {
                self.cancel_tracks(window);
                self.drop_node(node);
            }
            if let Some(entry) = self.windows.get_mut(&window) {
                entry.node = None;
                // The nodes under the window's node went with it; the tree is kept to be made again.
                entry.tree_nodes.clear();
                entry.tree_keys.clear();
            }
            return;
        };
        let bounds = Rect::from_size(content.size);
        match node {
            Some(node) => {
                // The window keeps its node: a size change moves the node's bounds, so its id and
                // the tree's view of it stay the same.
                let resized = self.windows.get(&window).and_then(|entry| entry.applied_size) != Some(content.size);
                if resized {
                    if self.scene.bounds(node) != Some(bounds) {
                        let _ = self.scene.set_bounds(node, bounds);
                    }
                    if let Some(entry) = self.windows.get_mut(&window) {
                        entry.applied_size = Some(content.size);
                    }
                }
                if force && let Some(current) = self.scene.transform(node).copied() {
                    let _ = self.scene.set_transform(node, current);
                }
                self.publish_geometry(window);
                self.retarget(window, animate);
                self.sync_tree(window);
            }
            None => {
                // The window has no node yet: it gets one and takes its target at once, except that
                // a window which maps fades in from transparent. The id is taken first, so an
                // exhausted allocator leaves the window without a node.
                let Some(id) = self.node_ids.allocate() else {
                    log::warn!(area = "render", %window, "no compositor node id is left; the window is not drawn");
                    return;
                };
                let fades_in = !self.reduced_motion;
                let mut spec_transform = transform;
                if fades_in {
                    spec_transform.opacity = 0.0;
                }
                let spec = NodeSpec {
                    parent: None,
                    bounds,
                    transform: spec_transform,
                    effects: Effects::new(),
                    kind: NodeKind::Visual,
                    visible: true,
                    clip: false,
                    input: true,
                };
                match self.scene.insert(id, spec) {
                    Ok(()) => {
                        let (title, app_id) = match self.windows.get(&window) {
                            Some(entry) => (entry.title.clone(), entry.app_id.clone()),
                            None => (String::new(), String::new()),
                        };
                        let _ = self.scene.set_class(id, Some("window".to_owned()));
                        let _ = self.scene.set_text(id, "title", title.clone());
                        let _ = self.scene.set_text(id, "app_id", app_id.clone());
                        let index = self.scene.root_ids().iter().position(|root| *root == id).unwrap_or(0);
                        self.tree_changes.push(TreeChange::Created {
                            node: id,
                            class: "window".to_owned(),
                            index,
                        });
                        self.tree_ready.push(id);
                        for (key, value) in [("title", title), ("app_id", app_id)] {
                            if !value.is_empty() {
                                self.tree_changes.push(TreeChange::Set {
                                    node: id,
                                    key: key.to_owned(),
                                    value: TreeValue::Text(value),
                                });
                            }
                        }
                        if let Some(entry) = self.windows.get_mut(&window) {
                            entry.node = Some(id);
                            entry.applied_size = Some(content.size);
                            if let Some(extent) = entry.hit_extent {
                                let _ = self.scene.set_hit_extent(id, Some(extent));
                            }
                        }
                        self.publish_geometry(window);
                        if fades_in {
                            self.retarget(window, true);
                        }
                        self.sync_tree(window);
                        self.publish_relations(window);
                        self.publish_dialogs_of(window);
                    }
                    Err(error) => {
                        log::warn!(area = "render", %window, ?error, "the window's node is refused; it is not drawn");
                    }
                }
            }
        }
    }

    /// A window's node's transform as the scene holds it now, mid-tween while a property animates. `None` when
    /// the window has no node.
    fn pose(&self, window: WindowId) -> Option<Pose> {
        let node = self.windows.get(&window)?.node?;
        let transform = *self.scene.transform(node)?;
        Some(Pose { transform })
    }

    /// Adds the layers of a window's whole surface tree to `frame`: the surfaces behind the toplevel surface, the
    /// toplevel surface, then every surface above it, each in paint order.
    fn push_window(
        &self,
        window: WindowId,
        entry: &WindowNode,
        node: NodeId,
        output: Rect,
        frame: &mut OutputFrame,
        spatial: &mut bool,
    ) {
        let Some(pose) = self.pose(window) else {
            return;
        };
        let paint = Paint {
            output,
            window,
            entry,
            node,
            tint: pose.transform.tint,
            // Pixman's frame API rotates the whole destination rather than one layer, so a rotated window would
            // appear as its bounding box: the window and its tree are left out, and the scene's damage, which
            // covers the rotated bounds, repaints those pixels with the background rather than leaving stale ones.
            rotated: pose.transform.base.rotation_degrees.z != 0.0,
        };
        self.push_children(&paint, node, true, frame, spatial);
        let own = match &entry.closing {
            Some(closing) => Some((closing.source.clone(), closing.content)),
            // A window whose client has committed content but no buffer has no pixels to draw.
            None => entry
                .content
                .zip(entry.buffer.clone())
                .map(|(content, source)| (source, content)),
        };
        if let Some((source, content)) = own {
            let drawn = Drawn {
                source,
                content,
                // The window's placed pose, mid-tween, not its target.
                opacity: pose.transform.opacity,
                corner_radius: pose.transform.corner_radius,
                effects: self.scene.effects(node).cloned().unwrap_or_default(),
            };
            self.push_surface(&paint, node, drawn, frame, spatial);
        }
        self.push_children(&paint, node, false, frame, spatial);
    }

    /// Adds the layers of the surfaces under `parent` that paint behind it (`behind`) or in front of it, each with
    /// the surfaces under it.
    fn push_children(&self, paint: &Paint, parent: NodeId, behind: bool, frame: &mut OutputFrame, spatial: &mut bool) {
        for child in self.scene.children(parent).unwrap_or(&[]) {
            if self.scene.behind(*child) != Some(behind) {
                continue;
            }
            let Some(surface) = paint
                .entry
                .tree_keys
                .get(child)
                .and_then(|key| paint.entry.tree.iter().find(|surface| surface.key == *key))
            else {
                continue;
            };
            self.push_children(paint, *child, true, frame, spatial);
            // A surface of the tree takes the window's opacity and its own factors, and none of the window's
            // effects or corner radius: a shadow belongs to the toplevel, and a menu is not rounded by its window.
            let drawn = Drawn {
                source: surface.source.clone(),
                content: surface.content,
                opacity: self.opacity_within(paint.node, *child),
                corner_radius: 0.0,
                effects: Effects::new(),
            };
            self.push_surface(paint, *child, drawn, frame, spatial);
            self.push_children(paint, *child, false, frame, spatial);
        }
    }

    /// The product of the opacities from `node` up to `root` inclusive.
    fn opacity_within(&self, root: NodeId, node: NodeId) -> f32 {
        let mut opacity = 1.0;
        let mut current = Some(node);
        while let Some(id) = current {
            opacity *= self.scene.transform(id).map_or(1.0, |transform| transform.opacity);
            current = if id == root { None } else { self.scene.parent(id) };
        }
        opacity
    }

    /// Adds the layer of one surface of a window where the scene places its node now, unless it is on another
    /// output or cannot be drawn.
    fn push_surface(&self, paint: &Paint, node: NodeId, drawn: Drawn, frame: &mut OutputFrame, spatial: &mut bool) {
        let output = paint.output;
        let Some(placed) = self.scene.content_bounds(node).map(RectF::to_pixel) else {
            return;
        };
        let dst = Rect::new(
            placed.x.saturating_sub(output.x),
            placed.y.saturating_sub(output.y),
            placed.width,
            placed.height,
        );
        let model = self.model(node, output);
        let on_output = match model {
            // A projected surface's flat rectangle says nothing about where it lands.
            Some(_) => self
                .scene
                .absolute_bounds(node)
                .is_some_and(|bounds| bounds.intersection(output).is_some()),
            None => dst.intersection(Rect::new(0, 0, output.width, output.height)).is_some(),
        };
        if !on_output {
            return;
        }
        if model.is_some() && !*spatial {
            // A tilted surface's pixels can land anywhere on the output, so while one is on it the whole output is
            // repainted rather than its flat rectangle.
            frame.push_damage(Rect::new(0, 0, output.width, output.height));
            *spatial = true;
        }
        if paint.rotated {
            log::debug!(area = "render", window = %paint.window, "a rotated window is not drawn by the software renderer");
            return;
        }
        frame.push_layer(Layer::Surface(Surface {
            source: drawn.source,
            src: drawn.content.src,
            dst,
            transform: drawn.content.transform,
            opacity: drawn.opacity,
            tint: paint.tint,
            corner_radius: drawn.corner_radius,
            effects: drawn.effects,
            model,
            secure: self.secure_scope.contains(&node),
        }));
    }
}

/// The picture a View's capture draws in place of a secure node's pixels: the neutral grey an output capture draws
/// over a secure layer ([`SecureTreatment::Mask`]). One texel is enough, as the view renderer stretches a picture over
/// its node's bounds.
fn secure_mask() -> Image {
    Image::new(Size::new(1, 1), 4, vec![128, 128, 128, 255]).expect("a one-pixel image is valid")
}

/// What [`Composer::draw_view`] made of a View: its picture on the CPU, the capture's picture when a node is secure,
/// or the scene a GPU renderer draws it from. All three are `None` for a View that draws nothing.
#[derive(Default)]
struct DrawnView {
    image: Option<Arc<Image>>,
    redacted: Option<Arc<Image>>,
    scene: Option<Arc<ViewScene>>,
}

/// The pixels of one node of a View's source, in the form a GPU renderer samples.
struct ViewPixels {
    source: SurfaceSource,
    size: Size,
}

/// What one eye's walk of the source subtree draws with.
struct ViewEmit<'a> {
    /// The root of the source subtree, the world the camera sits in.
    root: NodeId,
    /// The camera: the subtree's world to the eye's pixels and clip depth.
    projection: &'a Mat4,
    /// The pixels of each node that has any.
    pictures: &'a BTreeMap<NodeId, ViewPixels>,
    /// The nodes to replace with `treatment`, for the capture variant; `None` for the display's.
    secure: Option<&'a BTreeSet<NodeId>>,
    treatment: SecureTreatment,
    /// The picture a masked node draws.
    mask: &'a Arc<Image>,
}

impl ViewEmit<'_> {
    /// The layer that draws `node`'s pixels over `bounds`, placed by `model` at `opacity`, or `None` for a node with
    /// no pixels, no extent, or one the capture omits.
    fn layer(&self, node: NodeId, bounds: Rect, opacity: f64, model: Model) -> Option<Layer> {
        if bounds.width == 0 || bounds.height == 0 {
            return None;
        }
        let pixels = self.pictures.get(&node)?;
        let (source, size) = if self.secure.is_some_and(|secure| secure.contains(&node)) {
            match self.treatment {
                SecureTreatment::Mask => (SurfaceSource::Snapshot(Arc::clone(self.mask)), Size::new(1, 1)),
                SecureTreatment::Omit => return None,
            }
        } else {
            (pixels.source.clone(), pixels.size)
        };
        Some(Layer::Surface(Surface {
            source,
            src: RectF {
                x: 0.0,
                y: 0.0,
                width: f64::from(size.width),
                height: f64::from(size.height),
            },
            dst: Rect::new(0, 0, bounds.width, bounds.height),
            transform: Transform::Normal,
            opacity: opacity.min(1.0) as f32,
            tint: Color::WHITE,
            corner_radius: 0.0,
            effects: Effects::new(),
            model: Some(model),
            secure: false,
        }))
    }
}

/// Whether a node's plane, through a camera, lands as a plane a picture can show: the matrix's columns for `u`, `v`
/// and the origin, over the rows for the two pixel coordinates and the divisor, are finite and not singular. The CPU
/// rig drops a node whose homography is not invertible together with its whole subtree, and so does the scene.
fn projects_to_a_plane(matrix: &Mat4) -> bool {
    let m = &matrix.m;
    let at = |row: usize, column: usize| m[[0, 1, 3][row]][[0, 1, 3][column]];
    let det = at(0, 0) * (at(1, 1) * at(2, 2) - at(1, 2) * at(2, 1))
        - at(0, 1) * (at(1, 0) * at(2, 2) - at(1, 2) * at(2, 0))
        + at(0, 2) * (at(1, 0) * at(2, 1) - at(1, 1) * at(2, 0));
    matrix.is_finite() && det.is_finite() && det != 0.0
}

/// The size of the picture a surface's source holds, or `None` for one with no extent or no pixels a GPU samples: a
/// buffer that is not a packed 32-bit shared-memory buffer is not one, as the CPU view cannot read it either.
fn source_size(source: &SurfaceSource) -> Option<Size> {
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
fn source_picture(source: &SurfaceSource, reader: &mut ReadDmabuf<'_>) -> Option<Image> {
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
fn straight_alpha(pixel: [u8; 4]) -> [u8; 4] {
    let alpha = u32::from(pixel[3]);
    if alpha == 0 || alpha == 255 {
        return pixel;
    }
    let lift = |channel: u8| ((u32::from(channel) * 255 + alpha / 2) / alpha).min(255) as u8;
    [lift(pixel[0]), lift(pixel[1]), lift(pixel[2]), pixel[3]]
}

/// A global rectangle in an output's own coordinates.
fn local(rect: Rect, output: Rect) -> Rect {
    rect.translate(-output.x, -output.y)
}

/// A window's node as the scene holds it this frame: its transform, which a running tween moves.
struct Pose {
    transform: VisualTransform,
}

/// What one window's layers are drawn with: the output, the window and the look every surface of its tree shares.
struct Paint<'a> {
    output: Rect,
    window: WindowId,
    entry: &'a WindowNode,
    /// The window's node, at the root of the tree.
    node: NodeId,
    tint: Color,
    /// Whether the window is rotated, which the software renderer cannot draw.
    rotated: bool,
}

/// What one surface's layer shows, which differs between the toplevel surface and the rest of the tree.
struct Drawn {
    source: SurfaceSource,
    content: Content,
    opacity: f32,
    corner_radius: f32,
    effects: Effects,
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
    use gnoblin_render::Actor;

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

    // The composer's nodes are compositor-owned, so they live in the compositor's id range. A
    // shell node may hold any id below the base, and must never block a window's node.
    #[test]
    fn every_node_the_composer_creates_has_a_compositor_id() {
        let (mut composer, output) = screen();
        for raw in 1..=3 {
            let window = WindowId::new(raw);
            composer.insert_window(window, Rect::new(10 * raw as i32, 10, 100, 100));
            composer.set_window_content(window, Some(content()));
        }
        composer.begin_frame();
        let mut frame = OutputFrame::new();
        composer.compose(output, &mut frame);
        composer.end_frame();
        // The compositor's root and the output's screen are nodes too; the windows are the rest.
        let screen = composer.screen_node(output);
        let ids: Vec<NodeId> = composer.with_scene(|scene| {
            let root = scene.root();
            scene
                .node_ids()
                .filter(|id| Some(*id) != root && Some(*id) != screen)
                .collect()
        });
        assert_eq!(ids.len(), 3, "one node per window with content");
        for id in ids {
            assert!(
                id.get() >= gnoblin_render::scene_ids::COMPOSITOR_ID_BASE,
                "composer node {id} is in the shell's id range"
            );
        }
    }

    #[test]
    fn a_shell_node_on_the_composers_first_id_does_not_block_a_window() {
        let (mut composer, _output) = screen();
        let shell = NodeId::new(1);
        composer.with_scene(|scene| {
            scene
                .insert_as(
                    gnoblin_render::Actor::Shell,
                    shell,
                    NodeSpec::new(Rect::new(0, 0, 10, 10)),
                )
                .expect("the shell may insert a node below the base");
        });
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(10, 10, 100, 100));
        composer.set_window_content(window, Some(content()));
        composer.with_scene(|scene| {
            assert!(scene.contains(shell), "the shell's node is untouched");
            assert_eq!(
                scene.node_ids().count(),
                4,
                "the shell, the root, the screen and the window each have a node"
            );
        });
    }

    #[test]
    fn a_window_node_id_is_never_reused_within_a_session() {
        let (mut composer, _output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(10, 10, 100, 100));
        composer.set_window_content(window, Some(content()));
        let first = node_of(&composer, window);
        composer.remove_window(window);
        let again = WindowId::new(2);
        composer.insert_window(again, Rect::new(10, 10, 100, 100));
        composer.set_window_content(again, Some(content()));
        let second = node_of(&composer, again);
        assert!(second.get() > first.get(), "{second} reuses or precedes {first}");
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

    // The shell marks a window secure by writing `secure` on its node. The composer follows the node: the mark
    // takes effect when the shell writes it, and clears when the shell writes false.
    #[test]
    fn a_secure_value_the_shell_wrote_on_the_node_marks_the_window() {
        let (mut composer, output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(10, 10, 100, 100));
        composer.set_window_content(window, Some(content()));
        let node = node_of(&composer, window);
        assert!(damage_of(&mut composer, output).is_some(), "the new node is drawn once");

        composer.follow_shell_secure();
        assert_eq!(
            damage_of(&mut composer, output),
            None,
            "a node with no secure value keeps the window's mark"
        );

        composer.with_scene(|scene| {
            scene
                .set_value(node, "secure", TreeValue::Bool(true))
                .expect("the node takes it")
        });
        composer.follow_shell_secure();
        assert!(
            damage_of(&mut composer, output).is_some(),
            "the shell's mark redraws the window for capture"
        );
        composer.follow_shell_secure();
        assert_eq!(damage_of(&mut composer, output), None, "the same mark damages nothing");

        composer.with_scene(|scene| {
            scene
                .set_value(node, "secure", TreeValue::Bool(false))
                .expect("the node takes it")
        });
        composer.follow_shell_secure();
        assert!(
            damage_of(&mut composer, output).is_some(),
            "the shell clearing the mark redraws the window"
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

    /// Adds a View at (10, 20), 40 by 30, under `parent` or the root, sourced from `source`.
    fn add_view(composer: &mut Composer, view: NodeId, parent: Option<NodeId>, source: Option<NodeId>) {
        composer.with_scene(|scene| {
            let mut spec = NodeSpec::new(Rect::new(10, 20, 40, 30));
            spec.parent = parent;
            scene.insert(view, spec).expect("a view node");
            scene
                .set_class(view, Some("view".to_owned()))
                .expect("the view's class");
            scene.set_source(view, source).expect("the view's source");
        });
    }

    /// Adds the root of a view's source subtree: a plain node at `rect`.
    fn add_source(composer: &mut Composer, source: NodeId, rect: Rect) {
        composer.with_scene(|scene| scene.insert(source, NodeSpec::new(rect)).expect("a source node"));
    }

    /// The placement of each View picture the next frame for `output` carries.
    fn view_layers(composer: &mut Composer, output: OutputId) -> Vec<Rect> {
        composer.begin_frame();
        let mut frame = OutputFrame::new();
        composer.compose(output, &mut frame);
        composer.end_frame();
        frame
            .layers()
            .iter()
            .filter_map(|layer| match layer {
                Layer::Surface(surface) if matches!(surface.source, SurfaceSource::Snapshot(_)) => Some(surface.dst),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_view_with_a_source_draws_its_picture_where_it_sits() {
        let (mut composer, output) = screen();
        let source = NodeId::new(2);
        add_source(&mut composer, source, Rect::new(0, 0, 40, 30));
        add_view(&mut composer, NodeId::new(1), None, Some(source));
        assert_eq!(view_layers(&mut composer, output), vec![Rect::new(10, 20, 40, 30)]);
    }

    #[test]
    fn a_view_without_a_source_draws_no_picture() {
        let (mut composer, output) = screen();
        add_view(&mut composer, NodeId::new(1), None, None);
        assert!(view_layers(&mut composer, output).is_empty());
    }

    #[test]
    fn a_view_inside_its_own_source_draws_no_picture() {
        let (mut composer, output) = screen();
        let source = NodeId::new(2);
        add_source(&mut composer, source, Rect::new(0, 0, 40, 30));
        add_view(&mut composer, NodeId::new(1), Some(source), Some(source));
        assert!(view_layers(&mut composer, output).is_empty());
    }

    #[test]
    fn a_view_redraws_only_when_its_source_changes() {
        let (mut composer, output) = screen();
        let source = NodeId::new(2);
        add_source(&mut composer, source, Rect::new(0, 0, 40, 30));
        add_view(&mut composer, NodeId::new(1), None, Some(source));
        assert!(
            damage_of(&mut composer, output).is_some(),
            "a new view damages its pixels"
        );
        assert!(
            damage_of(&mut composer, output).is_none(),
            "an unchanged view draws nothing new"
        );
        composer.with_scene(|scene| {
            scene
                .set_bounds(source, Rect::new(0, 0, 20, 20))
                .expect("the source resizes")
        });
        assert!(
            damage_of(&mut composer, output).is_some(),
            "a source change re-renders the view"
        );
    }

    #[test]
    fn a_view_of_a_window_redraws_when_the_window_node_is_marked_dirty() {
        let (mut composer, output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(0, 0, 40, 50));
        composer.set_window_content(window, Some(content()));
        let node = composer.window_node(window).expect("the window's scene node");
        add_view(&mut composer, NodeId::new(1), None, Some(node));
        assert!(
            damage_of(&mut composer, output).is_some(),
            "a new view damages its pixels"
        );
        assert!(
            damage_of(&mut composer, output).is_none(),
            "an unchanged view draws nothing new"
        );
        composer.with_scene(|scene| scene.mark_dirty(node).expect("the window node"));
        assert!(
            damage_of(&mut composer, output).is_some(),
            "a new window buffer re-renders the view"
        );
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
                source: SurfaceSource::Snapshot(picture()),
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
                source: SurfaceSource::Snapshot(picture()),
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
                source: SurfaceSource::Snapshot(picture()),
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
                source: SurfaceSource::Snapshot(picture()),
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

    /// The opacity of the snapshot surface a frame draws for a closing window, after a fade
    /// tick that moves the node so the frame has damage to draw.
    fn closing_snapshot_opacity(surface_alpha: Option<f32>) -> f32 {
        let (mut composer, output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.advance_motion(MOTION_MS);
        composer.unmap_with_snapshot(
            window,
            Some(Closing {
                source: SurfaceSource::Snapshot(picture()),
                content: content(),
                on_end: CloseEnd::Hide,
            }),
        );
        composer.advance_motion(MOTION_MS / 2);
        if let Some(factor) = surface_alpha {
            composer.set_window_surface_alpha(window, factor);
        }
        composer.advance_motion(MOTION_MS / 4);
        composer.begin_frame();
        let mut frame = OutputFrame::new();
        assert!(composer.compose(output, &mut frame), "the fade tick damages the frame");
        composer.end_frame();
        frame
            .layers()
            .iter()
            .find_map(|layer| match layer {
                Layer::Surface(Surface {
                    source: SurfaceSource::Snapshot(_),
                    opacity,
                    ..
                }) => Some(*opacity),
                _ => None,
            })
            .expect("a closing window draws its snapshot")
    }

    #[test]
    fn a_surface_factor_set_during_a_close_fade_does_not_change_the_fade() {
        assert_eq!(
            closing_snapshot_opacity(Some(0.5)),
            closing_snapshot_opacity(None),
            "a closing window keeps the opacity its fade drew, as its placement and shell already do"
        );
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
                source: SurfaceSource::Snapshot(picture()),
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
                source: SurfaceSource::Snapshot(picture()),
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
                source: SurfaceSource::Snapshot(picture()),
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
                source: SurfaceSource::Snapshot(picture()),
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
                source: SurfaceSource::Snapshot(picture()),
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
                source: SurfaceSource::Snapshot(picture()),
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
                source: SurfaceSource::Snapshot(picture()),
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
                source: SurfaceSource::Snapshot(picture()),
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
                source: SurfaceSource::Snapshot(picture()),
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
                source: SurfaceSource::Snapshot(picture()),
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

    #[test]
    fn shell_opacity_multiplies_the_placement_opacity_and_survives_placement() {
        let (mut composer, _output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(100, 100, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.set_reduced_motion(true);
        composer.set_window_opacity(window, 0.5);
        composer.set_shell_opacity(window, 0.5).expect("0.5 is in range");
        assert_eq!(
            drawn_pose(&composer, window).1,
            0.25,
            "the shell opacity multiplies the placement's"
        );
        composer.set_window_opacity(window, 0.8);
        assert_eq!(
            drawn_pose(&composer, window).1,
            0.4,
            "a placement write does not clobber the shell opacity"
        );
    }

    #[test]
    fn shell_offset_and_scale_survive_placement_and_reset_restores_the_window() {
        let (mut composer, _output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(100, 100, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.set_reduced_motion(true);
        composer.set_shell_offset(window, 10.0, -20.0).expect("in range");
        assert_eq!(drawn_pose(&composer, window).0, Rect::new(110, 80, 40, 50));
        composer.place_window_animated(window, Rect::new(200, 200, 40, 50), gnoblin_types::Transform::IDENTITY);
        assert_eq!(
            drawn_pose(&composer, window).0,
            Rect::new(210, 180, 40, 50),
            "a placement moves the window and the shell offset rides on top"
        );
        composer.set_shell_scale(window, 2.0).expect("in range");
        assert_eq!(
            drawn_pose(&composer, window).0,
            Rect::new(190, 155, 80, 100),
            "a shell scale of two is about the window's centre"
        );
        composer.set_shell_opacity(window, 0.5).expect("in range");
        composer.reset_shell_visuals(window);
        assert_eq!(
            drawn_pose(&composer, window),
            (Rect::new(200, 200, 40, 50), 1.0),
            "reset puts the window back at its placement, fully opaque"
        );
    }

    #[test]
    fn shell_visual_values_out_of_range_are_refused_and_leave_the_window_unchanged() {
        let (mut composer, _output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(0, 0, 10, 10));
        assert_eq!(
            composer.set_shell_opacity(window, 1.5),
            Err(ShellVisualError::OutOfRange)
        );
        assert_eq!(
            composer.set_shell_opacity(window, f32::NAN),
            Err(ShellVisualError::OutOfRange)
        );
        assert_eq!(
            composer.set_shell_scale(window, 0.001),
            Err(ShellVisualError::OutOfRange)
        );
        assert_eq!(
            composer.set_shell_scale(window, 10.5),
            Err(ShellVisualError::OutOfRange)
        );
        assert_eq!(
            composer.set_shell_offset(window, 100_001.0, 0.0),
            Err(ShellVisualError::OutOfRange)
        );
        assert_eq!(
            composer.set_shell_offset(window, 0.0, f64::NAN),
            Err(ShellVisualError::OutOfRange)
        );
        let node = &composer.windows[&window];
        assert_eq!(
            (node.shell_opacity, node.shell_scale, node.shell_offset),
            (1.0, 1.0, (0.0, 0.0))
        );
        assert!(
            composer.set_shell_opacity(window, 0.0).is_ok(),
            "the documented bounds are accepted"
        );
        assert!(composer.set_shell_scale(window, 10.0).is_ok());
        assert!(composer.set_shell_offset(window, -100_000.0, 100_000.0).is_ok());
    }

    /// The rectangle and opacity `window` is drawn at: its node's placed rectangle and the opacity of its
    /// transform, which are what the frame's layer for the window is built from.
    fn drawn_pose(composer: &Composer, window: WindowId) -> (Rect, f32) {
        let pose = composer.pose(window).expect("a window with content has a node");
        let node = composer.window_node(window).expect("a window with content has a node");
        let bounds = composer
            .scene
            .content_bounds(node)
            .expect("a node has bounds")
            .to_pixel();
        (bounds, pose.transform.opacity)
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

    /// The opacity `window`'s surface is drawn at: the node's, which holds every factor the
    /// window draws with, so this is what the frame draws.
    fn drawn_opacity(composer: &Composer, window: WindowId) -> f32 {
        drawn_pose(composer, window).1
    }

    #[test]
    fn the_four_opacity_factors_multiply_into_the_drawn_opacity() {
        let (mut composer, _output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(100, 100, 40, 50));
        composer.set_window_content(window, Some(content()));
        composer.set_reduced_motion(true);
        composer.set_window_opacity(window, 0.5);
        composer.set_shell_opacity(window, 0.5).expect("0.5 is in range");
        composer.set_window_surface_alpha(window, 0.5);
        composer.set_window_rule_opacity(window, 0.5);
        assert_eq!(
            drawn_opacity(&composer, window),
            0.0625,
            "the placement, shell, surface and rule opacities multiply"
        );
    }

    #[test]
    fn a_factor_change_lands_at_once_and_a_tween_runs_under_the_factors() {
        let (mut composer, _output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_content(window, Some(content()));
        // Let the fade-in land so the window starts opaque.
        composer.advance_motion(200);
        composer.set_window_surface_alpha(window, 0.5);
        assert_eq!(drawn_opacity(&composer, window), 0.5, "a factor change lands at once");
        composer.set_window_rule_opacity(window, 0.5);
        assert_eq!(
            drawn_opacity(&composer, window),
            0.25,
            "the rule's factor lands at once"
        );
        track_window(
            &mut composer,
            window,
            VisualProperty::Opacity,
            PropertyValue::Number(0.0),
            200,
        );
        composer.advance_motion(100);
        assert_eq!(
            drawn_opacity(&composer, window),
            0.125,
            "halfway through the tween, under the factors"
        );
    }

    #[test]
    fn a_window_that_maps_under_a_factor_fades_in_to_the_factored_opacity() {
        let (mut composer, _output) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(20, 30, 40, 50));
        composer.set_window_surface_alpha(window, 0.5);
        composer.set_window_content(window, Some(content()));
        assert_eq!(
            drawn_opacity(&composer, window),
            0.0,
            "a newly mapped window starts transparent"
        );
        composer.advance_motion(100);
        let halfway = drawn_opacity(&composer, window);
        assert!(
            halfway > 0.0 && halfway < 0.5,
            "the fade is part way through, got {halfway}"
        );
        composer.advance_motion(100);
        assert_eq!(
            drawn_opacity(&composer, window),
            0.5,
            "the fade lands on the factored opacity exactly"
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
    fn the_scene_paints_windows_in_the_stacking_order_above_the_compositor_root() {
        let (mut composer, _) = screen();
        let first = WindowId::new(1);
        let second = WindowId::new(2);
        composer.insert_window(first, Rect::new(0, 0, 10, 10));
        composer.set_window_content(first, Some(content()));
        composer.insert_window(second, Rect::new(5, 5, 10, 10));
        composer.set_window_content(second, Some(content()));
        composer.set_stacking(&[first, second]);
        let first_node = node_of(&composer, first);
        let second_node = node_of(&composer, second);
        let roots = composer.scene.root_ids().to_vec();
        let root = composer.scene.root().expect("the compositor root is set");
        assert_eq!(roots.first(), Some(&root), "the root paints first: {roots:?}");
        assert_eq!(
            roots.last(),
            Some(&first_node),
            "the topmost window paints last: {roots:?}"
        );
        assert_eq!(roots.len(), 3, "the root and both windows: {roots:?}");
        assert_eq!(
            roots.get(1),
            Some(&second_node),
            "the window below comes next: {roots:?}"
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
    fn the_lock_node_is_a_compositor_node_that_publishes_only_a_state_change() {
        let mut composer = Composer::new();
        let lock = composer.add_lock_node().expect("a lock node");
        let root = composer.with_scene(|scene| scene.root_ids()[0]);
        let (parent, class, state) = composer.with_scene(|scene| {
            (
                scene.parent(lock),
                scene.class(lock).map(str::to_owned),
                scene.text(lock, "state").map(str::to_owned),
            )
        });
        assert_eq!(parent, Some(root));
        assert_eq!(class.as_deref(), Some("lock"));
        assert_eq!(state.as_deref(), Some("not-locked"), "a new session is not locked");
        assert_eq!(composer.add_lock_node(), Some(lock), "one lock node");
        composer.take_tree_changes();

        composer.set_lock_state("locked");
        let changes = composer.take_tree_changes();
        assert!(changes.iter().any(|change| matches!(
            change,
            TreeChange::Set { node, key, value: TreeValue::Text(text) }
                if *node == lock && key == "state" && text == "locked"
        )));
        composer.set_lock_state("locked");
        assert!(
            composer.take_tree_changes().is_empty(),
            "an unchanged state is not republished"
        );
        assert!(
            composer
                .with_scene(|scene| scene.remove_as(Actor::Shell, lock))
                .is_err(),
            "the shell may not destroy the compositor's lock node"
        );
    }

    /// The lock asks for the lock screen once when it leaves `not-locked`: the shell is told to
    /// engage, whether the lock passes through `pending` or goes straight to `locked`, and a
    /// repeated publish does not ask again.
    #[test]
    fn the_lock_node_asks_the_shell_for_its_lock_screen_once_when_the_lock_leaves_not_locked() {
        let mut composer = Composer::new();
        let lock = composer.add_lock_node().expect("a lock node");
        assert!(composer.take_lock_requested().is_empty(), "no lock yet, nothing asked");

        composer.set_lock_state("pending");
        assert_eq!(composer.take_lock_requested(), vec![lock]);
        assert!(composer.take_lock_requested().is_empty(), "taken once");

        composer.set_lock_state("pending");
        assert!(
            composer.take_lock_requested().is_empty(),
            "an unchanged state asks nothing"
        );

        composer.set_lock_state("locked");
        assert!(composer.take_lock_requested().is_empty(), "locking asks nothing");
    }

    #[test]
    fn the_lock_node_asks_once_when_the_lock_goes_straight_to_locked() {
        let mut composer = Composer::new();
        let lock = composer.add_lock_node().expect("a lock node");

        composer.set_lock_state("locked");
        assert_eq!(composer.take_lock_requested(), vec![lock]);

        composer.set_lock_state("not-locked");
        assert!(composer.take_lock_requested().is_empty(), "unlocking asks nothing");
    }

    /// F22: the lock unlocks while the shell is gone. The lock node follows the session-lock
    /// state with no shell attached and no frame draining the changes, so the state the
    /// shell reads on attach is the session's, not one the shell has to answer for.
    #[test]
    fn the_lock_node_returns_to_not_locked_when_the_session_unlocks_with_no_shell() {
        let mut composer = Composer::new();
        let lock = composer.add_lock_node().expect("a lock node");
        composer.set_lock_state("pending");
        composer.set_lock_state("locked");
        composer.take_tree_changes();

        composer.set_lock_state("not-locked");
        let changes = composer.take_tree_changes();
        assert!(changes.iter().any(|change| matches!(
            change,
            TreeChange::Set { node, key, value: TreeValue::Text(text) }
                if *node == lock && key == "state" && text == "not-locked"
        )));
        assert_eq!(
            composer
                .with_scene(|scene| scene.text(lock, "state").map(str::to_owned))
                .as_deref(),
            Some("not-locked"),
            "the scene reads the unlocked state"
        );
    }

    #[test]
    fn the_main_seat_is_a_compositor_node_under_the_root() {
        let mut composer = Composer::new();
        let seat = composer.add_main_seat("seat0").expect("a seat node");
        let root = composer.with_scene(|scene| scene.root_ids()[0]);
        let (parent, class, name) = composer.with_scene(|scene| {
            (
                scene.parent(seat),
                scene.class(seat).map(str::to_owned),
                scene.text(seat, "name").map(str::to_owned),
            )
        });
        let shell_remove = composer.with_scene(|scene| scene.remove_as(Actor::Shell, seat).is_err());
        assert_eq!(parent, Some(root));
        assert_eq!(class.as_deref(), Some("seat"));
        assert_eq!(name.as_deref(), Some("seat0"));
        assert!(shell_remove, "the shell may not destroy the compositor's seat");
        assert_eq!(composer.add_main_seat("seat0"), Some(seat), "one main seat");
        assert!(composer.take_tree_changes().iter().any(|change| matches!(
            change,
            TreeChange::Created { node, class, .. } if *node == seat && class == "seat"
        )));
    }

    #[test]
    fn the_main_seat_publishes_its_window_management_policy() {
        let mut composer = Composer::new();
        let seat = composer.add_main_seat("seat0").expect("a seat node");
        composer.with_scene(|scene| {
            for key in ["edge_tiling", "focus_mode", "auto_raise_delay", "right_click"] {
                assert_eq!(scene.value(seat, key), None, "{key}: the default is not stored yet");
            }
        });

        let mut policy = MainSeatPolicy {
            edge_tiling: true,
            drag_constraint: "minimum_strip",
            focus_mode: "hover",
            focus_new_windows: "prevent",
            raise_on_click: true,
            auto_raise: true,
            auto_raise_delay: 250,
            focus_change_on_pointer_rest: false,
            double_click: "toggle-maximize",
            middle_click: "lower",
            right_click: "menu",
            workspaces: vec!["1:Workspace 1".to_owned(), "2:Workspace 2".to_owned()],
        };
        composer.publish_main_seat_policy(&policy);
        composer.with_scene(|scene| {
            assert_eq!(scene.value(seat, "edge_tiling"), Some(&TreeValue::Bool(true)));
            assert_eq!(
                scene.value(seat, "workspaces"),
                Some(&TreeValue::ListText(vec![
                    "1:Workspace 1".to_owned(),
                    "2:Workspace 2".to_owned()
                ]))
            );
            assert_eq!(scene.text(seat, "drag_constraint"), Some("minimum_strip"));
            assert_eq!(scene.text(seat, "focus_mode"), Some("hover"));
            assert_eq!(scene.text(seat, "focus_new_windows"), Some("prevent"));
            assert_eq!(scene.value(seat, "raise_on_click"), Some(&TreeValue::Bool(true)));
            assert_eq!(scene.value(seat, "auto_raise"), Some(&TreeValue::Bool(true)));
            assert_eq!(scene.value(seat, "auto_raise_delay"), Some(&TreeValue::Int(250)));
            assert_eq!(
                scene.value(seat, "focus_change_on_pointer_rest"),
                Some(&TreeValue::Bool(false))
            );
            assert_eq!(scene.text(seat, "double_click"), Some("toggle-maximize"));
            assert_eq!(scene.text(seat, "middle_click"), Some("lower"));
            assert_eq!(scene.text(seat, "right_click"), Some("menu"));
        });
        let changes = composer.take_tree_changes();
        assert!(changes.iter().any(|change| matches!(
            change,
            TreeChange::Set { node, key, value: TreeValue::Bool(true) } if *node == seat && key == "edge_tiling"
        )));
        assert!(changes.iter().any(|change| matches!(
            change,
            TreeChange::Set { node, key, value: TreeValue::Int(250) } if *node == seat && key == "auto_raise_delay"
        )));

        composer.publish_main_seat_policy(&policy);
        assert!(
            composer.take_tree_changes().is_empty(),
            "an unchanged policy is not republished"
        );

        policy.auto_raise_delay = 900;
        composer.publish_main_seat_policy(&policy);
        let changes = composer.take_tree_changes();
        assert_eq!(changes.len(), 1, "a reload publishes only the value it changed");
        assert!(matches!(
            &changes[0],
            TreeChange::Set { node, key, value: TreeValue::Int(900) } if *node == seat && key == "auto_raise_delay"
        ));
    }

    /// The main seat carries the reduced-motion setting the composer honours, so the stock shell
    /// reads the same answer the compositor applies. Only a change is published.
    #[test]
    fn the_main_seat_publishes_reduced_motion_and_republishes_it_when_it_changes() {
        let mut composer = Composer::new();
        let seat = composer.add_main_seat("seat0").expect("a seat node");
        let policy = MainSeatPolicy {
            edge_tiling: false,
            drag_constraint: "work_area",
            focus_mode: "click",
            focus_new_windows: "prevent",
            raise_on_click: true,
            auto_raise: false,
            auto_raise_delay: 500,
            focus_change_on_pointer_rest: false,
            double_click: "toggle-maximize",
            middle_click: "lower",
            right_click: "menu",
            workspaces: Vec::new(),
        };
        composer.publish_main_seat_policy(&policy);
        composer.with_scene(|scene| {
            assert_eq!(scene.value(seat, "reduced_motion"), Some(&TreeValue::Bool(false)));
        });
        composer.take_tree_changes();

        composer.set_reduced_motion(true);
        composer.publish_main_seat_policy(&policy);
        let changes = composer.take_tree_changes();
        assert_eq!(changes.len(), 1, "only the setting changed: {changes:?}");
        assert!(matches!(
            &changes[0],
            TreeChange::Set { node, key, value: TreeValue::Bool(true) } if *node == seat && key == "reduced_motion"
        ));

        composer.publish_main_seat_policy(&policy);
        assert!(
            composer.take_tree_changes().is_empty(),
            "an unchanged setting is not republished"
        );
    }

    #[test]
    fn the_main_seat_publishes_the_pointer_only_when_it_moves() {
        let mut composer = Composer::new();
        let seat = composer.add_main_seat("seat0").expect("a seat node");
        composer.take_tree_changes();
        composer.set_main_seat_pointer(3.0, 4.0);
        let changes = composer.take_tree_changes();
        assert!(changes.iter().any(|change| matches!(
            change,
            TreeChange::Set { node, key, value: TreeValue::Double(v) } if *node == seat && key == "pointer_x" && *v == 3.0
        )));
        assert!(changes.iter().any(|change| matches!(
            change,
            TreeChange::Set { node, key, value: TreeValue::Double(v) } if *node == seat && key == "pointer_y" && *v == 4.0
        )));
        composer.set_main_seat_pointer(3.0, 4.0);
        assert!(
            composer.take_tree_changes().is_empty(),
            "an unchanged pointer is not republished"
        );
    }

    #[test]
    fn the_main_seat_publishes_its_devices_only_when_the_list_changes() {
        let mut composer = Composer::new();
        let seat = composer.add_main_seat("seat0").expect("a seat node");
        composer.take_tree_changes();
        let list = vec!["1:Desk Keyboard:keyboard".to_owned(), "2:Mouse:pointer".to_owned()];
        composer.publish_main_seat_devices(list.clone());
        let changes = composer.take_tree_changes();
        assert!(changes.iter().any(|change| matches!(
            change,
            TreeChange::Set { node, key, value: TreeValue::ListText(items) } if *node == seat && key == "device" && *items == list
        )));
        composer.with_scene(|scene| assert_eq!(scene.value(seat, "device"), Some(&TreeValue::ListText(list.clone()))));
        composer.publish_main_seat_devices(list);
        assert!(
            composer.take_tree_changes().is_empty(),
            "an unchanged device list is not republished"
        );
        composer.publish_main_seat_devices(vec!["2:Mouse:pointer".to_owned()]);
        assert_eq!(composer.take_tree_changes().len(), 1, "a removal is published");
    }

    #[test]
    fn the_main_seat_publishes_the_screen_under_the_pointer() {
        let mut composer = Composer::new();
        let seat = composer.add_main_seat("seat0").expect("a seat node");
        composer.set_output(OutputId::new(0), Rect::new(0, 0, 800, 600));
        composer.set_output(OutputId::new(1), Rect::new(800, 0, 800, 600));
        let first = composer.screen_node(OutputId::new(0)).expect("the first screen");
        let second = composer.screen_node(OutputId::new(1)).expect("the second screen");
        let screen = |composer: &mut Composer| composer.with_scene(|scene| scene.reference(seat, "screen"));
        assert_eq!(
            screen(&mut composer),
            Some(first),
            "the first screen before any pointer move"
        );
        composer.set_main_seat_pointer(900.0, 100.0);
        assert_eq!(screen(&mut composer), Some(second), "the screen the pointer moved onto");
        assert!(composer.take_tree_changes().iter().any(|change| matches!(
            change,
            TreeChange::Set { node, key, value: TreeValue::Node(Some(target)) }
                if *node == seat && key == "screen" && *target == second
        )));
        composer.set_main_seat_pointer(900.0, 100.0);
        assert!(
            composer.take_tree_changes().is_empty(),
            "an unchanged screen is not republished"
        );
        composer.remove_output(OutputId::new(1));
        assert_eq!(
            screen(&mut composer),
            Some(first),
            "the first screen once the pointer's screen is gone"
        );
        composer.remove_output(OutputId::new(0));
        assert_eq!(screen(&mut composer), None, "no screen once every output is gone");
    }

    #[test]
    fn the_main_seat_publishes_the_focused_window_node() {
        let mut composer = Composer::new();
        let seat = composer.add_main_seat("seat0").expect("a seat node");
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(10, 10, 100, 100));
        composer.set_window_content(window, Some(content()));
        let node = node_of(&composer, window);
        composer.take_tree_changes();
        composer.set_main_seat_focus(Some(window));
        assert_eq!(composer.main_seat_focus(), Some(node));
        assert!(composer.take_tree_changes().iter().any(|change| matches!(
            change,
            TreeChange::Set { node: set, key, value: TreeValue::Node(Some(target)) }
                if *set == seat && key == "focus" && *target == node
        )));
        composer.set_main_seat_focus(None);
        assert_eq!(composer.main_seat_focus(), None);
        assert!(composer.take_tree_changes().iter().any(|change| matches!(
            change,
            TreeChange::Set { node: set, key, value: TreeValue::Node(None) } if *set == seat && key == "focus"
        )));
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

    /// The window's node, which it has once content arrives.
    fn node_of(composer: &Composer, window: WindowId) -> NodeId {
        composer.windows[&window].node.expect("the window has a node")
    }

    /// The x and y the scene draws the node at.
    fn offset_of(composer: &Composer, node: NodeId) -> (f64, f64) {
        let transform = composer.scene.transform(node).expect("the node has a transform");
        (transform.base.offset.x, transform.base.offset.y)
    }

    #[test]
    fn without_a_shell_the_window_manager_places_the_window() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(10, 10, 100, 100));
        composer.set_window_content(window, Some(content()));
        composer.place_window(window, Rect::new(200, 120, 100, 100));
        let node = node_of(&composer, window);
        assert_eq!(offset_of(&composer, node), (200.0, 120.0));
    }

    #[test]
    fn while_the_shell_holds_the_tree_the_composer_publishes_no_x_or_y() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(10, 10, 100, 100));
        composer.set_window_content(window, Some(content()));
        let node = node_of(&composer, window);
        composer.with_scene(|scene| scene.set_shell_places(true));
        composer.follow_placement();
        composer.take_tree_changes();
        // The window manager moves the window while the shell holds the tree: the shell's x and y
        // in the mirror must not be overwritten with the window manager's.
        composer.place_window(window, Rect::new(200, 120, 100, 100));
        let held = composer.take_tree_changes();
        assert_eq!(set_number(&held, node, "x"), None, "no x is published during the hold");
        assert_eq!(set_number(&held, node, "y"), None, "no y is published during the hold");
        // Once the shell lets go, the placed position is published, because the tree no longer
        // holds the value it last saw.
        composer.with_scene(|scene| scene.set_shell_places(false));
        composer.set_reduced_motion(true);
        composer.follow_placement();
        let released = composer.take_tree_changes();
        assert_eq!(set_number(&released, node, "x"), Some(200.0));
        assert_eq!(set_number(&released, node, "y"), Some(120.0));
    }

    #[test]
    fn a_shell_holding_the_tree_keeps_the_position_it_set() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(10, 10, 100, 100));
        composer.set_window_content(window, Some(content()));
        let node = node_of(&composer, window);
        composer.with_scene(|scene| scene.set_shell_places(true));
        composer.follow_placement();
        // The shell moves the node; the window manager's placement must not move it back.
        composer.with_scene(|scene| {
            let mut transform = *scene.transform(node).expect("the node has a transform");
            transform.base.offset.x = 333.0;
            transform.base.offset.y = 77.0;
            scene
                .set_transform(node, transform)
                .expect("the shell may set its node's transform");
        });
        composer.place_window(window, Rect::new(200, 120, 100, 100));
        assert_eq!(offset_of(&composer, node), (333.0, 77.0));
    }

    #[test]
    fn the_window_manager_places_the_window_again_when_the_shell_lets_go() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(10, 10, 100, 100));
        composer.set_window_content(window, Some(content()));
        let node = node_of(&composer, window);
        composer.with_scene(|scene| scene.set_shell_places(true));
        composer.follow_placement();
        composer.place_window(window, Rect::new(200, 120, 100, 100));
        composer.with_scene(|scene| scene.set_shell_places(false));
        // Reduced motion lands the placement at once, so the node's offset is the placed one.
        composer.set_reduced_motion(true);
        composer.follow_placement();
        assert_eq!(offset_of(&composer, node), (200.0, 120.0));
    }

    #[test]
    fn a_window_gets_a_window_node_with_its_title_and_app_id() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(10, 10, 100, 100));
        composer.set_window_text(window, "Alpha", "org.gnoblin.Alpha");
        composer.set_window_content(window, Some(content()));
        let changes = composer.take_tree_changes();
        let node = node_of(&composer, window);
        assert!(
            changes.iter().any(|change| matches!(
                change,
                TreeChange::Created { node: created, class, .. } if *created == node && class == "window"
            )),
            "the node is created as a window: {changes:?}"
        );
        assert!(changes.iter().any(|change| matches!(
            change,
            TreeChange::Set { node: set, key, value: TreeValue::Text(text) }
                if *set == node && key == "title" && text == "Alpha"
        )));
        assert!(changes.iter().any(|change| matches!(
            change,
            TreeChange::Set { node: set, key, value: TreeValue::Text(text) }
                if *set == node && key == "app_id" && text == "org.gnoblin.Alpha"
        )));
    }

    /// The node a `parent_window` set named last, over the changes taken.
    fn parent_published(changes: &[TreeChange], node: NodeId) -> Option<Option<NodeId>> {
        changes.iter().rev().find_map(|change| match change {
            TreeChange::Set {
                node: set,
                key,
                value: TreeValue::Node(target),
            } if *set == node && key == "parent_window" => Some(*target),
            _ => None,
        })
    }

    #[test]
    fn a_dialog_publishes_its_parents_node_once_both_have_nodes() {
        let (mut composer, _) = screen();
        let (parent, dialog) = (WindowId::new(1), WindowId::new(2));
        composer.insert_window(parent, Rect::new(10, 10, 100, 100));
        composer.insert_window(dialog, Rect::new(20, 20, 50, 50));
        // The relation is known before either window has content: nothing is published until the nodes exist.
        composer.set_window_parent(dialog, Some(parent));
        composer.set_window_content(dialog, Some(content()));
        let dialog_node = node_of(&composer, dialog);
        let changes = composer.take_tree_changes();
        assert_eq!(
            parent_published(&changes, dialog_node),
            None,
            "the parent has no node yet"
        );
        composer.set_window_content(parent, Some(content()));
        let parent_node = node_of(&composer, parent);
        let changes = composer.take_tree_changes();
        assert_eq!(
            parent_published(&changes, dialog_node),
            Some(Some(parent_node)),
            "the dialog hears of its parent when the parent's node appears: {changes:?}"
        );
        assert_eq!(
            composer.scene.reference(dialog_node, "parent_window"),
            Some(parent_node)
        );
    }

    #[test]
    fn a_dialog_with_a_node_and_a_parent_with_a_node_publishes_at_once_and_clears_with_the_parent() {
        let (mut composer, _) = screen();
        let (parent, dialog) = (WindowId::new(1), WindowId::new(2));
        for window in [parent, dialog] {
            composer.insert_window(window, Rect::new(10, 10, 100, 100));
            composer.set_window_content(window, Some(content()));
        }
        let (parent_node, dialog_node) = (node_of(&composer, parent), node_of(&composer, dialog));
        composer.take_tree_changes();
        composer.set_window_parent(dialog, Some(parent));
        let changes = composer.take_tree_changes();
        assert_eq!(parent_published(&changes, dialog_node), Some(Some(parent_node)));
        composer.set_window_parent(dialog, Some(parent));
        assert!(
            composer.take_tree_changes().is_empty(),
            "an unchanged relation publishes nothing"
        );
        composer.remove_window(parent);
        let changes = composer.take_tree_changes();
        assert_eq!(
            parent_published(&changes, dialog_node),
            Some(None),
            "a dialog whose parent closed has no parent: {changes:?}"
        );
        assert_eq!(composer.scene.reference(dialog_node, "parent_window"), None);
    }

    #[test]
    fn a_window_marked_from_a_launcher_publishes_the_mark_once_it_has_a_node() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(10, 10, 100, 100));
        composer.set_window_from_launcher(window);
        composer.set_window_content(window, Some(content()));
        let node = node_of(&composer, window);
        let changes = composer.take_tree_changes();
        let marks = |changes: &[TreeChange]| {
            changes
                .iter()
                .filter(|change| {
                    matches!(change, TreeChange::Set { node: set, key, value: TreeValue::Bool(true) }
                        if *set == node && key == "from_launcher")
                })
                .count()
        };
        assert_eq!(marks(&changes), 1, "{changes:?}");
        composer.set_window_from_launcher(window);
        assert_eq!(marks(&composer.take_tree_changes()), 0, "the mark is published once");
    }

    #[test]
    fn an_output_publishes_its_identity_mode_scale_and_primary_flag_only_when_they_change() {
        let mut composer = Composer::new();
        let output = OutputId::new(7);
        let info = ScreenInfo {
            make: "Gnoblin".to_owned(),
            model: "HEADLESS-1".to_owned(),
            mode: "800x600@60".to_owned(),
            scale: 1.0,
            primary: true,
        };
        composer.set_output_info(output, &info);
        assert!(
            composer.take_tree_changes().is_empty(),
            "an output with no screen node publishes nothing"
        );
        composer.set_output(output, Rect::new(0, 0, 800, 600));
        composer.take_tree_changes();
        composer.set_output_info(output, &info);
        let node = composer.screen_node(output).expect("the output has a screen node");
        assert_eq!(composer.scene.text(node, "make"), Some("Gnoblin"));
        assert_eq!(composer.scene.text(node, "model"), Some("HEADLESS-1"));
        assert_eq!(composer.scene.text(node, "mode"), Some("800x600@60"));
        assert_eq!(composer.scene.number(node, "output_scale"), Some(1.0));
        assert_eq!(composer.scene.value(node, "primary"), Some(&TreeValue::Bool(true)));
        composer.take_tree_changes();
        composer.set_output_info(output, &info);
        assert!(composer.take_tree_changes().is_empty());
        let demoted = ScreenInfo { primary: false, ..info };
        composer.set_output_info(output, &demoted);
        assert_eq!(
            composer.take_tree_changes(),
            vec![TreeChange::Set {
                node,
                key: "primary".to_owned(),
                value: TreeValue::Bool(false),
            }]
        );
    }

    #[test]
    fn a_title_change_is_published_only_when_it_changes() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(10, 10, 100, 100));
        composer.set_window_content(window, Some(content()));
        composer.set_window_text(window, "Alpha", "app");
        composer.take_tree_changes();
        composer.set_window_text(window, "Alpha", "app");
        assert!(
            composer.take_tree_changes().is_empty(),
            "the same text publishes nothing"
        );
        composer.set_window_text(window, "Beta", "app");
        let changes = composer.take_tree_changes();
        assert_eq!(changes.len(), 1, "only the title changed: {changes:?}");
    }

    #[test]
    fn a_window_state_is_published_only_when_it_changes() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(10, 10, 100, 100));
        composer.set_window_content(window, Some(content()));
        composer.set_window_text(window, "Alpha", "app");
        composer.take_tree_changes();
        composer.set_window_state(window, [true, false, false, false]);
        assert_eq!(
            composer.take_tree_changes().len(),
            4,
            "focused, fullscreen, maximised and minimised are each set"
        );
        composer.set_window_state(window, [true, false, false, false]);
        assert!(
            composer.take_tree_changes().is_empty(),
            "the same state publishes nothing"
        );
        composer.set_window_state(window, [false, true, false, false]);
        assert_eq!(
            composer.take_tree_changes().len(),
            4,
            "a changed state publishes all four"
        );
    }

    #[test]
    fn while_a_shell_holds_the_tree_only_focus_is_published_and_the_flags_are_read_from_the_node() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(10, 10, 100, 100));
        composer.set_window_content(window, Some(content()));
        composer.take_tree_changes();
        let node = composer.window_node(window).expect("the window has a node");
        composer.with_scene(|scene| scene.set_shell_places(true));
        assert_eq!(
            composer.shell_window_flags(window),
            Some([false; 3]),
            "nothing is set yet"
        );

        // The shell writes the three flags on the node itself.
        composer.with_scene(|scene| {
            for key in ["fullscreen", "maximised"] {
                scene
                    .set_value(node, key, TreeValue::Bool(true))
                    .expect("the node takes the value");
            }
        });
        assert_eq!(
            composer.shell_window_flags(window),
            Some([true, true, false]),
            "the flags are read from the node"
        );

        composer.set_window_state(window, [true, true, true, false]);
        let changes = composer.take_tree_changes();
        assert_eq!(changes.len(), 1, "only focus is published: {changes:?}");
        assert!(
            matches!(&changes[0], TreeChange::Set { key, value: TreeValue::Bool(true), .. } if key == "focused"),
            "{changes:?}"
        );

        composer.with_scene(|scene| scene.set_shell_places(false));
        assert_eq!(
            composer.shell_window_flags(window),
            None,
            "with no shell the node holds nothing"
        );
    }

    /// The number a `set` of `key` on `node` wrote in `changes`, if one did.
    fn set_number(changes: &[TreeChange], node: NodeId, key: &str) -> Option<f64> {
        changes.iter().find_map(|change| match change {
            TreeChange::Set {
                node: set,
                key: set_key,
                value: TreeValue::Double(value),
            } if *set == node && set_key == key => Some(*value),
            _ => None,
        })
    }

    #[test]
    fn a_window_is_announced_ready_once_when_it_first_has_a_node() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(10, 10, 100, 100));
        assert!(composer.take_tree_ready().is_empty(), "no content, no node, no ready");
        composer.set_window_content(window, Some(content()));
        let node = node_of(&composer, window);
        assert_eq!(composer.take_tree_ready(), vec![node]);
        composer.set_window_content(window, Some(content()));
        assert!(composer.take_tree_ready().is_empty(), "a commit announces nothing new");
    }

    #[test]
    fn a_window_node_publishes_its_position_and_size() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(10, 10, 100, 100));
        composer.set_window_content(window, Some(content()));
        let changes = composer.take_tree_changes();
        let node = node_of(&composer, window);
        assert_eq!(set_number(&changes, node, "width"), Some(40.0), "{changes:?}");
        assert_eq!(set_number(&changes, node, "height"), Some(50.0), "{changes:?}");
        assert_eq!(set_number(&changes, node, "x"), Some(10.0), "{changes:?}");
        assert_eq!(set_number(&changes, node, "y"), Some(10.0), "{changes:?}");
    }

    #[test]
    fn a_resize_and_a_move_publish_the_new_size_and_position() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(10, 10, 100, 100));
        composer.set_window_content(window, Some(content()));
        composer.take_tree_changes();
        let node = node_of(&composer, window);
        let bigger = Content {
            src: RectF::new(0.0, 0.0, 60.0, 70.0),
            size: Size::new(60, 70),
            transform: Transform::Normal,
        };
        composer.set_window_content(window, Some(bigger));
        let resized = composer.take_tree_changes();
        assert_eq!(set_number(&resized, node, "width"), Some(60.0), "{resized:?}");
        assert_eq!(set_number(&resized, node, "height"), Some(70.0), "{resized:?}");
        composer.place_window(window, Rect::new(200, 120, 60, 70));
        let moved = composer.take_tree_changes();
        assert_eq!(set_number(&moved, node, "x"), Some(200.0), "{moved:?}");
        assert_eq!(set_number(&moved, node, "y"), Some(120.0), "{moved:?}");
        composer.place_window(window, Rect::new(200, 120, 60, 70));
        assert!(
            composer.take_tree_changes().is_empty(),
            "the same place publishes nothing"
        );
    }

    #[test]
    fn removing_a_window_publishes_its_node_as_destroyed() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(10, 10, 100, 100));
        composer.set_window_content(window, Some(content()));
        composer.take_tree_changes();
        let node = node_of(&composer, window);
        composer.remove_window(window);
        let changes = composer.take_tree_changes();
        assert!(
            changes.iter().any(|change| matches!(
                change,
                TreeChange::Destroyed { node: destroyed, .. } if *destroyed == node
            )),
            "the destruction is published: {changes:?}"
        );
    }

    /// A shell node inserted after a window sits above it in the scene, so the ray picks the shell
    /// node, in its local space.
    #[test]
    fn a_shell_node_above_a_window_is_the_pick_in_its_local_space() {
        let (mut composer, _) = screen();
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(0, 0, 100, 100));
        composer.set_window_content(window, Some(content()));
        let shell = NodeId::new(1);
        composer.with_scene(|scene| {
            scene
                .insert_as(
                    gnoblin_render::Actor::Shell,
                    shell,
                    NodeSpec::new(Rect::new(0, 0, 50, 50)),
                )
                .expect("the shell may insert a node below the base");
        });
        let (node, local) = composer
            .shell_node_at(PointF::new(10.0, 20.0))
            .expect("the shell node is topmost at the point");
        assert_eq!(node, shell);
        assert_eq!(local, PointF::new(10.0, 20.0));
    }

    /// A window inserted after a shell node sits above it, so the ray picks no shell node and the
    /// pointer stays with the window.
    #[test]
    fn a_window_above_a_shell_node_leaves_no_pick() {
        let (mut composer, _) = screen();
        let shell = NodeId::new(1);
        composer.with_scene(|scene| {
            scene
                .insert_as(
                    gnoblin_render::Actor::Shell,
                    shell,
                    NodeSpec::new(Rect::new(0, 0, 50, 50)),
                )
                .expect("the shell may insert a node below the base");
        });
        let window = WindowId::new(1);
        composer.insert_window(window, Rect::new(0, 0, 100, 100));
        composer.set_window_content(window, Some(content()));
        assert!(composer.shell_node_at(PointF::new(10.0, 20.0)).is_none());
    }

    /// A node the compositor created that no batch has published yet is not picked (F29): the
    /// pick falls through to the shell node under it, and once the batch has taken the creation
    /// the node is picked.
    #[test]
    fn a_node_not_yet_in_a_batch_is_not_picked() {
        let (mut composer, _) = screen();
        let below = NodeId::new(1);
        let above = NodeId::new(2);
        composer.with_scene(|scene| {
            for node in [below, above] {
                scene
                    .insert_as(
                        gnoblin_render::Actor::Shell,
                        node,
                        NodeSpec::new(Rect::new(0, 0, 50, 50)),
                    )
                    .expect("the shell may insert a node below the base");
            }
        });
        composer.tree_changes.push(TreeChange::Created {
            node: above,
            class: "shell".to_owned(),
            index: 0,
        });
        let (node, _) = composer
            .shell_node_at(PointF::new(10.0, 20.0))
            .expect("the published node under the unpublished one is picked");
        assert_eq!(node, below);
        composer.take_tree_changes();
        let (node, _) = composer
            .shell_node_at(PointF::new(10.0, 20.0))
            .expect("the node is picked once its batch has taken it");
        assert_eq!(node, above);
    }

    #[test]
    fn a_new_output_is_a_screen_node_under_the_root_with_its_geometry() {
        let mut composer = Composer::new();
        let output = OutputId::new(7);
        composer.set_output(output, Rect::new(800, 0, 1920, 1080));
        let node = composer.screen_node(output).expect("the output has a screen node");
        assert_eq!(composer.scene.class(node), Some("screen"));
        assert_eq!(composer.scene.number(node, "x"), Some(800.0));
        assert_eq!(composer.scene.number(node, "width"), Some(1920.0));
        assert_eq!(composer.scene.number(node, "height"), Some(1080.0));
        assert_eq!(composer.scene.text(node, "name"), Some("output-7"));
        let changes = composer.take_tree_changes();
        assert!(
            changes.iter().any(|change| matches!(
                change,
                TreeChange::Created { node: created, class, .. } if *created == node && class == "screen"
            )),
            "the screen is announced: {changes:?}"
        );
    }

    #[test]
    fn a_moved_output_publishes_only_the_geometry_that_changed() {
        let mut composer = Composer::new();
        let output = OutputId::new(7);
        composer.set_output(output, Rect::new(0, 0, 800, 600));
        composer.take_tree_changes();
        composer.set_output(output, Rect::new(0, 0, 1024, 600));
        let node = composer.screen_node(output).expect("the output keeps its screen node");
        assert_eq!(composer.scene.number(node, "width"), Some(1024.0));
        let changes = composer.take_tree_changes();
        assert_eq!(
            changes,
            vec![TreeChange::Set {
                node,
                key: "width".to_owned(),
                value: TreeValue::Double(1024.0),
            }]
        );
    }

    #[test]
    fn a_removed_output_destroys_its_screen_node() {
        let mut composer = Composer::new();
        let output = OutputId::new(7);
        composer.set_output(output, Rect::new(0, 0, 800, 600));
        let node = composer.screen_node(output).expect("the output has a screen node");
        composer.take_tree_changes();
        composer.remove_output(output);
        assert_eq!(composer.screen_node(output), None);
        assert!(!composer.scene.contains(node));
        let changes = composer.take_tree_changes();
        assert!(
            changes
                .iter()
                .any(|change| matches!(change, TreeChange::Destroyed { node: gone, .. } if *gone == node)),
            "the screen's destruction is announced: {changes:?}"
        );
    }

    /// A paint list of one opaque rectangle that fills `width` by `height` from the origin.
    fn fill_list(width: f32, height: f32, red: u8) -> Vec<u8> {
        use gnoblin_paint::{PaintList, PaintOp, Rgba8};
        gnoblin_paint::encode(&PaintList {
            ops: vec![PaintOp::Rect {
                x: 0.0,
                y: 0.0,
                width,
                height,
                radius: 0.0,
                fill: Rgba8::new(red, 0, 0, 255),
                stroke: Rgba8::TRANSPARENT,
                stroke_width: 0.0,
            }],
        })
    }

    /// Adds a Canvas node at `rect` under `parent` or the root, holding `content`.
    fn add_canvas(
        composer: &mut Composer,
        canvas: NodeId,
        parent: Option<NodeId>,
        rect: Rect,
        content: Option<Vec<u8>>,
    ) {
        composer.with_scene(|scene| {
            let mut spec = NodeSpec::new(rect);
            spec.parent = parent;
            scene.insert(canvas, spec).expect("a canvas node");
            scene
                .set_class(canvas, Some("canvas".to_owned()))
                .expect("the canvas's class");
            if let Some(content) = content {
                scene
                    .set_value(canvas, "content", TreeValue::Bytes(content))
                    .expect("the canvas's content");
            }
        });
    }

    /// The snapshot layers the next frame for `output` carries: where each lands, how opaque, and its picture.
    fn canvas_layers(composer: &mut Composer, output: OutputId) -> Vec<(Rect, f32, Arc<Image>)> {
        composer.begin_frame();
        let mut frame = OutputFrame::new();
        composer.compose(output, &mut frame);
        composer.end_frame();
        frame
            .layers()
            .iter()
            .filter_map(|layer| match layer {
                Layer::Surface(surface) => match &surface.source {
                    SurfaceSource::Snapshot(image) => Some((surface.dst, surface.opacity, Arc::clone(image))),
                    SurfaceSource::Buffer(_) | SurfaceSource::Dmabuf(_) | SurfaceSource::View(_) => None,
                },
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_canvas_draws_its_painted_content_where_it_sits() {
        let (mut composer, output) = screen();
        let canvas = NodeId::new(1);
        add_canvas(
            &mut composer,
            canvas,
            None,
            Rect::new(5, 6, 20, 10),
            Some(fill_list(20.0, 10.0, 200)),
        );
        let layers = canvas_layers(&mut composer, output);
        assert_eq!(layers.len(), 1);
        let (dst, opacity, image) = &layers[0];
        assert_eq!(*dst, Rect::new(5, 6, 20, 10));
        assert_eq!(*opacity, 1.0);
        assert_eq!(image.size(), Size::new(20, 10));
        assert_eq!(image.pixel(Point::new(3, 3)), Some([200, 0, 0, 255]));
    }

    #[test]
    fn a_canvas_without_content_or_not_drawn_has_no_layer() {
        let (mut composer, output) = screen();
        add_canvas(&mut composer, NodeId::new(1), None, Rect::new(0, 0, 10, 10), None);
        add_canvas(
            &mut composer,
            NodeId::new(2),
            None,
            Rect::new(0, 0, 10, 10),
            Some(fill_list(10.0, 10.0, 9)),
        );
        composer.with_scene(|scene| scene.set_visible(NodeId::new(2), false).expect("the canvas hides"));
        assert!(canvas_layers(&mut composer, output).is_empty());
        composer.with_scene(|scene| scene.set_visible(NodeId::new(2), true).expect("the canvas shows"));
        assert_eq!(canvas_layers(&mut composer, output).len(), 1);
    }

    #[test]
    fn a_canvas_is_painted_again_only_when_its_content_or_size_changes() {
        let (mut composer, output) = screen();
        let canvas = NodeId::new(1);
        add_canvas(
            &mut composer,
            canvas,
            None,
            Rect::new(0, 0, 8, 8),
            Some(fill_list(8.0, 8.0, 10)),
        );
        composer.begin_frame();
        let first = composer.canvas_picture(canvas).expect("painted");
        composer.end_frame();
        let _ = damage_of(&mut composer, output);
        assert!(
            Arc::ptr_eq(&first, &composer.canvas_picture(canvas).expect("painted")),
            "an unchanged canvas keeps its picture"
        );
        composer.with_scene(|scene| {
            scene
                .set_value(canvas, "content", TreeValue::Bytes(fill_list(8.0, 8.0, 20)))
                .expect("new content");
        });
        composer.begin_frame();
        let second = composer.canvas_picture(canvas).expect("painted again");
        composer.end_frame();
        assert!(!Arc::ptr_eq(&first, &second), "new content paints a new picture");
        assert_eq!(second.pixel(Point::new(1, 1)), Some([20, 0, 0, 255]));
        composer.with_scene(|scene| {
            scene
                .set_bounds(canvas, Rect::new(0, 0, 12, 9))
                .expect("the canvas resizes")
        });
        composer.begin_frame();
        let third = composer.canvas_picture(canvas).expect("painted again");
        composer.end_frame();
        assert_eq!(third.size(), Size::new(12, 9), "a resize paints at the new size");
    }

    #[test]
    fn writing_a_canvas_content_damages_the_canvas_and_nothing_else() {
        let (mut composer, output) = screen();
        let canvas = NodeId::new(1);
        add_canvas(
            &mut composer,
            canvas,
            None,
            Rect::new(30, 40, 20, 10),
            Some(fill_list(20.0, 10.0, 1)),
        );
        assert!(
            damage_of(&mut composer, output).is_some(),
            "a new canvas damages its pixels"
        );
        assert!(
            damage_of(&mut composer, output).is_none(),
            "an unchanged canvas draws nothing new"
        );
        composer.with_scene(|scene| {
            scene
                .set_value(canvas, "content", TreeValue::Bytes(fill_list(20.0, 10.0, 2)))
                .expect("new content");
        });
        let damage = damage_of(&mut composer, output).expect("a repaint is damage");
        assert_eq!(damage, vec![Rect::new(30, 40, 20, 10)]);
    }

    #[test]
    fn canvases_draw_in_tree_order_children_over_their_parent() {
        let (mut composer, output) = screen();
        let (parent, child, sibling) = (NodeId::new(1), NodeId::new(2), NodeId::new(3));
        add_canvas(
            &mut composer,
            parent,
            None,
            Rect::new(0, 0, 10, 10),
            Some(fill_list(10.0, 10.0, 1)),
        );
        add_canvas(
            &mut composer,
            sibling,
            None,
            Rect::new(0, 0, 30, 30),
            Some(fill_list(30.0, 30.0, 3)),
        );
        add_canvas(
            &mut composer,
            child,
            Some(parent),
            Rect::new(2, 2, 4, 4),
            Some(fill_list(4.0, 4.0, 2)),
        );
        let drawn: Vec<(Rect, u8)> = canvas_layers(&mut composer, output)
            .iter()
            .map(|(dst, _, image)| (*dst, image.pixel(Point::new(0, 0)).expect("a pixel")[0]))
            .collect();
        assert_eq!(
            drawn,
            vec![
                (Rect::new(0, 0, 10, 10), 1),
                (Rect::new(2, 2, 4, 4), 2),
                (Rect::new(0, 0, 30, 30), 3)
            ],
            "a parent, then its child placed in the parent's space, then the next sibling"
        );
    }

    #[test]
    fn a_canvas_composes_the_opacity_of_its_parents() {
        let (mut composer, output) = screen();
        let (parent, child) = (NodeId::new(1), NodeId::new(2));
        add_canvas(&mut composer, parent, None, Rect::new(0, 0, 10, 10), None);
        add_canvas(
            &mut composer,
            child,
            Some(parent),
            Rect::new(0, 0, 10, 10),
            Some(fill_list(10.0, 10.0, 5)),
        );
        composer.with_scene(|scene| {
            scene
                .set_property(parent, VisualProperty::Opacity, PropertyValue::Number(0.5))
                .expect("the parent fades");
            scene
                .set_property(child, VisualProperty::Opacity, PropertyValue::Number(0.5))
                .expect("the child fades");
        });
        let layers = canvas_layers(&mut composer, output);
        assert_eq!(layers.len(), 1);
        assert!(
            (layers[0].1 - 0.25).abs() < 1e-6,
            "opacity multiplies down the tree: {}",
            layers[0].1
        );
    }

    #[test]
    fn a_view_of_a_canvas_draws_the_canvas_pixels() {
        let (mut composer, output) = screen();
        let canvas = NodeId::new(2);
        add_canvas(
            &mut composer,
            canvas,
            None,
            Rect::new(0, 0, 40, 30),
            Some(fill_list(40.0, 30.0, 77)),
        );
        add_view(&mut composer, NodeId::new(1), None, Some(canvas));
        let _ = view_layers(&mut composer, output);
        let pictures = composer.view_pictures();
        assert_eq!(pictures.len(), 1);
        assert_eq!(pictures[0].1.pixel(Point::new(5, 5)), Some([77, 0, 0, 255]));
    }

    /// A camera node (3) that the View (1) of a 40 by 30 canvas draws through: `projection`, positioned at
    /// (20, 15, 100) in front of the canvas, with `numbers` set on top of the schema's defaults.
    fn view_through_camera(projection: &str, numbers: &[(&str, f64)]) -> Composer {
        let (mut composer, output) = screen();
        let canvas = NodeId::new(2);
        add_canvas(
            &mut composer,
            canvas,
            None,
            Rect::new(0, 0, 40, 30),
            Some(fill_list(40.0, 30.0, 77)),
        );
        add_view(&mut composer, NodeId::new(1), None, Some(canvas));
        let camera = NodeId::new(3);
        composer.with_scene(|scene| {
            scene
                .insert(camera, NodeSpec::new(Rect::new(0, 0, 0, 0)))
                .expect("a camera node");
            scene
                .set_class(camera, Some("camera".to_owned()))
                .expect("the camera's class");
            scene
                .set_text(camera, "projection", projection.to_owned())
                .expect("the camera's projection");
            for (key, value) in numbers {
                scene.set_number(camera, key, *value).expect("a camera number");
            }
            let mut transform = *scene.transform(camera).expect("the camera's transform");
            transform.base.offset = Vec3 {
                x: 20.0,
                y: 15.0,
                z: 100.0,
            };
            scene.set_transform(camera, transform).expect("the camera's pose");
            scene
                .set_camera(NodeId::new(1), Some(camera))
                .expect("the view's camera");
        });
        let _ = view_layers(&mut composer, output);
        composer
    }

    /// The pixel of the picture a View draws at `(x, y)`, or `None` when it has none.
    fn view_pixel(composer: &Composer, view: NodeId, x: i32, y: i32) -> Option<[u8; 4]> {
        let (_, picture) = composer.view_pictures().into_iter().find(|(node, _)| *node == view)?;
        picture.pixel(Point::new(x, y))
    }

    #[test]
    fn a_view_through_a_stereo_camera_draws_each_eye_in_its_half_from_its_side() {
        // The eyes are 63 units apart (0.063 m at 1 mm a unit), 100 units from the canvas. The left eye, at x = -11.5,
        // sees the canvas at the right of its half; the right eye, at x = 51.5, sees it at the left of its half.
        let composer = view_through_camera("stereo", &[]);
        let view = NodeId::new(1);
        assert_eq!(
            view_pixel(&composer, view, 19, 15),
            Some(CANVAS_PIXEL),
            "left eye, right edge of its half"
        );
        assert_eq!(
            view_pixel(&composer, view, 0, 15),
            Some(BACKGROUND_PIXEL),
            "left eye, left edge of its half"
        );
        assert_eq!(
            view_pixel(&composer, view, 20, 15),
            Some(CANVAS_PIXEL),
            "right eye, left edge of its half"
        );
        assert_eq!(
            view_pixel(&composer, view, 39, 15),
            Some(BACKGROUND_PIXEL),
            "right eye, right edge of its half"
        );
    }

    #[test]
    fn the_eye_separation_follows_ipd_and_metres() {
        // No separation: both eyes are at x = 20, and see the canvas centred in their half.
        let composer = view_through_camera("stereo", &[("ipd", 0.0)]);
        let view = NodeId::new(1);
        assert_eq!(view_pixel(&composer, view, 10, 15), Some(CANVAS_PIXEL));
        assert_eq!(view_pixel(&composer, view, 30, 15), Some(CANVAS_PIXEL));
        assert_eq!(view_pixel(&composer, view, 0, 15), view_pixel(&composer, view, 20, 15));
        // The same 0.063 m in metres of 0.002 is 31.5 units: half the displacement, so the canvas fills more of
        // each half than at 63 units.
        let wide = view_through_camera("stereo", &[]);
        let narrow = view_through_camera("stereo", &[("metres", 0.002)]);
        let red = |composer: &Composer| {
            (0..20)
                .filter(|x| view_pixel(composer, view, *x, 15) == Some(CANVAS_PIXEL))
                .count()
        };
        assert!(red(&narrow) > red(&wide), "{} vs {}", red(&narrow), red(&wide));
    }

    #[test]
    fn a_stereo_camera_with_an_unusable_frustum_or_scale_draws_a_blank_picture() {
        let view = NodeId::new(1);
        for numbers in [
            vec![
                ("fov_left", 30.0),
                ("fov_right", -30.0),
                ("fov_up", 30.0),
                ("fov_down", -30.0),
            ],
            vec![("fov", 0.0)],
            vec![("metres", 0.0)],
            vec![("ipd", -1.0)],
        ] {
            let composer = view_through_camera("stereo", &numbers);
            for x in [0, 10, 19, 20, 30, 39] {
                assert_eq!(
                    view_pixel(&composer, view, x, 15),
                    Some(BACKGROUND_PIXEL),
                    "{numbers:?} at {x}"
                );
            }
        }
    }

    #[test]
    fn an_off_axis_perspective_camera_shifts_the_view_and_a_bad_frustum_blanks_it() {
        let view = NodeId::new(1);
        // A frustum that reaches 50 degrees left and 5 right puts the canvas (straight ahead) at the picture's right.
        let shifted = view_through_camera(
            "perspective",
            &[
                ("fov_left", -50.0),
                ("fov_right", 5.0),
                ("fov_up", 30.0),
                ("fov_down", -30.0),
            ],
        );
        assert_eq!(view_pixel(&shifted, view, 39, 15), Some(CANVAS_PIXEL));
        assert_eq!(view_pixel(&shifted, view, 0, 15), Some(BACKGROUND_PIXEL));
        let blank = view_through_camera(
            "perspective",
            &[
                ("fov_left", 5.0),
                ("fov_right", -50.0),
                ("fov_up", 30.0),
                ("fov_down", -30.0),
            ],
        );
        assert_eq!(view_pixel(&blank, view, 39, 15), Some(BACKGROUND_PIXEL));
    }

    /// What a canvas filled with red 77 looks like under a View, and what the neutral grey of a secure node is.
    const CANVAS_PIXEL: [u8; 4] = [77, 0, 0, 255];
    const MASK_PIXEL: [u8; 4] = [128, 128, 128, 255];
    const BACKGROUND_PIXEL: [u8; 4] = [0, 0, 0, 255];

    /// A View (node 1) of a canvas (node 2) filled with red 77, drawn once, as `secure` or not.
    fn view_of_a_canvas(secure: bool) -> (Composer, OutputId) {
        let (mut composer, output) = screen();
        let canvas = NodeId::new(2);
        add_canvas(
            &mut composer,
            canvas,
            None,
            Rect::new(0, 0, 40, 30),
            Some(fill_list(40.0, 30.0, 77)),
        );
        add_view(&mut composer, NodeId::new(1), None, Some(canvas));
        if secure {
            mark_secure(&mut composer, canvas, true);
        }
        (composer, output)
    }

    /// Writes the `secure` value a shell writes on a node.
    fn mark_secure(composer: &mut Composer, node: NodeId, secure: bool) {
        composer.with_scene(|scene| {
            scene
                .set_value(node, "secure", TreeValue::Bool(secure))
                .expect("the node's secure value")
        });
    }

    /// The pixel at (5, 5) of the picture capture reads for a View, or `None` when it has none.
    fn captured_pixel(composer: &Composer, view: NodeId) -> Option<[u8; 4]> {
        let (_, picture) = composer.view_pictures().into_iter().find(|(node, _)| *node == view)?;
        picture.pixel(Point::new(5, 5))
    }

    /// What the next frame for `output` draws for each View, at the rectangle `add_view` gives it: whether the layer
    /// is secure, and its pixel at (5, 5).
    fn drawn_views(composer: &mut Composer, output: OutputId) -> Vec<(bool, Option<[u8; 4]>)> {
        composer.begin_frame();
        let mut frame = OutputFrame::new();
        composer.compose(output, &mut frame);
        composer.end_frame();
        frame
            .layers()
            .iter()
            .filter_map(|layer| match layer {
                Layer::Surface(Surface {
                    source: SurfaceSource::Snapshot(image),
                    secure,
                    dst,
                    ..
                }) if *dst == Rect::new(10, 20, 40, 30) => Some((*secure, image.pixel(Point::new(5, 5)))),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_secure_node_in_a_views_source_is_masked_in_its_capture_picture_and_not_on_the_display() {
        let (mut composer, output) = view_of_a_canvas(true);
        let drawn = drawn_views(&mut composer, output);
        assert_eq!(
            drawn,
            vec![(true, Some(CANVAS_PIXEL))],
            "the display shows the canvas, and its layer is secure so an output capture replaces the whole View"
        );
        assert_eq!(
            captured_pixel(&composer, NodeId::new(1)),
            Some(MASK_PIXEL),
            "a capture of the View reads the mask in the secure node's place"
        );
    }

    #[test]
    fn a_view_with_no_secure_node_is_captured_as_drawn_and_is_not_a_secure_layer() {
        let (mut composer, output) = view_of_a_canvas(false);
        assert_eq!(drawn_views(&mut composer, output), vec![(false, Some(CANVAS_PIXEL))]);
        assert_eq!(captured_pixel(&composer, NodeId::new(1)), Some(CANVAS_PIXEL));
    }

    #[test]
    fn omit_leaves_a_secure_node_out_of_a_views_capture_picture() {
        let (mut composer, output) = view_of_a_canvas(true);
        composer.set_secure_treatment(SecureTreatment::Omit);
        let _ = drawn_views(&mut composer, output);
        assert_eq!(
            captured_pixel(&composer, NodeId::new(1)),
            Some(BACKGROUND_PIXEL),
            "nothing is drawn where the secure node is, as an output capture omits a secure layer"
        );
    }

    #[test]
    fn the_children_of_a_secure_node_are_masked_with_it() {
        let (mut composer, output) = screen();
        let (parent, canvas) = (NodeId::new(3), NodeId::new(2));
        add_source(&mut composer, parent, Rect::new(0, 0, 40, 30));
        add_canvas(
            &mut composer,
            canvas,
            Some(parent),
            Rect::new(0, 0, 40, 30),
            Some(fill_list(40.0, 30.0, 77)),
        );
        add_view(&mut composer, NodeId::new(1), None, Some(parent));
        mark_secure(&mut composer, parent, true);
        let _ = drawn_views(&mut composer, output);
        assert_eq!(captured_pixel(&composer, NodeId::new(1)), Some(MASK_PIXEL));
    }

    #[test]
    fn a_node_that_becomes_secure_is_masked_in_the_next_picture_and_one_that_stops_is_shown() {
        let (mut composer, output) = view_of_a_canvas(false);
        let _ = drawn_views(&mut composer, output);
        assert_eq!(captured_pixel(&composer, NodeId::new(1)), Some(CANVAS_PIXEL));
        mark_secure(&mut composer, NodeId::new(2), true);
        let _ = drawn_views(&mut composer, output);
        assert_eq!(captured_pixel(&composer, NodeId::new(1)), Some(MASK_PIXEL));
        mark_secure(&mut composer, NodeId::new(2), false);
        let _ = drawn_views(&mut composer, output);
        assert_eq!(captured_pixel(&composer, NodeId::new(1)), Some(CANVAS_PIXEL));
    }

    #[test]
    fn a_secure_mark_is_drawn_at_once_though_the_view_is_paced() {
        let (mut composer, output) = paced_view(1);
        assert_eq!(view_red(&composer), Some(1), "the first picture is drawn at once");
        // A second of the slot is far from over, so a plain change would wait.
        composer.advance_motion(20);
        mark_secure(&mut composer, PACED_CANVAS, true);
        let _ = damage_of(&mut composer, output);
        assert_eq!(
            view_red(&composer),
            Some(128),
            "the capture picture holds the mask, not the pixels drawn before the mark"
        );
    }

    #[test]
    fn a_view_of_a_view_never_carries_a_secure_nodes_pixels() {
        let (mut composer, output) = screen();
        let (outer, inner, holder, canvas) = (NodeId::new(1), NodeId::new(3), NodeId::new(4), NodeId::new(2));
        add_canvas(
            &mut composer,
            canvas,
            None,
            Rect::new(0, 0, 40, 30),
            Some(fill_list(40.0, 30.0, 77)),
        );
        mark_secure(&mut composer, canvas, true);
        add_source(&mut composer, holder, Rect::new(0, 0, 40, 30));
        add_view(&mut composer, inner, Some(holder), Some(canvas));
        add_view(&mut composer, outer, None, Some(holder));
        let _ = drawn_views(&mut composer, output);
        assert_eq!(
            captured_pixel(&composer, inner),
            Some(MASK_PIXEL),
            "the inner View's own capture is masked"
        );
        let outer_pixel = captured_pixel(&composer, outer).expect("the outer View has a picture");
        assert_ne!(
            outer_pixel, CANVAS_PIXEL,
            "the outer View holds none of the secure node's pixels"
        );
    }

    const PACED_CANVAS: NodeId = NodeId::new(2);
    const PACED_VIEW: NodeId = NodeId::new(1);

    /// A View of a canvas filled with red 1, drawn once, with its `fps` set to `fps`.
    fn paced_view(fps: u64) -> (Composer, OutputId) {
        let (mut composer, output) = screen();
        add_canvas(
            &mut composer,
            PACED_CANVAS,
            None,
            Rect::new(0, 0, 40, 30),
            Some(fill_list(40.0, 30.0, 1)),
        );
        add_view(&mut composer, PACED_VIEW, None, Some(PACED_CANVAS));
        composer.with_scene(|scene| {
            scene
                .set_value(PACED_VIEW, "fps", TreeValue::Uint(fps))
                .expect("the view's fps")
        });
        let _ = damage_of(&mut composer, output);
        (composer, output)
    }

    /// Repaints the source canvas red `red`, as a live source changing between two frames.
    fn repaint(composer: &mut Composer, red: u8) {
        composer.with_scene(|scene| {
            scene
                .set_value(PACED_CANVAS, "content", TreeValue::Bytes(fill_list(40.0, 30.0, red)))
                .expect("the canvas's content")
        });
    }

    /// The red of the View's picture, or `None` when it has none.
    fn view_red(composer: &Composer) -> Option<u8> {
        let (_, picture) = composer
            .view_pictures()
            .into_iter()
            .find(|(view, _)| *view == PACED_VIEW)?;
        picture.pixel(Point::new(5, 5)).map(|pixel| pixel[0])
    }

    #[test]
    fn a_view_with_fps_redraws_at_most_once_per_slot_though_its_source_changes_every_frame() {
        let (mut composer, output) = paced_view(10);
        assert_eq!(view_red(&composer), Some(1), "the first picture is drawn at once");
        let mut drawn = Vec::new();
        for step in 1..=25u8 {
            composer.advance_motion(20);
            repaint(&mut composer, step + 1);
            let before = view_red(&composer);
            let _ = damage_of(&mut composer, output);
            if view_red(&composer) != before {
                drawn.push((step, view_red(&composer)));
            }
        }
        // Ten frames a second is a slot every 100 ms: steps 5, 10, 15, 20 and 25 of 20 ms, each drawing the
        // source as it is then.
        assert_eq!(
            drawn,
            vec![
                (5, Some(6)),
                (10, Some(11)),
                (15, Some(16)),
                (20, Some(21)),
                (25, Some(26))
            ]
        );
    }

    #[test]
    fn damage_in_a_slot_is_coalesced_and_the_last_state_is_drawn_at_the_next() {
        let (mut composer, output) = paced_view(10);
        composer.advance_motion(10);
        repaint(&mut composer, 10);
        let _ = damage_of(&mut composer, output);
        composer.advance_motion(40);
        repaint(&mut composer, 20);
        let _ = damage_of(&mut composer, output);
        assert_eq!(view_red(&composer), Some(1), "nothing is redrawn inside the slot");
        composer.advance_motion(49);
        let _ = damage_of(&mut composer, output);
        assert_eq!(view_red(&composer), Some(1), "99 ms is still inside the slot");
        // No write at the slot itself: the pending redraw still happens, with the last state.
        composer.advance_motion(1);
        assert!(
            damage_of(&mut composer, output).is_some(),
            "the redraw damages the View's pixels"
        );
        assert_eq!(view_red(&composer), Some(20), "the last state is what the slot draws");
        // Nothing is pending afterwards.
        composer.advance_motion(500);
        assert!(damage_of(&mut composer, output).is_none(), "the slot drew everything");
    }

    #[test]
    fn a_view_with_fps_zero_redraws_on_every_change() {
        let (mut composer, output) = paced_view(0);
        for red in 2..=6u8 {
            repaint(&mut composer, red);
            let _ = damage_of(&mut composer, output);
            assert_eq!(
                view_red(&composer),
                Some(red),
                "no time has passed and the View followed"
            );
        }
        let _ = damage_of(&mut composer, output);
        composer.advance_motion(1000);
        assert!(
            damage_of(&mut composer, output).is_none(),
            "an unchanged View draws nothing"
        );
    }

    #[test]
    fn a_view_whose_source_is_idle_is_not_redrawn_at_its_slot() {
        let (mut composer, output) = paced_view(10);
        for _ in 0..5 {
            composer.advance_motion(100);
            assert!(
                damage_of(&mut composer, output).is_none(),
                "fps caps redraws, it does not force them"
            );
        }
    }

    #[test]
    fn lowering_fps_to_zero_draws_the_pending_state_at_once() {
        let (mut composer, output) = paced_view(1);
        composer.advance_motion(10);
        repaint(&mut composer, 9);
        let _ = damage_of(&mut composer, output);
        assert_eq!(view_red(&composer), Some(1), "one frame a second holds the picture");
        composer.with_scene(|scene| {
            scene
                .set_value(PACED_VIEW, "fps", TreeValue::Uint(0))
                .expect("the view's fps")
        });
        let _ = damage_of(&mut composer, output);
        assert_eq!(view_red(&composer), Some(9), "on demand draws what was waiting");
    }

    #[test]
    fn a_view_that_resizes_is_drawn_at_once_whatever_its_fps() {
        let (mut composer, output) = paced_view(1);
        composer.with_scene(|scene| {
            scene
                .set_bounds(PACED_VIEW, Rect::new(10, 20, 20, 10))
                .expect("the view resizes")
        });
        let _ = damage_of(&mut composer, output);
        let (_, picture) = composer
            .view_pictures()
            .into_iter()
            .find(|(view, _)| *view == PACED_VIEW)
            .expect("the view's picture");
        assert_eq!(
            picture.size(),
            Size::new(20, 10),
            "a picture of the wrong size is never kept"
        );
    }

    /// An `fps` too large to mean anything is paced to the one-millisecond clock the loop runs on: the slot is never
    /// zero (no division by it, no redraw inside one tick) and never a panic, including at the largest value a `uint`
    /// holds.
    #[test]
    fn an_enormous_fps_is_a_one_millisecond_slot_and_neither_panics_nor_draws_twice_in_one() {
        for fps in [1000, 1_000_000, u64::from(u32::MAX) + 1, u64::MAX] {
            let (mut composer, output) = paced_view(fps);
            repaint(&mut composer, 5);
            let _ = damage_of(&mut composer, output);
            assert_eq!(
                view_red(&composer),
                Some(1),
                "fps {fps}: inside the millisecond the picture holds"
            );
            composer.advance_motion(1);
            let _ = damage_of(&mut composer, output);
            assert_eq!(
                view_red(&composer),
                Some(5),
                "fps {fps}: the next millisecond draws the change"
            );
            repaint(&mut composer, 6);
            let _ = damage_of(&mut composer, output);
            assert_eq!(
                view_red(&composer),
                Some(5),
                "fps {fps}: one draw a millisecond, not one a change"
            );
        }
    }

    /// A node's `effects` list is the effect chain of the layer that draws it, for a canvas and for a View, and
    /// clearing the list clears the chain.
    #[test]
    fn a_nodes_effects_list_is_the_effect_chain_of_its_layer() {
        let (mut composer, output) = screen();
        let canvas = NodeId::new(2);
        add_canvas(
            &mut composer,
            canvas,
            None,
            Rect::new(0, 0, 40, 30),
            Some(fill_list(40.0, 30.0, 77)),
        );
        add_view(&mut composer, NodeId::new(1), None, Some(canvas));
        let chains = |composer: &mut Composer| -> Vec<Effects> {
            composer.begin_frame();
            let mut frame = OutputFrame::new();
            composer.compose(output, &mut frame);
            composer.end_frame();
            frame
                .layers()
                .iter()
                .filter_map(|layer| match layer {
                    Layer::Surface(surface) => Some(surface.effects.clone()),
                    _ => None,
                })
                .collect()
        };
        assert!(
            chains(&mut composer).iter().all(Effects::is_empty),
            "no effects list, no passes"
        );
        for node in [canvas, NodeId::new(1)] {
            composer.with_scene(|scene| scene.set_effects(node, blur_and_shadow()).expect("the effects"));
        }
        let drawn = chains(&mut composer);
        assert_eq!(drawn.len(), 2, "the canvas and the View each draw one layer");
        assert!(
            drawn.iter().all(|chain| *chain == blur_and_shadow()),
            "each layer carries its node's list, in order: {drawn:?}"
        );
        for node in [canvas, NodeId::new(1)] {
            composer.with_scene(|scene| scene.set_effects(node, Effects::new()).expect("the effects"));
        }
        assert!(
            chains(&mut composer).iter().all(Effects::is_empty),
            "an emptied list draws no passes"
        );
    }

    #[test]
    fn a_canvas_with_content_that_does_not_decode_draws_nothing() {
        let (mut composer, output) = screen();
        add_canvas(
            &mut composer,
            NodeId::new(1),
            None,
            Rect::new(0, 0, 10, 10),
            Some(b"junk".to_vec()),
        );
        assert!(canvas_layers(&mut composer, output).is_empty());
    }

    #[test]
    fn a_canvas_too_large_to_paint_draws_nothing() {
        let (mut composer, output) = screen();
        add_canvas(
            &mut composer,
            NodeId::new(1),
            None,
            Rect::new(0, 0, 9000, 9000),
            Some(fill_list(10.0, 10.0, 1)),
        );
        assert!(canvas_layers(&mut composer, output).is_empty());
    }

    #[test]
    fn a_zero_size_canvas_draws_nothing_and_one_pixel_is_enough() {
        let (mut composer, output) = screen();
        add_canvas(
            &mut composer,
            NodeId::new(1),
            None,
            Rect::new(4, 4, 0, 0),
            Some(fill_list(10.0, 10.0, 1)),
        );
        add_canvas(
            &mut composer,
            NodeId::new(2),
            None,
            Rect::new(4, 4, 10, 0),
            Some(fill_list(10.0, 10.0, 1)),
        );
        assert!(canvas_layers(&mut composer, output).is_empty(), "no area, no picture");
        composer.with_scene(|scene| {
            scene
                .set_bounds(NodeId::new(2), Rect::new(4, 4, 1, 1))
                .expect("the canvas resizes")
        });
        let layers = canvas_layers(&mut composer, output);
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].2.size(), Size::new(1, 1));
    }

    #[test]
    fn a_canvas_is_painted_at_the_scale_of_the_output_it_is_on_and_drawn_over_its_logical_rectangle() {
        let (mut composer, output) = screen();
        composer.set_output_scale(output, Scale::integral(2));
        add_canvas(
            &mut composer,
            NodeId::new(1),
            None,
            Rect::new(5, 6, 20, 10),
            Some(fill_list(20.0, 10.0, 200)),
        );
        let layers = canvas_layers(&mut composer, output);
        assert_eq!(layers.len(), 1);
        let (dst, _, image) = &layers[0];
        assert_eq!(*dst, Rect::new(5, 6, 20, 10), "the rectangle is logical");
        assert_eq!(
            image.size(),
            Size::new(40, 20),
            "the picture has two pixels per logical pixel"
        );
        assert_eq!(
            image.pixel(Point::new(39, 19)),
            Some([200, 0, 0, 255]),
            "the fill covers it all"
        );

        composer.set_output_scale(output, Scale::new(3, 2));
        let layers = canvas_layers(&mut composer, output);
        assert_eq!(
            layers[0].2.size(),
            Size::new(30, 15),
            "a fractional scale repaints at its own size"
        );

        composer.set_output_scale(output, Scale::IDENTITY);
        let layers = canvas_layers(&mut composer, output);
        assert_eq!(layers[0].2.size(), Size::new(20, 10), "and one scale repaints at one");
    }

    #[test]
    fn a_canvas_whose_scaled_picture_is_too_large_is_painted_at_one() {
        let (mut composer, output) = screen();
        composer.set_output_scale(output, Scale::integral(2));
        add_canvas(
            &mut composer,
            NodeId::new(1),
            None,
            Rect::new(0, 0, 3000, 3000),
            Some(fill_list(3000.0, 3000.0, 9)),
        );
        let layers = canvas_layers(&mut composer, output);
        assert_eq!(layers.len(), 1, "nine million pixels still draw");
        assert_eq!(layers[0].2.size(), Size::new(3000, 3000));
    }

    /// Whether each canvas layer the next frame carries is secure, in paint order.
    fn canvas_layers_secure(composer: &mut Composer, output: OutputId) -> Vec<bool> {
        composer.begin_frame();
        let mut frame = OutputFrame::new();
        composer.compose(output, &mut frame);
        composer.end_frame();
        frame
            .layers()
            .iter()
            .filter_map(|layer| match layer {
                Layer::Surface(Surface {
                    source: SurfaceSource::Snapshot(_),
                    secure,
                    ..
                }) => Some(*secure),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_secure_canvas_and_its_children_are_secure_layers_and_a_mark_redraws_them() {
        let (mut composer, output) = screen();
        let parent = NodeId::new(1);
        let child = NodeId::new(2);
        add_canvas(
            &mut composer,
            parent,
            None,
            Rect::new(0, 0, 30, 30),
            Some(fill_list(30.0, 30.0, 1)),
        );
        add_canvas(
            &mut composer,
            child,
            Some(parent),
            Rect::new(5, 5, 10, 10),
            Some(fill_list(10.0, 10.0, 2)),
        );
        assert_eq!(canvas_layers_secure(&mut composer, output), vec![false, false]);
        assert_eq!(damage_of(&mut composer, output), None, "nothing changed");

        mark_secure(&mut composer, parent, true);
        assert_eq!(
            canvas_layers_secure(&mut composer, output),
            vec![true, true],
            "a secure canvas's children are secure with it"
        );
        mark_secure(&mut composer, parent, false);
        mark_secure(&mut composer, child, true);
        assert_eq!(
            canvas_layers_secure(&mut composer, output),
            vec![false, true],
            "a secure child leaves its parent as it was"
        );
        mark_secure(&mut composer, child, false);
        composer.begin_frame();
        let mut frame = OutputFrame::new();
        assert!(
            composer.compose(output, &mut frame),
            "clearing the mark redraws the frame a capture reads"
        );
        composer.end_frame();
        let secure: Vec<bool> = frame
            .layers()
            .iter()
            .filter_map(|layer| match layer {
                Layer::Surface(surface) => Some(surface.secure),
                _ => None,
            })
            .collect();
        assert_eq!(secure, vec![false, false]);
    }

    /// The `skipped` texts the composer queued for `node`, last first, or `None` when it queued none.
    fn queued_skipped(composer: &mut Composer, node: NodeId) -> Option<Vec<String>> {
        composer
            .take_tree_changes()
            .into_iter()
            .rev()
            .find_map(|change| match change {
                TreeChange::Set {
                    node: changed,
                    key,
                    value: TreeValue::ListText(texts),
                } if changed == node && key == "skipped" => Some(texts),
                _ => None,
            })
    }

    #[test]
    fn an_effect_on_a_node_placed_in_3d_is_reported_skipped_and_cleared_when_it_is_flat() {
        use gnoblin_render::{BlurParams, Shadow};
        let (mut composer, _output) = screen();
        let node = NodeId::new(0x4000);
        let mut chain = Effects::new();
        chain
            .push(Pass::Blur(BlurParams {
                radius: 4.0,
                passes: 2,
                ..BlurParams::default()
            }))
            .expect("a blur");
        chain
            .push(Pass::Shadow(Shadow {
                offset: Vec3::new(0.0, 4.0, 0.0),
                blur: 12.0,
                spread: 0.0,
                opacity: 1.0,
                color: Color::BLACK,
            }))
            .expect("a shadow");
        composer.with_scene(|scene| {
            scene
                .insert(node, NodeSpec::new(Rect::new(0, 0, 100, 100)))
                .expect("the node");
            scene.set_effects(node, chain).expect("the effects");
        });
        composer.take_tree_changes();

        composer.publish_skipped_effects();
        assert_eq!(queued_skipped(&mut composer, node), None, "a flat node skips nothing");

        composer.with_scene(|scene| {
            scene
                .set_property(node, VisualProperty::RotationX, PropertyValue::Number(30.0))
                .expect("a tilt");
        });
        composer.publish_skipped_effects();
        let want = vec![
            "blur: unsupported-effect-on-3d".to_owned(),
            "shadow: unsupported-effect-on-3d".to_owned(),
        ];
        assert_eq!(queued_skipped(&mut composer, node), Some(want.clone()));
        assert_eq!(
            composer.with_scene(|scene| scene.value(node, "skipped").cloned()),
            Some(TreeValue::ListText(want)),
            "the node itself carries the list a snapshot reads"
        );

        composer.publish_skipped_effects();
        assert_eq!(
            queued_skipped(&mut composer, node),
            None,
            "an unchanged list is not republished"
        );

        composer.with_scene(|scene| {
            scene
                .set_property(node, VisualProperty::RotationX, PropertyValue::Number(0.0))
                .expect("flat again");
        });
        composer.publish_skipped_effects();
        assert_eq!(
            queued_skipped(&mut composer, node),
            Some(Vec::new()),
            "a node that is flat again gets the empty list back"
        );
    }

    /// A blur and a shadow, in that order.
    fn blur_and_shadow() -> Effects {
        use gnoblin_render::{BlurParams, Shadow};
        let mut chain = Effects::new();
        chain
            .push(Pass::Blur(BlurParams {
                radius: 4.0,
                passes: 2,
                ..BlurParams::default()
            }))
            .expect("a blur");
        chain
            .push(Pass::Shadow(Shadow {
                offset: Vec3::new(0.0, 4.0, 0.0),
                blur: 12.0,
                spread: 0.0,
                opacity: 1.0,
                color: Color::BLACK,
            }))
            .expect("a shadow");
        chain
    }

    #[test]
    fn an_effect_the_renderer_cannot_draw_is_reported_skipped_by_kind() {
        let (mut composer, _output) = screen();
        let node = NodeId::new(0x4000);
        composer.with_scene(|scene| {
            scene
                .insert(node, NodeSpec::new(Rect::new(0, 0, 100, 100)))
                .expect("the node");
            scene.set_effects(node, blur_and_shadow()).expect("the effects");
        });
        composer.take_tree_changes();

        composer.set_effect_kinds(&["shadow"]);
        composer.publish_skipped_effects();
        assert_eq!(
            queued_skipped(&mut composer, node),
            Some(vec!["blur: unsupported-by-renderer".to_owned()]),
            "a renderer that draws shadows only skips the blur"
        );

        composer.set_effect_kinds(&[]);
        composer.publish_skipped_effects();
        assert_eq!(
            queued_skipped(&mut composer, node),
            Some(vec![
                "blur: unsupported-by-renderer".to_owned(),
                "shadow: unsupported-by-renderer".to_owned(),
            ]),
            "a renderer that draws none skips both, in list order"
        );
        composer.publish_skipped_effects();
        assert_eq!(
            queued_skipped(&mut composer, node),
            None,
            "an unchanged list is not republished"
        );

        composer.set_effect_kinds(Pass::NAMES);
        composer.publish_skipped_effects();
        assert_eq!(
            queued_skipped(&mut composer, node),
            Some(Vec::new()),
            "a renderer that draws both skips nothing"
        );
    }

    #[test]
    fn an_effect_with_both_reasons_names_both_on_one_entry() {
        let (mut composer, _output) = screen();
        let node = NodeId::new(0x4000);
        composer.with_scene(|scene| {
            scene
                .insert(node, NodeSpec::new(Rect::new(0, 0, 100, 100)))
                .expect("the node");
            scene.set_effects(node, blur_and_shadow()).expect("the effects");
            scene
                .set_property(node, VisualProperty::RotationX, PropertyValue::Number(30.0))
                .expect("a tilt");
        });
        composer.take_tree_changes();

        composer.set_effect_kinds(&["shadow"]);
        composer.publish_skipped_effects();
        assert_eq!(
            queued_skipped(&mut composer, node),
            Some(vec![
                "blur: unsupported-by-renderer, unsupported-effect-on-3d".to_owned(),
                "shadow: unsupported-effect-on-3d".to_owned(),
            ])
        );
    }

    #[test]
    fn the_main_seat_publishes_the_effect_kinds_the_renderer_draws() {
        let mut composer = Composer::new();
        composer.set_effect_kinds(&[]);
        let seat = composer.add_main_seat("seat0").expect("a seat node");
        assert_eq!(
            composer.with_scene(|scene| scene.value(seat, "effect_kinds").cloned()),
            Some(TreeValue::ListText(Vec::new())),
            "a kind set before the seat exists reaches the seat when it is made"
        );
        composer.take_tree_changes();

        composer.set_effect_kinds(Pass::NAMES);
        let want = TreeValue::ListText(vec!["blur".to_owned(), "shadow".to_owned()]);
        assert!(composer.take_tree_changes().iter().any(|change| matches!(
            change,
            TreeChange::Set { node, key, value } if *node == seat && key == "effect_kinds" && *value == want
        )));
        composer.set_effect_kinds(Pass::NAMES);
        assert!(
            composer.take_tree_changes().is_empty(),
            "an unchanged list is not republished"
        );
    }
}

#[cfg(test)]
#[path = "surface_tree_tests.rs"]
mod surface_tree_tests;

#[cfg(test)]
#[path = "view_gpu_tests.rs"]
mod view_gpu_tests;
