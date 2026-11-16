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

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use smithay::backend::allocator::Buffer as _;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::utils::Transform;

use gnoblin_animation::{Easing, PropertyValue, SampledCurve, VisualProperty, VisualTransform};
use gnoblin_paint::{SharedPainter, decode};
use gnoblin_render::scene_ids::NodeIdAllocator;
use gnoblin_render::transaction::{
    Animation, Motion, ShellTrackEnd, TreeChange, TreeValue, TweenControl, animated_properties, land_at_once,
};
use gnoblin_render::{
    Damage, Effects, Mat4, NodeKind, NodeSpec, Pass, Projection, Scene, TrackEnd, TrackEndReason, TrackSpec,
    TrackStart, TweenPlayer,
};
use gnoblin_types::{Color, NodeId, OutputId, Point, PointF, Rect, RectF, Scale, Size, TrackId, Vec3, WindowId};

use gnoblin_render_frame::footprint::Footprint;
use gnoblin_render_frame::frame::{Layer, Model, OutputFrame, SecureTreatment, Solid, Surface, SurfaceSource};
use gnoblin_render_frame::image::Image;
use gnoblin_render_frame::shm;
use gnoblin_render_frame::view_scene::{ClipPlane, ViewEye, ViewOp, ViewScene, ViewTexture, size_fits};
use gnoblin_render_software::{
    Frustum, StereoRig, ViewCamera, ViewProjection, ViewRig, eye_pixel_matrix, eye_sizes, pick_view, render_rig,
};

use crate::log;

mod canvases;
use canvases::CanvasPicture;

mod lock_ui;

mod seat_input;
pub use seat_input::ShellCursor;

mod views;
use views::{Landing, VIEW_DEPTH, ViewPicture};

mod motion;
use motion::{Flash, MOTION_MS, ShellPlay};

/// Reads a client's GPU buffer into a picture, or answers `None` when the backend cannot. A View of a window that
/// shows a GPU buffer draws what this returns: see [`Composer::begin_frame_reading`].
pub type ReadDmabuf<'a> = dyn FnMut(&smithay::backend::allocator::dmabuf::Dmabuf) -> Option<Image> + 'a;

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

/// Why the composer refused a shell visual value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellVisualError {
    /// The value is not finite or lies outside the range the window methods document.
    OutOfRange,
}

/// How the compositor paces a window's client, as the window node's `frame_policy` and `frame_rate` say.
///
/// The shell writes both while it holds the tree; with no shell every window is [`FramePolicy::Auto`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FramePolicy {
    /// The client is answered after every frame that shows its window.
    #[default]
    Auto,
    /// The client is never answered: it draws nothing new until the policy changes.
    Hold,
    /// The client is answered at most once in `interval_ms` milliseconds.
    Throttle {
        /// The shortest time between two answers, in milliseconds. At least 1.
        interval_ms: u32,
    },
}

/// The rate a `throttle` window runs at while the shell wrote none: the schema's default `frame_rate`.
const DEFAULT_FRAME_RATE: f64 = 30.0;

/// What a window's client asked of `xdg-decoration`, as the window node's `decoration_wanted` publishes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DecorationWanted {
    /// The client never bound the protocol, asked for no preference, or is an X11 window.
    #[default]
    Unset,
    /// The client asked to draw its own frame.
    Client,
    /// The client asked for the compositor to draw its frame.
    Server,
}

impl DecorationWanted {
    /// The text the window node's `decoration_wanted` holds.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Unset => "unset",
            Self::Client => "client",
            Self::Server => "server",
        }
    }
}

/// Who draws a window's frame, as the compositor negotiated it and the window node's `decoration` publishes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowDecoration {
    /// The client draws its own title bar and borders.
    #[default]
    Client,
    /// The shell draws the frame.
    Server,
    /// Nobody does: an X11 window while no shell declares decorations.
    None,
}

impl WindowDecoration {
    /// The text the window node's `decoration` holds.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Server => "server",
            Self::None => "none",
        }
    }
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
    /// The window's frame rectangle in global logical coordinates: where the window geometry the client set
    /// (`xdg_surface.set_window_geometry`), or the whole surface when it set none, sits. The window's node is this
    /// rectangle: its position is the geometry's origin and its size the geometry's size.
    geometry: Rect,
    /// Where the window geometry starts inside the client's surface, in surface-local pixels; zero when it set none.
    /// The surface is drawn at the node's origin less this offset, so a shadow or a bar the client draws outside its
    /// window geometry is drawn and is not the window.
    surface_offset: (i32, i32),
    /// The size of the window geometry the client set, `None` while it set none and the whole surface is the window.
    geometry_size: Option<Size>,
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
    /// What the client asked of `xdg-decoration` and what the compositor answered. A node starts at the schema's
    /// defaults (`unset`, `client`).
    decoration: (DecorationWanted, WindowDecoration),
    /// The pair last published to the node.
    published_decoration: (DecorationWanted, WindowDecoration),
    /// The rectangle the window's node takes hits over beyond its own bounds, in surface-local pixels: the surface
    /// tree's, when a subsurface reaches past the root surface.
    hit_extent: Option<Rect>,
    /// The draw and hit extents last set on the node, in the node's local plane, so a sync that changes neither
    /// damages nothing.
    applied_extents: (Option<Rect>, Option<Rect>),
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
    /// The supersample factor: the output is composed at this multiple of its mode.
    pub supersample: f64,
    /// Whether the layout's primary logical monitor is on this output.
    pub primary: bool,
    /// The connector the backend knows the output by.
    pub connector: String,
    /// The monitor's EDID identity, empty when it reports none.
    pub identity: String,
    /// The connector this output shows a copy of, empty when it shows its own picture.
    pub mirror_of: String,
    /// Whether the output is on.
    pub enabled: bool,
}

/// What the memory budget says about the whole session: the bytes of every client's live buffers, the budget, the
/// requests refused since the session began and the bytes evicted from caches since then. They are published on every
/// output's screen node (`gpu_memory_*`), the same value on each, because the tree has no node for the session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionMemory {
    /// The bytes of every client's live buffers.
    pub clients: u64,
    /// The budget, or zero when it is off.
    pub budget: u64,
    /// The requests refused since the session began.
    pub refused: u64,
    /// The bytes evicted from caches since the session began.
    pub evicted: u64,
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
    /// The damage of earlier iterations on the outputs nobody asked a frame of, which joins the damage of the iteration
    /// after it. A frame is drawn from the damage, so a change an output was never asked about must wait for it.
    carried: Damage,
    /// The outputs a frame was asked of in this iteration: [`Composer::compose`] and every other way an output is
    /// described note it, and [`Composer::end_frame`] keeps the damage of the rest. Behind a cell because describing an
    /// output reads the composer and changes nothing a caller sees.
    asked: RefCell<BTreeSet<OutputId>>,
    /// Animations of window properties, driven by [`Composer::advance_motion`].
    motion: TweenPlayer,
    /// The live track for each window property, so a retarget can supersede it.
    tracks: BTreeMap<(WindowId, VisualProperty), TrackId>,
    /// The live track of each property the shell animated with a timed `set`, so a later write to
    /// the same property ends or supersedes it.
    shell_tracks: BTreeMap<(NodeId, VisualProperty), TrackId>,
    /// The timed `set`s of the shell whose tracks are still running, by an id of their own.
    shell_plays: BTreeMap<u64, ShellPlay>,
    /// The id the next play takes.
    next_play: u64,
    /// The play each running shell track belongs to.
    track_plays: BTreeMap<TrackId, u64>,
    /// The running tracks of each `tween` node's group, which its `progress`, `paused` and `reverse` drive.
    groups: BTreeMap<NodeId, BTreeSet<TrackId>>,
    /// The timed `set`s that have ended since the tree last took them, for the `track_ended` signal.
    shell_ends: Vec<ShellTrackEnd>,
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
            carried: Damage::default(),
            asked: RefCell::new(BTreeSet::new()),
            motion: TweenPlayer::new(),
            tracks: BTreeMap::new(),
            shell_tracks: BTreeMap::new(),
            shell_plays: BTreeMap::new(),
            next_play: 0,
            track_plays: BTreeMap::new(),
            groups: BTreeMap::new(),
            shell_ends: Vec::new(),
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

    /// Declares the screen node of an output the desktop gives no place of its own (a mirror, or one that is off):
    /// the node exists and carries `rect`, but the scene draws and damages nothing for it, so no window, pointer or
    /// damage computation sees a second monitor. A previous real declaration of the output is withdrawn.
    pub fn set_shadow_output(&mut self, output: OutputId, rect: Rect) {
        self.scene.remove_output(output);
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
            ("supersample", TreeValue::Double(info.supersample)),
            ("transform", TreeValue::Text("normal".to_owned())),
            ("primary", TreeValue::Bool(info.primary)),
            ("enabled", TreeValue::Bool(info.enabled)),
            ("connector", TreeValue::Text(info.connector.clone())),
            ("identity", TreeValue::Text(info.identity.clone())),
            ("mirror_of", TreeValue::Text(info.mirror_of.clone())),
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

    /// Publishes the bytes an output's renderer holds, and the bytes capture keeps of its last frame, to its screen
    /// node: `gpu_memory_*` in bytes, each announced as a `set` only when it differs from what the node holds, so a
    /// steady session publishes nothing. `session` is the memory budget's view of the whole session
    /// (`gpu_memory_clients_bytes`, `gpu_memory_budget_bytes`, `gpu_memory_refused_total`,
    /// `gpu_memory_evicted_total`), the same on every output's node. An output with no screen node yet publishes
    /// nothing.
    pub fn set_output_memory(
        &mut self,
        output: OutputId,
        footprint: &Footprint,
        capture_cache: u64,
        session: &SessionMemory,
    ) {
        let Some(node) = self.screens.get(&output).copied() else {
            return;
        };
        let total = footprint.total().saturating_add(capture_cache);
        let values = [
            ("gpu_memory_target_bytes", footprint.target),
            ("gpu_memory_scratch_bytes", footprint.scratch),
            ("gpu_memory_staging_bytes", footprint.staging),
            ("gpu_memory_views_bytes", footprint.views),
            ("gpu_memory_capture_bytes", capture_cache),
            ("gpu_memory_bytes", total),
            ("gpu_memory_clients_bytes", session.clients),
            ("gpu_memory_budget_bytes", session.budget),
            ("gpu_memory_refused_total", session.refused),
            ("gpu_memory_evicted_total", session.evicted),
        ];
        for (key, bytes) in values {
            // Bytes are published as doubles like every other number on the tree; a count below 2^53 is exact.
            let value = TreeValue::Double(bytes as f64);
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

    /// Notes that a frame was asked of `output` in this iteration, so its damage is spent and not carried to the next.
    ///
    /// [`Composer::compose`] does it. A caller that describes an output another way (the lock's cover, a mirror) calls
    /// it itself, because that frame is drawn all the same.
    pub fn frame_asked(&self, output: OutputId) {
        self.asked.borrow_mut().insert(output);
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
        let mut damage = self.scene.take_damage();
        // What changed on an output nobody asked for a frame last time is still not on its screen.
        let carried = std::mem::take(&mut self.carried);
        if !carried.is_empty() {
            damage.absorb(carried, self.scene.outputs(), self.scene.damage_rect_limit());
        }
        self.damage = Some(damage);
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

    /// Whether `node` is `root` or below it.
    fn is_within(&self, node: NodeId, root: NodeId) -> bool {
        let mut current = Some(node);
        while let Some(id) = current {
            if id == root {
                return true;
            }
            current = self.scene.parent(id);
        }
        false
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

    /// Ends the iteration. The damage of an output a frame was asked of is spent; the damage of one nobody asked is kept
    /// for the next iteration, because the next frame is drawn from the damage and would otherwise miss the change.
    pub fn end_frame(&mut self) {
        let asked = std::mem::take(&mut *self.asked.borrow_mut());
        if let Some(mut damage) = self.damage.take() {
            damage.outputs.retain(|entry| !asked.contains(&entry.output));
            self.carried = damage;
        }
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
                geometry_size: None,
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
                decoration: (DecorationWanted::Unset, WindowDecoration::Client),
                published_decoration: (DecorationWanted::Unset, WindowDecoration::Client),
                applied_size: None,
                published_geometry: None,
                hit_extent: None,
                applied_extents: (None, None),
                tree: Vec::new(),
                tree_nodes: BTreeMap::new(),
                tree_keys: BTreeMap::new(),
            },
        );
        self.order.push(window);
        self.sync(window, false);
    }

    /// Sets the rectangle a window takes hits over beyond its own geometry, in surface-local pixels, or clears it.
    /// It changes what the pointer and touch reach and draws nothing.
    pub fn set_window_hit_extent(&mut self, window: WindowId, extent: Option<Rect>) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        entry.hit_extent = extent;
        self.apply_extents(window);
    }

    /// The size of a window's node: the window geometry the client set, or the whole surface while it set none.
    fn frame_size(entry: &WindowNode, content: Content) -> Size {
        entry
            .geometry_size
            .filter(|size| !size.is_empty())
            .unwrap_or(content.size)
    }

    /// Where a window's whole buffer lies in its node's plane and the rectangle of the node, when they are not the
    /// same: a window whose client draws outside its window geometry. `None` when the buffer is the window.
    fn own_placement(entry: &WindowNode) -> Option<(Rect, Rect)> {
        let content = entry.content?;
        let frame = Rect::from_size(Self::frame_size(entry, content));
        let (x, y) = entry.surface_offset;
        let local = Rect::new(-x, -y, content.size.width, content.size.height);
        (local != frame).then_some((local, frame))
    }

    /// Brings the node's draw and hit extents to what the window's surface needs. The node is the window geometry
    /// and its surface is placed at the geometry's offset inside it, so the buffer is drawn over a rectangle that
    /// starts at the negated offset and runs as far as the buffer does, and the surface tree is hit over the tree's
    /// bounds moved into the same plane.
    fn apply_extents(&mut self, window: WindowId) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        let (Some(node), Some(content)) = (entry.node, entry.content) else {
            return;
        };
        let frame = Rect::from_size(Self::frame_size(entry, content));
        let (x, y) = entry.surface_offset;
        let draw = Self::own_placement(entry).map(|(local, _)| local);
        let hit = entry
            .hit_extent
            .map(|extent| extent.translate(-x, -y))
            .filter(|extent| *extent != frame);
        let (applied_draw, applied_hit) = entry.applied_extents;
        entry.applied_extents = (draw, hit);
        if applied_draw != draw {
            let _ = self.scene.set_draw_extent(node, draw);
        }
        if applied_hit != hit {
            let _ = self.scene.set_hit_extent(node, hit);
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
    /// publish. The rectangle is the window geometry: its origin and its size, whatever the buffer the client draws
    /// around it.
    ///
    /// While the shell holds the tree its placement stands, so the position is not published and
    /// the tree is treated as not knowing it: the next publish after the hold writes it.
    fn publish_geometry(&mut self, window: WindowId) {
        let holds = self.scene.holder_attached();
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        let (Some(node), Some(content)) = (entry.node, entry.content) else {
            return;
        };
        let size = Self::frame_size(entry, content);
        let values = [
            f64::from(entry.geometry.x),
            f64::from(entry.geometry.y),
            f64::from(size.width),
            f64::from(size.height),
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
        let holds = self.scene.holder_attached();
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

    /// Records what a window's client asked of `xdg-decoration` and what the compositor answered, and publishes both
    /// to the node's `decoration_wanted` and `decoration` when they changed. A window with no node yet is published
    /// once it has one.
    pub fn set_window_decoration(&mut self, window: WindowId, wanted: DecorationWanted, decoration: WindowDecoration) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        entry.decoration = (wanted, decoration);
        self.publish_decoration(window);
    }

    /// What a window's client asked of `xdg-decoration` and what the compositor answered, as the composer holds it.
    #[must_use]
    pub fn window_decoration(&self, window: WindowId) -> Option<(DecorationWanted, WindowDecoration)> {
        self.windows.get(&window).map(|entry| entry.decoration)
    }

    /// Publishes a window's `decoration_wanted` and `decoration` when they differ from what its node holds.
    fn publish_decoration(&mut self, window: WindowId) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        let Some(node) = entry.node else {
            return;
        };
        let (wanted, decoration) = entry.decoration;
        let (published_wanted, published_decoration) = entry.published_decoration;
        entry.published_decoration = entry.decoration;
        let mut changes = Vec::new();
        if wanted != published_wanted {
            changes.push(("decoration_wanted", wanted.name()));
        }
        if decoration != published_decoration {
            changes.push(("decoration", decoration.name()));
        }
        for (key, text) in changes {
            let _ = self.scene.set_text(node, key, text.to_owned());
            self.tree_changes.push(TreeChange::Set {
                node,
                key: key.to_owned(),
                value: TreeValue::Text(text.to_owned()),
            });
        }
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
        if !self.scene.holder_attached() {
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
        if !self.scene.holder_attached() {
            return false;
        }
        let Some(node) = self.windows.get(&window).and_then(|entry| entry.node) else {
            return false;
        };
        matches!(self.scene.value(node, "tiled"), Some(TreeValue::Bool(true)))
    }

    /// The size of a window's node: the window geometry the client last committed (the whole surface while it set
    /// none), or the size the shell wrote since. This is the size an `xdg_toplevel.configure` asks for. `None` when
    /// the window has no node.
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

    /// Follows the shell's hold on the tree. When the hold changed since the last check, every
    /// window is resynced: placed by the composer again, or left to the shell.
    pub fn follow_placement(&mut self) {
        let holds = self.scene.holder_attached();
        if holds == self.placing {
            return;
        }
        self.placing = holds;
        if !holds {
            self.release_shell_controls();
        }
        let order = self.order.clone();
        for window in order {
            self.sync_animated(window);
        }
    }

    /// The transform a window's node should reach. While the shell holds the tree, the node's
    /// x and y and its corner radius are the shell's, so the target keeps them where they are:
    /// a style writes the radius to the node, and a retarget that put the window's own (zero) back
    /// would square the corners again at the next sync.
    fn placed_target(&self, entry: &WindowNode) -> VisualTransform {
        let mut transform = Self::target(entry);
        if self.scene.holder_attached() {
            let current = entry.node.and_then(|node| self.scene.transform(node));
            let (x, y) = current.map_or((0.0, 0.0), |current| (current.base.offset.x, current.base.offset.y));
            transform.base.offset.x = x;
            transform.base.offset.y = y;
            if let Some(current) = current {
                transform.corner_radius = current.corner_radius;
            }
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
    /// `None` when there is no picture to read, or when motion is reduced or the shell turned the compositor's own
    /// animations off, so the window is hidden or removed at once.
    fn snapshot(&self, window: WindowId, on_end: CloseEnd) -> Option<Closing> {
        if !self.builtin_motion_plays() {
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

    /// Sets the window geometry the client committed (`xdg_surface.set_window_geometry`), in its surface's
    /// coordinates, or `None` while it set none and the whole surface is the window. The window's node takes the
    /// geometry's size, and the surface is drawn at the node's origin less the geometry's origin.
    pub fn set_window_client_geometry(&mut self, window: WindowId, geometry: Option<Rect>) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        let geometry = geometry.filter(|geometry| !geometry.is_empty());
        let offset = geometry.map_or((0, 0), |geometry| (geometry.x, geometry.y));
        let size = geometry.map(Rect::size);
        if entry.surface_offset == offset && entry.geometry_size == size {
            return;
        }
        entry.surface_offset = offset;
        entry.geometry_size = size;
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

    /// A window's node's world matrix, as the scene currently holds it, or `None` when it is not drawn.
    #[must_use]
    pub fn window_matrix(&self, window: WindowId) -> Option<Mat4> {
        let node = self.windows.get(&window)?.node?;
        self.scene.world_matrix(node)
    }

    /// A window's node's bounds projected to the output plane, including what the node draws past them (a client's
    /// shadow outside its window geometry), or `None` when it is not drawn.
    #[must_use]
    pub fn window_projected_bounds(&self, window: WindowId) -> Option<Rect> {
        let node = self.windows.get(&window)?.node?;
        self.scene.absolute_bounds(node)
    }

    /// The window geometry on the output plane, rounded to whole pixels: the rectangle of the window's node, where a
    /// shell that holds the tree put the window. `None` when the window has no node yet or it is not drawn.
    #[must_use]
    pub fn window_frame(&self, window: WindowId) -> Option<Rect> {
        let node = self.windows.get(&window)?.node?;
        self.scene.content_bounds(node).map(gnoblin_types::RectF::to_pixel)
    }

    /// The window a ray at a point on the output plane lands on, and the point in that window's local space, which is
    /// relative to the window geometry's origin (the surface point is that point plus the geometry's origin in the
    /// surface). The topmost window wins, and a window `accept` refuses passes the point through to what lies under it.
    ///
    /// A View above the windows that forwards its input takes the ray into its source first (see
    /// [`Composer::shell_node_at`]): the window is then the one the View's picture shows under the ray, wherever that
    /// window is drawn on the output, and a ray that lands on nothing in the picture lands on no window.
    #[must_use]
    pub fn window_at(&self, point: PointF, accept: &dyn Fn(WindowId) -> bool) -> Option<(WindowId, PointF)> {
        let hit = self.scene.hit_test_where(point, &|node| {
            self.window_of(node).is_some_and(accept) || self.forwards_input(node)
        })?;
        match self.landing(hit.node, hit.local, VIEW_DEPTH) {
            Landing::Window(window, local) if accept(window) => Some((window, local)),
            _ => None,
        }
    }

    /// The window whose node is `node`.
    fn window_of(&self, node: NodeId) -> Option<WindowId> {
        self.windows
            .iter()
            .find_map(|(window, entry)| (entry.node == Some(node)).then_some(*window))
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

    /// Whether the shell that holds the tree has declared that it draws decorations: the main seat's
    /// `decorations_available` is true and a shell holds the tree. With no shell, or one that has not declared it, a
    /// client is never told the compositor draws its frame.
    #[must_use]
    pub fn decorations_available(&self) -> bool {
        self.scene.holder_attached() && self.decorations_declared()
    }

    /// Whether the main seat's `decorations_available` is true, whoever holds the tree.
    #[must_use]
    pub fn decorations_declared(&self) -> bool {
        self.main_seat.is_some_and(|seat| {
            matches!(
                self.scene.value(seat, "decorations_available"),
                Some(TreeValue::Bool(true))
            )
        })
    }

    /// Clears the main seat's `decorations_available`: the shell that wrote it no longer holds the tree, so nothing
    /// draws decorations. Published like any other change, so the next shell starts from a seat nobody declared on.
    pub fn release_decorations_declared(&mut self) {
        let Some(seat) = self.main_seat else {
            return;
        };
        if self
            .scene
            .set_value(seat, "decorations_available", TreeValue::Bool(false))
            .is_ok()
        {
            self.tree_changes.push(TreeChange::Set {
                node: seat,
                key: "decorations_available".to_owned(),
                value: TreeValue::Bool(false),
            });
        }
    }

    /// Whether the shell that holds the tree turned the compositor's own window animations off: the main seat's
    /// `builtin_motion` is false and a shell holds the tree. With no shell the seat's value is ignored, so the
    /// compositor's own desktop animates as it always has.
    #[must_use]
    pub fn builtin_motion_off(&self) -> bool {
        self.scene.holder_attached()
            && self
                .main_seat
                .is_some_and(|seat| matches!(self.scene.value(seat, "builtin_motion"), Some(TreeValue::Bool(false))))
    }

    /// Whether a window's fade in, fade out and placement tween play: motion is not reduced and the shell did not
    /// turn the compositor's own animations off. The shell's own timed writes do not ask this; they play unless
    /// motion is reduced.
    fn builtin_motion_plays(&self) -> bool {
        !self.reduced_motion && !self.builtin_motion_off()
    }

    /// How the shell asked a window's client to be paced: its node's `frame_policy` and `frame_rate`. A window with no
    /// node, a policy nobody wrote, or any policy while no shell holds the tree is [`FramePolicy::Auto`]. A refused
    /// value never reaches the node, so the text is one of the three the schema names.
    #[must_use]
    pub fn window_frame_policy(&self, window: WindowId) -> FramePolicy {
        if !self.scene.holder_attached() {
            return FramePolicy::Auto;
        }
        let Some(node) = self.windows.get(&window).and_then(|entry| entry.node) else {
            return FramePolicy::Auto;
        };
        match self.scene.value(node, "frame_policy") {
            Some(TreeValue::Text(policy)) if policy == "hold" => FramePolicy::Hold,
            Some(TreeValue::Text(policy)) if policy == "throttle" => {
                let rate = match self.scene.value(node, "frame_rate") {
                    Some(TreeValue::Double(rate)) if *rate > 0.0 => *rate,
                    _ => DEFAULT_FRAME_RATE,
                };
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let interval_ms = (1000.0 / rate).round().max(1.0) as u32;
                FramePolicy::Throttle { interval_ms }
            }
            _ => FramePolicy::Auto,
        }
    }

    /// Writes `presented` on a window's node, the time (milliseconds on `CLOCK_MONOTONIC`) the frame that showed the
    /// window reached the screen, when the shell set the node's `report_presented`. A window that is not reported, or
    /// whose last stamp is the same millisecond, changes nothing.
    pub fn stamp_presented(&mut self, window: WindowId, millis: u64) {
        let Some(node) = self.windows.get(&window).and_then(|entry| entry.node) else {
            return;
        };
        if !matches!(self.scene.value(node, "report_presented"), Some(TreeValue::Bool(true))) {
            return;
        }
        let value = TreeValue::Uint(millis);
        if self.scene.value(node, "presented") == Some(&value) {
            return;
        }
        if self.scene.set_value(node, "presented", value.clone()).is_ok() {
            self.tree_changes.push(TreeChange::Set {
                node,
                key: "presented".to_owned(),
                value,
            });
        }
    }

    /// Sets what a shell wrote to control the compositor back to its default, once the shell no longer holds the tree:
    /// the seat's `builtin_motion` is true again and every window's `frame_policy` is `auto`, its `frame_rate` 30 and
    /// its `report_presented` false. A window cannot stay held with nobody to release it, and the next shell starts
    /// from the built-in behaviour. Each change is published like any other.
    fn release_shell_controls(&mut self) {
        let mut writes: Vec<(NodeId, &'static str, TreeValue)> = Vec::new();
        if let Some(seat) = self.main_seat
            && self.scene.value(seat, "builtin_motion") == Some(&TreeValue::Bool(false))
        {
            writes.push((seat, "builtin_motion", TreeValue::Bool(true)));
        }
        for node in self.windows.values().filter_map(|entry| entry.node) {
            for (key, value) in [
                ("frame_policy", TreeValue::Text("auto".to_owned())),
                ("frame_rate", TreeValue::Double(DEFAULT_FRAME_RATE)),
                ("report_presented", TreeValue::Bool(false)),
            ] {
                if self.scene.value(node, key).is_some_and(|held| *held != value) {
                    writes.push((node, key, value));
                }
            }
        }
        for (node, key, value) in writes {
            if self.scene.set_value(node, key, value.clone()).is_ok() {
                self.tree_changes.push(TreeChange::Set {
                    node,
                    key: key.to_owned(),
                    value,
                });
            }
        }
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

    /// The shell's node a ray at a point on the output plane lands on, with the point in that node's local space. A
    /// window above it wins: only a topmost node that is not a window is the shell's, so a shell node under a window is
    /// not reported. A node the compositor created since the last frame and has not yet published in a batch is
    /// skipped (F29): the shell has not heard of it, so it cannot be sent input, and the pick falls through to what
    /// lies under it.
    ///
    /// A View is where `input_mode` decides. `capture` lands on the View itself, so it is the shell's. `forward` (the
    /// default) takes the ray into the View's source, through the camera the picture was drawn with, and lands on what
    /// the picture shows there: a window is the client's and the shell is sent nothing for the View, another node of
    /// the source is the shell's, and a point where the picture shows nothing that takes input lands on nothing. A
    /// View that has no source has nothing to forward to and is the shell's like any other node.
    #[must_use]
    pub fn shell_node_at(&self, point: PointF) -> Option<(NodeId, PointF)> {
        let hit = self
            .scene
            .hit_test_where(point, &|node| self.window_of(node).is_some() || !self.unpublished(node))?;
        match self.landing(hit.node, hit.local, VIEW_DEPTH) {
            Landing::Node(node, local) => Some((node, local)),
            Landing::Window(..) | Landing::Nowhere => None,
        }
    }

    /// `point` of the output plane in the local space of the shell node `node`, wherever it lies: what a press that
    /// holds the node is told of the pointer while the button stays down. `None` for a node that has gone.
    #[must_use]
    pub fn shell_node_local(&self, node: NodeId, point: PointF) -> Option<PointF> {
        self.scene.local_point(node, point)
    }

    /// Whether a node the compositor created has not been published in a batch yet.
    fn unpublished(&self, node: NodeId) -> bool {
        self.tree_changes
            .iter()
            .any(|change| matches!(change, TreeChange::Created { node: created, .. } if *created == node))
    }

    /// Whether a holder is attached to the tree's placement, so window requests are the shell's to decide.
    #[must_use]
    pub fn holder_attached(&self) -> bool {
        self.scene.holder_attached()
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
        // A surface hung straight from the window's own surface is placed in the node's plane, which starts at the
        // window geometry's origin inside that surface.
        let origin = entry.surface_offset;
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
            let transform = Self::tree_transform(surface, Self::tree_origin(surface, origin));
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
                self.publish_tree_geometry(node, surface, Self::tree_origin(surface, origin));
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

    /// Where a surface's node sits in its parent's plane. A surface hung from the window's own surface is in the
    /// surface's coordinates, and its parent is the window's node, whose plane starts at the window geometry's origin
    /// `root` inside that surface. A surface hung from another surface is placed in that surface's plane as it is.
    fn tree_origin(surface: &TreeSurface, root: (i32, i32)) -> (i32, i32) {
        match surface.parent {
            None => (surface.offset.x - root.0, surface.offset.y - root.1),
            Some(_) => (surface.offset.x, surface.offset.y),
        }
    }

    /// The transform a surface's node holds: its place in its parent's plane, `at`, and its alpha factor.
    fn tree_transform(surface: &TreeSurface, at: (i32, i32)) -> VisualTransform {
        let mut transform = VisualTransform::IDENTITY;
        transform.base.offset = Vec3::new(f64::from(at.0), f64::from(at.1), 0.0);
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
    fn publish_tree_geometry(&mut self, node: &mut TreeNode, surface: &TreeSurface, at: (i32, i32)) {
        let values = [
            f64::from(at.0),
            f64::from(at.1),
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
        if !self.scene.holder_attached() {
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
        self.frame_asked(output);
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
        // below what the tree puts after it. Windows keep the stacking order; canvases wait their turn. So does a
        // window's border ring, which is drawn over the whole frame: after the window's own surfaces and after every
        // canvas of its subtree (its titlebar), and before the next window.
        let ringed: Vec<(WindowId, NodeId)> = if self.effect_kinds.contains(&"border") {
            self.order
                .iter()
                .filter_map(|window| {
                    let node = self.windows.get(window)?.node?;
                    self.scene.effects(node)?.has("border").then_some((*window, node))
                })
                .collect()
        } else {
            Vec::new()
        };
        let ranks = if self.canvases.is_empty() && ringed.is_empty() {
            BTreeMap::new()
        } else {
            self.paint_ranks()
        };
        // Keyed by twice the paint rank, so a ring (the odd key after the last rank of its subtree) sorts after the
        // canvases under it and before the next node.
        // The canvases `placement` puts under a window are painted before it, in their own paint order.
        let under = self.canvases_under_windows();
        let mut queue: Vec<(u64, QueuedLayer)> = self
            .canvases
            .iter()
            .filter(|(node, picture)| picture.image.is_some() && self.scene.drawn(**node))
            .filter(|(node, _)| !under.values().any(|nodes| nodes.contains(node)))
            .filter_map(|(node, _)| Some((u64::from(*ranks.get(node)?) * 2, QueuedLayer::Canvas(*node))))
            .collect();
        for (window, node) in ringed {
            if let Some(rank) = ranks.get(&node) {
                let last = u64::from(*rank) + self.descendants(node);
                queue.push((last * 2 + 1, QueuedLayer::Ring(window)));
            }
        }
        queue.sort_unstable_by_key(|(key, _)| *key);
        let mut waiting = queue.into_iter().peekable();
        for window in &self.order {
            let Some(entry) = self.windows.get(window) else {
                continue;
            };
            let Some(window_node) = entry.node else {
                continue;
            };
            if let Some(rank) = ranks.get(&window_node) {
                while let Some((_, item)) = waiting.next_if(|(key, _)| *key < u64::from(*rank) * 2) {
                    self.push_waiting(item, rect, frame, &mut spatial);
                }
            }
            for canvas in under.get(&window_node).into_iter().flatten() {
                if self.canvases.get(canvas).is_some_and(|picture| picture.image.is_some()) && self.scene.drawn(*canvas)
                {
                    self.push_canvas(*canvas, rect, frame, &mut spatial);
                }
            }
            self.push_window(*window, entry, window_node, rect, frame, &mut spatial);
        }
        for (_, item) in waiting {
            self.push_waiting(item, rect, frame, &mut spatial);
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
                silhouette: None,
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

    /// Draws what waited for its place in the paint order: a canvas, or a window's border ring.
    fn push_waiting(&self, item: QueuedLayer, output: Rect, frame: &mut OutputFrame, spatial: &mut bool) {
        match item {
            QueuedLayer::Canvas(canvas) => self.push_canvas(canvas, output, frame, spatial),
            QueuedLayer::Ring(window) => self.push_ring(window, output, frame),
        }
    }

    /// The number of nodes under `node`, which is how many paint ranks its subtree takes after its own.
    fn descendants(&self, node: NodeId) -> u64 {
        let mut count = 0;
        let mut pending: Vec<NodeId> = self.scene.children(node).unwrap_or(&[]).to_vec();
        while let Some(next) = pending.pop() {
            count += 1;
            pending.extend(self.scene.children(next).unwrap_or(&[]));
        }
        count
    }

    /// The frame of a window node on `output`, in the output's own logical space: the rectangle its content covers,
    /// together with the decoration children that lie outside it (a titlebar above the content, an edge of the
    /// shell's frame) and are part of the frame (`placement` `frame`), and the corner radius the node carries.
    /// Everything a window draws, its content, its decoration, its borders and, when it asks for the frame, its shadow,
    /// is rounded and outlined by this one shape.
    fn window_shape(&self, node: NodeId, output: Rect) -> Option<WindowShape> {
        let mut area = self.scene.content_bounds(node)?;
        for child in self.scene.children(node).unwrap_or(&[]) {
            let drawn = self.canvases.get(child).is_some_and(|picture| picture.image.is_some());
            // A layer over or under the window (`placement`) is drawn with the window and is not part of its frame.
            if drawn
                && self.scene.framed(*child) != Some(false)
                && self.scene.drawn(*child)
                && let Some(bounds) = self.scene.content_bounds(*child)
            {
                area = area.union(bounds);
            }
        }
        let placed = area.to_pixel();
        Some(WindowShape {
            rect: Rect::new(
                placed.x.saturating_sub(output.x),
                placed.y.saturating_sub(output.y),
                placed.width,
                placed.height,
            ),
            radius: self.scene.transform(node)?.corner_radius,
        })
    }

    /// Adds the border ring of a window: a layer with no pixels of its own that carries the window's borders and is
    /// drawn over its frame, above its titlebar. A window placed in 3D or rotated about the depth axis has no ring:
    /// the renderers have no depth pass for a stroke, and its node's `skipped` list says so.
    fn push_ring(&self, window: WindowId, output: Rect, frame: &mut OutputFrame) {
        let Some(node) = self.windows.get(&window).and_then(|entry| entry.node) else {
            return;
        };
        let (Some(effects), Some(opacity), Some(transform)) = (
            self.scene.effects(node),
            self.scene.composed_opacity(node),
            self.scene.transform(node),
        ) else {
            return;
        };
        if !self.scene.drawn(node) || self.spatial_world(node).is_some() || transform.base.rotation_degrees.z != 0.0 {
            return;
        }
        let (_, borders) = effects.split_borders();
        let Some(shape) = self.window_shape(node, output) else {
            return;
        };
        if borders.is_empty() {
            return;
        }
        let mut ring = Solid::new(shape.rect, Color::TRANSPARENT);
        ring.tint = Color::new(1.0, 1.0, 1.0, opacity.clamp(0.0, 1.0));
        ring.corner_radius = shape.radius;
        ring.effects = borders;
        frame.push_layer(Layer::Solid(ring));
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
            f64::from(entry.geometry.x) + shell_x,
            f64::from(entry.geometry.y) + shell_y,
            transform.base.offset.z,
        );
        let scale = f64::from(entry.shell_scale);
        transform.base.scale.x *= scale;
        transform.base.scale.y *= scale;
        transform.opacity *= entry.shell_opacity * entry.surface_alpha * entry.rule_opacity;
        transform
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
                entry.applied_extents = (None, None);
                // The nodes under the window's node went with it; the tree is kept to be made again.
                entry.tree_nodes.clear();
                entry.tree_keys.clear();
            }
            return;
        };
        let size = self
            .windows
            .get(&window)
            .map_or(content.size, |entry| Self::frame_size(entry, content));
        let bounds = Rect::from_size(size);
        match node {
            Some(node) => {
                // The window keeps its node: a size change moves the node's bounds, so its id and
                // the tree's view of it stay the same. The size is the window geometry the client set, so a shell's
                // write of it survives a commit that leaves the geometry as it was.
                let resized = self.windows.get(&window).and_then(|entry| entry.applied_size) != Some(size);
                if resized {
                    if self.scene.bounds(node) != Some(bounds) {
                        let _ = self.scene.set_bounds(node, bounds);
                    }
                    if let Some(entry) = self.windows.get_mut(&window) {
                        entry.applied_size = Some(size);
                    }
                }
                if force && let Some(current) = self.scene.transform(node).copied() {
                    let _ = self.scene.set_transform(node, current);
                }
                self.apply_extents(window);
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
                let fades_in = self.builtin_motion_plays();
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
                            entry.applied_size = Some(size);
                            entry.applied_extents = (None, None);
                        }
                        self.apply_extents(window);
                        self.publish_geometry(window);
                        if fades_in {
                            self.retarget(window, true);
                        }
                        self.sync_tree(window);
                        self.publish_relations(window);
                        self.publish_decoration(window);
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
            let (x, y) = entry.surface_offset;
            // The window's borders are drawn over its whole frame by a layer of their own (`push_ring`), so the
            // content's layer carries the rest of the chain. The content, and with it the corner radius, is
            // rounded to the frame, not to its own rectangle, when the window has a titlebar above it.
            let (effects, _) = self.scene.effects(node).map(Effects::split_borders).unwrap_or_default();
            let radius = pose.transform.corner_radius;
            let silhouette = (radius > 0.0 || effects.has_frame_shadow())
                .then(|| self.window_shape(node, output))
                .flatten()
                .map(|shape| shape.rect);
            let drawn = Drawn {
                source,
                content,
                local: Some(Rect::new(-x, -y, content.size.width, content.size.height)),
                // The window's placed pose, mid-tween, not its target.
                opacity: pose.transform.opacity,
                corner_radius: radius,
                silhouette,
                effects,
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
                local: None,
                opacity: self.opacity_within(paint.node, *child),
                corner_radius: 0.0,
                silhouette: None,
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
        let placed = match drawn.local {
            Some(local) => self.scene.project_local(node, local),
            None => self.scene.content_bounds(node),
        };
        let Some(placed) = placed.map(RectF::to_pixel) else {
            return;
        };
        let dst = Rect::new(
            placed.x.saturating_sub(output.x),
            placed.y.saturating_sub(output.y),
            placed.width,
            placed.height,
        );
        // A surface drawn in 3D is placed by the node's matrix, which maps the node's own plane: the picture starts
        // where the node's plane puts it.
        let model = self.model(node, output).map(|mut model| {
            if let Some(local) = drawn.local {
                model.matrix = model
                    .matrix
                    .mul(&Mat4::translation(f64::from(local.x), f64::from(local.y), 0.0));
            }
            model
        });
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
            silhouette: drawn.silhouette.filter(|_| model.is_none()),
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
    /// Where the picture sits in its node's own plane when it does not start at the node's corner: the toplevel
    /// surface of a window whose node is the window geometry starts at the geometry's offset, negated.
    local: Option<Rect>,
    opacity: f32,
    corner_radius: f32,
    /// The window's frame the corner radius rounds, when the layer is one part of a decorated window.
    silhouette: Option<Rect>,
    effects: Effects,
}

/// What waits for its place in the paint order while windows are drawn.
enum QueuedLayer {
    /// A canvas node's picture.
    Canvas(NodeId),
    /// A window's border ring, which is drawn over its whole frame, after its titlebar.
    Ring(WindowId),
}

/// A window's frame on one output: the rounded shape its content, its decoration children, its borders and, when it
/// asks for the frame, its shadow share.
struct WindowShape {
    /// The frame's rectangle, in the output's logical space.
    rect: Rect,
    /// The corner radius of the frame, in logical pixels; 0 for square corners.
    radius: f32,
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
mod tests;

#[cfg(test)]
#[path = "decorated_window_tests.rs"]
mod decorated_window_tests;

#[cfg(test)]
#[path = "surface_tree_tests.rs"]
mod surface_tree_tests;

#[cfg(test)]
#[path = "view_gpu_tests.rs"]
mod view_gpu_tests;

#[cfg(test)]
#[path = "idle_cost_tests.rs"]
mod idle_cost_tests;
