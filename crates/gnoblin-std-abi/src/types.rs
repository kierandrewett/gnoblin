//! The std ABI's types, constants, callbacks, table and exports.
//!
//! `libgnoblin-std` holds the desktop object model above the core: the collection of clients, workspaces and the layout driver.
//! A shell that links it reaches it through one versioned table (`GnoblinStdApi`, behind `gnoblin_std_get_api`), written by the
//! same rules as the core's: no Rust type crosses the boundary, strings are `(pointer, length)` UTF-8, inputs are borrowed for the
//! call, each side frees what it allocated, and a struct a caller passes in starts with `size: u32`.
//!
//! Two things differ from the core's table, both on purpose:
//!
//! - Every function takes the **core's** generation context (`GnoblinCtx`) first. The std library keeps its state per generation
//!   and acts on the compositor only through the installed core, so a retired generation's calls fail the way the core's do.
//! - The library **never runs a shell callback inside a table call**. A change a call makes is announced from the next turn of the
//!   shell's dispatch, with the other events of that turn, so a callback cannot re-enter the call that caused it.

use core::ffi::c_void;

use gnoblin_core_abi::{GnoblinCtx, GnoblinDropFn, GnoblinNode, GnoblinStr};
use gnoblin_core_abi::{abi_aliases, abi_callbacks, abi_consts, abi_exports, abi_structs};

abi_aliases! {
    /// A registration the std library holds for a shell: a listener on a collection. Owned by the generation that made it;
    /// `remove` ends it, and the generation's retirement ends the rest.
    pub type GnoblinStdReg = u64;
    /// A live collection of clients, outputs or workspaces: a root, a workspace's clients, or a selection of another
    /// collection. Zero is nil.
    pub type GnoblinStdCollection = u64;
    /// A workspace, by the library's own id. Zero is nil; an id is never reused in one process.
    pub type GnoblinStdWorkspace = u64;
}

abi_callbacks! {
    /// A collection event. `event` is valid until the callback returns. Returns `E_OK`, or `E_PANIC` when the SDK trampoline
    /// caught a panic.
    pub type GnoblinStdEventFn = unsafe extern "C" fn(user: *mut c_void, event: *const GnoblinStdEvent) -> i32;
    /// A layout's `compute`: places the workspace's tiled clients by calling `cx.emit` once for each placement. Returns `E_OK`,
    /// or a failure code, in which case the placements already emitted are dropped and nothing is written.
    pub type GnoblinStdComputeFn = unsafe extern "C" fn(user: *mut c_void, cx: *const GnoblinStdLayoutContext) -> i32;
    /// A layout hook that names one client: it joined or left the workspace.
    pub type GnoblinStdClientHookFn = unsafe extern "C" fn(user: *mut c_void, cx: *const GnoblinStdLayoutContext, client: GnoblinNode);
    /// A layout hook told that the workspace's area changed.
    pub type GnoblinStdAreaHookFn = unsafe extern "C" fn(user: *mut c_void, old: *const GnoblinStdRect, new: *const GnoblinStdRect);
    /// A layout hook told that two clients traded places in the workspace's order.
    pub type GnoblinStdSwapHookFn = unsafe extern "C" fn(user: *mut c_void, a: GnoblinNode, b: GnoblinNode);
    /// A layout's revision: a number that changes whenever one of the layout's own options does, so the workspace computes
    /// again. The library reads it before every pass.
    pub type GnoblinStdRevisionFn = unsafe extern "C" fn(user: *mut c_void) -> u64;
    /// The sink a layout's `compute` gives its placements to: `sink` is `GnoblinStdLayoutContext.sink`. Returns `E_OK`, or
    /// `E_TYPE` for a placement that names no client of the workspace or holds a rectangle that is not finite.
    pub type GnoblinStdEmitFn = unsafe extern "C" fn(sink: *mut c_void, placement: *const GnoblinStdPlacement) -> i32;
    /// A layout's `find_split`: the divider that a drag which pressed `client` at (`x`, `y`) would move, written to `out`. Returns 1
    /// when there is one, 0 when the layout has none there, or a failure code.
    pub type GnoblinStdFindSplitFn = unsafe extern "C" fn(user: *mut c_void, client: GnoblinNode, x: f64, y: f64, out: *mut GnoblinStdSplit) -> i32;
    /// A layout's `set_split`: moves the dividers of the drag that `find_split` named `token`, to `ratio` and, when the split had a
    /// second divider, `cross_ratio` (NaN when there is none). The layout holds the ratios to their range.
    pub type GnoblinStdSetSplitFn = unsafe extern "C" fn(user: *mut c_void, token: u64, ratio: f64, cross_ratio: f64);
    /// A layout's `reset_split`: puts the dividers of the drag named `token` back as they were when `find_split` returned them.
    pub type GnoblinStdResetSplitFn = unsafe extern "C" fn(user: *mut c_void, token: u64);
}

abi_consts! {
    /// `collection_root` kind: every client (window) of the compositor, in the order they appeared.
    pub const STD_COLLECTION_CLIENTS: u32 = 1;
    /// `collection_root` kind: every connected output.
    pub const STD_COLLECTION_OUTPUTS: u32 = 2;
    /// `collection_root` kind: the workspaces the shell added, in order.
    pub const STD_COLLECTION_WORKSPACES: u32 = 3;

    /// `GnoblinStdEvent.kind`: a member joined the collection.
    pub const STD_EVENT_ADDED: u32 = 1;
    /// `GnoblinStdEvent.kind`: a member left the collection.
    pub const STD_EVENT_REMOVED: u32 = 2;
    /// `GnoblinStdEvent.kind`: a client's title changed (`text`).
    pub const STD_EVENT_TITLE_CHANGED: u32 = 3;
    /// `GnoblinStdEvent.kind`: a client's application id changed (`text`).
    pub const STD_EVENT_APP_ID_CHANGED: u32 = 4;
    /// `GnoblinStdEvent.kind`: a client gained or lost the keyboard (`flag`).
    pub const STD_EVENT_FOCUS_CHANGED: u32 = 5;
    /// `GnoblinStdEvent.kind`: a client's or an output's geometry changed (`rect`). Several writes of one turn are one event.
    pub const STD_EVENT_GEOMETRY_CHANGED: u32 = 6;
    /// `GnoblinStdEvent.kind`: a client closed. The client is already gone from reads.
    pub const STD_EVENT_CLOSED: u32 = 7;
    /// `GnoblinStdEvent.kind`: a client completed its initial setup: it is a member of the root collection with its final values.
    pub const STD_EVENT_READY: u32 = 8;
    /// `GnoblinStdEvent.kind`: who draws a client's frame changed (`text`: `client`, `server` or `none`).
    pub const STD_EVENT_DECORATION_CHANGED: u32 = 9;
    /// `GnoblinStdEvent.kind`: an output's mode changed (`text`: `WxH@Hz`).
    pub const STD_EVENT_MODE_CHANGED: u32 = 10;

    /// `collection_listen` flag: tell the listener of every member now, as if each had just been added (the replay a shell that
    /// reloads uses to see the clients that are already there). The replay arrives with the next turn's events.
    pub const STD_LISTEN_REPLAY: u32 = 1;

    /// `GnoblinStdQueryTerm.kind`: `text` is compared with the property's text.
    pub const STD_TERM_TEXT: u32 = 1;
    /// `GnoblinStdQueryTerm.kind`: `number` is compared with the property's number.
    pub const STD_TERM_NUMBER: u32 = 2;
    /// `GnoblinStdQueryTerm.kind`: `flag` is compared with the property's boolean.
    pub const STD_TERM_BOOL: u32 = 3;

    /// `GnoblinStdLayoutSpec.kind`: the library's own floating layout. A window is placed once when it joins the workspace and
    /// is left where the user puts it.
    pub const STD_LAYOUT_FLOATING: u32 = 1;
    /// `GnoblinStdLayoutSpec.kind`: a layout the shell implements, reached through `vtable`.
    pub const STD_LAYOUT_FOREIGN: u32 = 2;

    /// `GnoblinStdLayoutSpec.placement`: the first spot of the area that overlaps nothing, else a cascade.
    pub const STD_PLACE_SMART: u32 = 0;
    /// `GnoblinStdLayoutSpec.placement`: centred, cascading only when that exact spot is taken.
    pub const STD_PLACE_CENTER: u32 = 1;
    /// `GnoblinStdLayoutSpec.placement`: a cascade from the area's top left.
    pub const STD_PLACE_CASCADE: u32 = 2;

    /// `GnoblinStdPlacement.flags`: `bounds` holds the client's rectangle; without it the client stays where it is.
    pub const STD_PLACEMENT_BOUNDS: u32 = 1;
    /// `GnoblinStdPlacement.flags`: the client is hidden (a monocle's other windows).
    pub const STD_PLACEMENT_HIDDEN: u32 = 2;
    /// `GnoblinStdPlacement.flags`: the client is in a tiled layout, so it drops the border and rounding it draws itself.
    pub const STD_PLACEMENT_TILED: u32 = 4;
    /// `GnoblinStdPlacement.flags`: `offset_x` and `offset_y` move the rectangle (or, without `bounds`, the client's own size
    /// centred in the area).
    pub const STD_PLACEMENT_OFFSET: u32 = 8;

    /// `GnoblinStdOutputPolicy.mode`: every output shows a workspace of its own (the default).
    pub const STD_OUTPUTS_SEPARATE: u32 = 0;
    /// `GnoblinStdOutputPolicy.mode`: every output shows what the lead output shows (the compositor mirrors them).
    pub const STD_OUTPUTS_DUPLICATE: u32 = 1;
    /// `GnoblinStdOutputPolicy.mode`: one workspace is laid out over the union of the outputs, which show a part each.
    pub const STD_OUTPUTS_EXTEND: u32 = 2;

    /// `GnoblinStdWorkspacePolicy.mode`: the shell adds and removes every workspace itself (the default).
    pub const STD_WORKSPACES_MANUAL: u32 = 0;
    /// `GnoblinStdWorkspacePolicy.mode`: the library adds one workspace for every output that appears and removes it when the
    /// output goes, its windows moving to a workspace that stays.
    pub const STD_WORKSPACES_PER_OUTPUT: u32 = 1;

    /// `GnoblinStdSplit.axis` and `.cross_axis`: the divider is carried by the pointer's horizontal travel.
    pub const STD_AXIS_X: u32 = 0;
    /// `GnoblinStdSplit.axis` and `.cross_axis`: the divider is carried by the pointer's vertical travel.
    pub const STD_AXIS_Y: u32 = 1;
    /// `GnoblinStdSplit.flags`: a second divider on the other axis moves with the same drag (the `cross_*` fields).
    pub const STD_SPLIT_CROSS: u32 = 1;

    /// `GnoblinStdClientState.flags`: `floating` holds the rectangle the client floats at.
    pub const STD_CLIENT_HAS_FLOATING: u32 = 1;
    /// `GnoblinStdClientState.flags`: `min_width` and `min_height` hold the least size the client may be resized to.
    pub const STD_CLIENT_HAS_MIN: u32 = 2;
    /// `GnoblinStdClientState.flags`: `max_width` and `max_height` hold the most the client may be resized to.
    pub const STD_CLIENT_HAS_MAX: u32 = 4;

    /// `client_snap` zone and `GnoblinStdClientState.snap_zone`: the client is not snapped.
    pub const STD_SNAP_NONE: u32 = 0;
    /// A snap zone: the left half of the work area.
    pub const STD_SNAP_LEFT: u32 = 1;
    /// A snap zone: the right half of the work area.
    pub const STD_SNAP_RIGHT: u32 = 2;
    /// A snap zone: all of the work area.
    pub const STD_SNAP_TOP: u32 = 3;
}

abi_structs! {
    /// A rectangle in the compositor's logical pixels.
    pub struct GnoblinStdRect {
        /// Left edge.
        pub x: f64,
        /// Top edge.
        pub y: f64,
        /// Width.
        pub width: f64,
        /// Height.
        pub height: f64,
    }

    /// A divider a drag moves, as a layout describes it: dragging the pointer `span` pixels along `axis` moves `ratio` by one, in
    /// the direction `direction` says. The layout holds `ratio` to `min` and `max`.
    pub struct GnoblinStdSplit {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// `STD_SPLIT_*` flags.
        pub flags: u32,
        /// Names this drag's dividers in `set_split` and `reset_split`. Valid until the next `find_split` of the layout.
        pub token: u64,
        /// `STD_AXIS_X` or `STD_AXIS_Y`.
        pub axis: u32,
        /// 1 when moving the pointer right or down grows `ratio`, -1 when it shrinks it.
        pub direction: i32,
        /// Pixels of pointer travel that change `ratio` by 1. Above zero.
        pub span: f64,
        /// Lowest ratio the layout allows.
        pub min: f64,
        /// Highest ratio the layout allows.
        pub max: f64,
        /// The divider's position as a ratio.
        pub ratio: f64,
        /// With `STD_SPLIT_CROSS`: the axis of the second divider.
        pub cross_axis: u32,
        /// With `STD_SPLIT_CROSS`: the direction of the second divider.
        pub cross_direction: i32,
        /// With `STD_SPLIT_CROSS`: the pointer travel that changes the second ratio by 1.
        pub cross_span: f64,
        /// With `STD_SPLIT_CROSS`: the lowest ratio of the second divider.
        pub cross_min: f64,
        /// With `STD_SPLIT_CROSS`: the highest ratio of the second divider.
        pub cross_max: f64,
        /// With `STD_SPLIT_CROSS`: the second divider's position as a ratio.
        pub cross_ratio: f64,
    }

    /// What the library remembers about a client beyond the tree: the room its frame takes, the sizes it allows itself, the
    /// rectangle it floats at and the zone it is snapped to. `client_state` fills it; `client_set_state` takes the first four
    /// groups and ignores the snap fields (`client_snap` and `client_unsnap` change those).
    pub struct GnoblinStdClientState {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// `STD_CLIENT_HAS_*` flags.
        pub flags: u32,
        /// The room the frame takes above the content, in logical pixels.
        pub inset_top: f64,
        /// The room the frame takes to the right of the content.
        pub inset_right: f64,
        /// The room the frame takes below the content.
        pub inset_bottom: f64,
        /// The room the frame takes to the left of the content.
        pub inset_left: f64,
        /// With `STD_CLIENT_HAS_MIN`: the least width a resize leaves the client.
        pub min_width: f64,
        /// With `STD_CLIENT_HAS_MIN`: the least height.
        pub min_height: f64,
        /// With `STD_CLIENT_HAS_MAX`: the most width.
        pub max_width: f64,
        /// With `STD_CLIENT_HAS_MAX`: the most height.
        pub max_height: f64,
        /// With `STD_CLIENT_HAS_FLOATING`: the rectangle the client floats at.
        pub floating: GnoblinStdRect,
        /// A `STD_SNAP_*` zone: where the client is snapped. Read only.
        pub snap_zone: u32,
        /// Zero.
        pub reserved: u32,
        /// The rectangle `client_unsnap` puts a snapped client back to. Read only.
        pub snap_floating: GnoblinStdRect,
    }

    /// One collection event, delivered to a `GnoblinStdEventFn`. Valid until the callback returns; the fields a kind does not
    /// use are zero or empty.
    pub struct GnoblinStdEvent {
        /// Size of this struct, set by the library.
        pub size: u32,
        /// One of the `STD_EVENT_*` kinds.
        pub kind: u32,
        /// The boolean a kind carries: the focus after a `STD_EVENT_FOCUS_CHANGED`.
        pub flag: u32,
        /// Zero.
        pub reserved: u32,
        /// The collection the listener is on.
        pub collection: GnoblinStdCollection,
        /// The member the event is about: a client's or an output's node id, or a workspace's id.
        pub member: u64,
        /// The text a kind carries: a title, an application id, a mode, a decoration.
        pub text: GnoblinStr,
        /// A geometry event's rectangle.
        pub rect: GnoblinStdRect,
    }

    /// One condition of a `collection_where` query: a property of the members and the value it has to equal. Terms of the same
    /// key are alternatives (any one matches); terms of different keys all have to match.
    pub struct GnoblinStdQueryTerm {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// `STD_TERM_TEXT`, `STD_TERM_NUMBER` or `STD_TERM_BOOL`.
        pub kind: u32,
        /// The value of a `STD_TERM_BOOL` term (0 or 1).
        pub flag: u32,
        /// Zero.
        pub reserved: u32,
        /// The property, by the name the member kind documents (`app_id`, `title`, `focused`, `width`, `id`).
        pub key: GnoblinStr,
        /// The value of a `STD_TERM_TEXT` term.
        pub text: GnoblinStr,
        /// The value of a `STD_TERM_NUMBER` term.
        pub number: f64,
    }

    /// An array of member ids the library lends for the call: `ptr` points at `len` ids, valid until the next call into the
    /// library on this thread.
    pub struct GnoblinStdMembers {
        /// First id, or null when `len` is zero.
        pub ptr: *const u64,
        /// Number of ids.
        pub len: usize,
    }

    /// Where one client goes: what a layout's `compute` hands to `GnoblinStdLayoutContext.emit`. A client the layout gives no
    /// placement is placed with the defaults (not tiled, shown, no depth, no rotation, unit scale).
    pub struct GnoblinStdPlacement {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// `STD_PLACEMENT_*` flags.
        pub flags: u32,
        /// The client this placement is for: one of the context's children.
        pub node: GnoblinNode,
        /// The client's rectangle (with `STD_PLACEMENT_BOUNDS`).
        pub bounds: GnoblinStdRect,
        /// Depth along the viewing axis; negative is in front of the camera.
        pub z: f64,
        /// Rotation about the x axis in radians.
        pub rotation_x: f64,
        /// Rotation about the y axis in radians.
        pub rotation_y: f64,
        /// Rotation about the z axis in radians.
        pub rotation_z: f64,
        /// Scale along x; zero reads as the default 1.
        pub scale_x: f64,
        /// Scale along y; zero reads as the default 1.
        pub scale_y: f64,
        /// Scale along z; zero reads as the default 1.
        pub scale_z: f64,
        /// With `STD_PLACEMENT_OFFSET`: how far the rectangle moves along x.
        pub offset_x: f64,
        /// With `STD_PLACEMENT_OFFSET`: how far the rectangle moves along y.
        pub offset_y: f64,
    }

    /// What a layout is told when it computes or hears a hook. Valid until the call returns. It is a copy: a layout can keep
    /// what it reads and nothing it does to it reaches the workspace.
    pub struct GnoblinStdLayoutContext {
        /// Size of this struct, set by the library.
        pub size: u32,
        /// Zero.
        pub reserved: u32,
        /// The workspace the layout belongs to.
        pub workspace: GnoblinStdWorkspace,
        /// The clients to place, in the workspace's order: the tiled ones (a floating, fullscreen or maximised client, and a
        /// dialog, keep their own place).
        pub children: *const GnoblinNode,
        /// Number of children.
        pub n_children: usize,
        /// The workspace's rectangle. A tiling layout insets it by `gap_outer` itself.
        pub area: GnoblinStdRect,
        /// The workspace's gap between tiles.
        pub gap_inner: f64,
        /// The workspace's gap between the tiles and the edge of the area.
        pub gap_outer: f64,
        /// The child that has the keyboard, or zero.
        pub focused: GnoblinNode,
        /// Pass this to `emit`.
        pub sink: *mut c_void,
        /// Gives one placement to the library. Valid for the `compute` call only.
        pub emit: GnoblinStdEmitFn,
    }

    /// The functions of a layout the shell implements. Every function gets the layout's `user` first.
    pub struct GnoblinStdLayoutVtable {
        /// Size of this struct, set by the shell.
        pub size: u32,
        /// Zero.
        pub reserved: u32,
        /// Places the workspace's tiled clients. Pure: the same context and the same options give the same placements.
        pub compute: GnoblinStdComputeFn,
        /// A client joined the workspace.
        pub client_added: Option<GnoblinStdClientHookFn>,
        /// A client left the workspace.
        pub client_removed: Option<GnoblinStdClientHookFn>,
        /// The workspace's area changed.
        pub area_changed: Option<GnoblinStdAreaHookFn>,
        /// Two clients traded places in the workspace's order.
        pub clients_swapped: Option<GnoblinStdSwapHookFn>,
        /// The layout's revision: absent means the layout never changes by itself and the shell calls `workspace_invalidate`.
        pub revision: Option<GnoblinStdRevisionFn>,
        /// The divider a drag that pressed a client at a point would move. Absent: the layout has no dividers.
        pub find_split: Option<GnoblinStdFindSplitFn>,
        /// Moves the dividers `find_split` returned.
        pub set_split: Option<GnoblinStdSetSplitFn>,
        /// Puts the dividers `find_split` returned back as they were.
        pub reset_split: Option<GnoblinStdResetSplitFn>,
    }

    /// Which layout a workspace has: the argument of `workspace_set_layout`. A struct that starts with `size` may grow.
    pub struct GnoblinStdLayoutSpec {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// `STD_LAYOUT_FLOATING` or `STD_LAYOUT_FOREIGN`.
        pub kind: u32,
        /// For the floating layout: `STD_PLACE_*`.
        pub placement: u32,
        /// Zero.
        pub reserved: u32,
        /// For the floating layout: the distance between cascade positions in logical pixels; zero reads as 32.
        pub cascade_step: f64,
        /// For a foreign layout: its functions. The library copies the table.
        pub vtable: *const GnoblinStdLayoutVtable,
        /// For a foreign layout: the first argument of every function. The library calls `drop_user` once when the layout is
        /// replaced or the generation retires; on a failure the library has not adopted `user`.
        pub user: *mut c_void,
        /// Frees `user`.
        pub drop_user: Option<GnoblinDropFn>,
    }

    /// A workspace to make: the argument of `workspace_new`. A struct that starts with `size` may grow.
    pub struct GnoblinStdWorkspaceSpec {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// Zero.
        pub reserved: u32,
        /// The workspace's id, unique among the added workspaces. Required.
        pub id: GnoblinStr,
        /// The name people see; empty reads as the id.
        pub name: GnoblinStr,
        /// The gap between tiles.
        pub gap_inner: f64,
        /// The gap between the tiles and the edge of the area.
        pub gap_outer: f64,
        /// The workspace's layout, or null for the default floating layout.
        pub layout: *const GnoblinStdLayoutSpec,
    }

    /// What a workspace is now: filled by `workspace_info`.
    pub struct GnoblinStdWorkspaceInfo {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// Nonzero while the workspace is in the added list.
        pub attached: u32,
        /// The id. Valid until the next call into the library on this thread.
        pub id: GnoblinStr,
        /// The name. Valid until the next call into the library on this thread.
        pub name: GnoblinStr,
        /// The position among the added workspaces, or -1.
        pub index: i64,
        /// Nonzero while an output shows the workspace.
        pub active: u32,
        /// Zero.
        pub reserved: u32,
        /// The output that shows it, or zero.
        pub output: GnoblinNode,
        /// The gap between tiles.
        pub gap_inner: f64,
        /// The gap between the tiles and the edge of the area.
        pub gap_outer: f64,
    }

    /// How a workspace is shown: the argument of `workspace_show`. A struct that starts with `size` may grow.
    pub struct GnoblinStdShow {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// Zero.
        pub reserved: u32,
        /// Milliseconds the slide between the two workspaces takes; zero switches at once.
        pub duration_ms: f64,
        /// How much of the slide a touchpad gesture has made already, 0 to 1.
        pub progress: f64,
        /// The output to show it on, or zero for the one under the pointer (the primary one when no pointer says).
        pub output: GnoblinNode,
    }

    /// How the outputs share workspaces: the argument of `policy_outputs`. A struct that starts with `size` may grow.
    pub struct GnoblinStdOutputPolicy {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// `STD_OUTPUTS_SEPARATE`, `STD_OUTPUTS_DUPLICATE` or `STD_OUTPUTS_EXTEND`.
        pub mode: u32,
        /// The output the others copy or the span starts from; zero for the primary output.
        pub lead: GnoblinNode,
    }

    /// Where workspaces come from: the argument of `policy_workspaces`. A struct that starts with `size` may grow.
    pub struct GnoblinStdWorkspacePolicy {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// `STD_WORKSPACES_MANUAL` or `STD_WORKSPACES_PER_OUTPUT`.
        pub mode: u32,
        /// For per-output: the workspaces are named `<prefix><n>`, `n` counting from 1. Empty reads as `workspace-`.
        pub name_prefix: GnoblinStr,
        /// For per-output: the gap between tiles of every workspace it makes.
        pub gap_inner: f64,
        /// For per-output: the gap between the tiles and the edge of the area.
        pub gap_outer: f64,
        /// For per-output: the layout of every workspace it makes (a foreign layout is refused: it has one `user`), or null for
        /// floating.
        pub layout: *const GnoblinStdLayoutSpec,
    }

    /// Counters the library keeps. Filled by `stats`.
    pub struct GnoblinStdStats {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// Zero.
        pub reserved: u32,
        /// Workspaces that exist.
        pub workspaces: u64,
        /// Clients the library knows.
        pub clients: u64,
        /// Collections that exist.
        pub collections: u64,
        /// Listeners the library holds.
        pub registrations: u64,
        /// Layout passes that ran.
        pub layout_passes: u64,
        /// Layout `compute` calls that ran.
        pub computes: u64,
    }

    /// The versioned function table of `libgnoblin-std`. `size` is the byte size of the table the library filled in; a field is
    /// present when `offset_of(field) + size_of(field) <= size`. Fields are appended in later minors, never moved, retyped or
    /// removed within a major. Every function takes the core's generation context first and runs on the shell thread.
    pub struct GnoblinStdApi {
        /// Byte size of this table.
        pub size: u32,
        /// ABI major of the std library.
        pub major: u32,
        /// ABI minor of the std library.
        pub minor: u32,
        /// Zero.
        pub reserved: u32,

        /// Attaches the std library to the generation: attaches to the tree, takes stock of the windows and outputs that exist,
        /// and starts watching them. Idempotent. Everything the generation then does through this table ends with it.
        pub open: unsafe extern "C" fn(ctx: GnoblinCtx) -> i32,
        /// Fills the library's counters for the generation.
        pub stats: unsafe extern "C" fn(ctx: GnoblinCtx, out: *mut GnoblinStdStats) -> i32,
        /// Removes a listener; its `drop_user` runs now, or after the running callback returns.
        pub remove: unsafe extern "C" fn(reg: GnoblinStdReg) -> i32,

        /// Writes the root collection of `kind` (`STD_COLLECTION_*`) to `out`. The same handle every time.
        pub collection_root: unsafe extern "C" fn(ctx: GnoblinCtx, kind: u32, out: *mut GnoblinStdCollection) -> i32,
        /// Makes the live selection of `parent` that holds the members whose properties match `terms`, and writes it to `out`.
        /// An unknown key is `E_NO_SUCH_MEMBER`.
        pub collection_where: unsafe extern "C" fn(ctx: GnoblinCtx, parent: GnoblinStdCollection, terms: *const GnoblinStdQueryTerm, n_terms: usize, out: *mut GnoblinStdCollection) -> i32,
        /// Writes the kind of members the collection holds (`STD_COLLECTION_*`).
        pub collection_kind: unsafe extern "C" fn(ctx: GnoblinCtx, collection: GnoblinStdCollection, out: *mut u32) -> i32,
        /// Writes the number of members.
        pub collection_len: unsafe extern "C" fn(ctx: GnoblinCtx, collection: GnoblinStdCollection, out: *mut usize) -> i32,
        /// Writes the members, in order, to `out`. The array is valid until the next call into the library on this thread.
        pub collection_members: unsafe extern "C" fn(ctx: GnoblinCtx, collection: GnoblinStdCollection, out: *mut GnoblinStdMembers) -> i32,
        /// 1 when `member` is in the collection now, 0 when it is not, or a negative error.
        pub collection_contains: unsafe extern "C" fn(ctx: GnoblinCtx, collection: GnoblinStdCollection, member: u64) -> i32,
        /// Calls `cb` for `event` of the collection (`STD_EVENT_*`): `STD_EVENT_ADDED` and `STD_EVENT_REMOVED` for its
        /// membership, any other for the members it holds. Zero on failure, with `user` not adopted. Never called inside this call.
        pub collection_listen: unsafe extern "C" fn(ctx: GnoblinCtx, collection: GnoblinStdCollection, event: u32, flags: u32, cb: GnoblinStdEventFn, user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> GnoblinStdReg,
        /// Adds `member` to a collection the shell may change (a workspace's clients, or the workspaces): on a workspace's
        /// clients this moves the client from its workspace. `E_REFUSED` for any other collection.
        pub collection_add: unsafe extern "C" fn(ctx: GnoblinCtx, collection: GnoblinStdCollection, member: u64) -> i32,
        /// Removes `member`: 1 when it was removed, 0 when it was absent or refused (the last workspace stays).
        pub collection_remove: unsafe extern "C" fn(ctx: GnoblinCtx, collection: GnoblinStdCollection, member: u64) -> i32,
        /// Removes every member without closing any window.
        pub collection_clear: unsafe extern "C" fn(ctx: GnoblinCtx, collection: GnoblinStdCollection) -> i32,
        /// Trades the places of two members in the collection's order. For a workspace's clients its layout hears of it.
        pub collection_swap: unsafe extern "C" fn(ctx: GnoblinCtx, collection: GnoblinStdCollection, a: u64, b: u64) -> i32,

        /// Writes the workspace a client is on, or zero.
        pub client_workspace: unsafe extern "C" fn(ctx: GnoblinCtx, client: GnoblinNode, out: *mut GnoblinStdWorkspace) -> i32,
        /// Moves a client to a workspace: 1 when it moved, 0 when it was refused (the client is gone, fullscreen, a dialog,
        /// already there, or the workspace is not added).
        pub client_move_to_workspace: unsafe extern "C" fn(ctx: GnoblinCtx, client: GnoblinNode, workspace: GnoblinStdWorkspace) -> i32,
        /// Marks a client floating: a layout leaves it where it is.
        pub client_set_floating: unsafe extern "C" fn(ctx: GnoblinCtx, client: GnoblinNode, floating: u32) -> i32,
        /// 1 when the shell marked the client floating.
        pub client_is_floating: unsafe extern "C" fn(ctx: GnoblinCtx, client: GnoblinNode) -> i32,

        /// Makes a workspace and writes its handle to `out`. It is not in the list of added workspaces until `workspace_add`.
        pub workspace_new: unsafe extern "C" fn(ctx: GnoblinCtx, spec: *const GnoblinStdWorkspaceSpec, out: *mut GnoblinStdWorkspace) -> i32,
        /// Writes the added workspace with this id to `out`; `E_NO_SUCH_MEMBER` when there is none.
        pub workspace_by_id: unsafe extern "C" fn(ctx: GnoblinCtx, id: *const u8, len: usize, out: *mut GnoblinStdWorkspace) -> i32,
        /// Fills `out` with what the workspace is now.
        pub workspace_info: unsafe extern "C" fn(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace, out: *mut GnoblinStdWorkspaceInfo) -> i32,
        /// Adds the workspace to the list and gives it the windows that wait for one. `E_BUSY` when a workspace with its id is
        /// there already.
        pub workspace_add: unsafe extern "C" fn(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace) -> i32,
        /// Takes the workspace out of the list: 1 when it left, 0 when it was refused (the last workspace stays). Its windows
        /// move to the workspace before it (after it, for the first) and none is closed.
        pub workspace_remove: unsafe extern "C" fn(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace) -> i32,
        /// Sets the name people see.
        pub workspace_set_name: unsafe extern "C" fn(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace, name: *const u8, len: usize) -> i32,
        /// Sets the gaps; the workspace computes again.
        pub workspace_set_gaps: unsafe extern "C" fn(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace, inner: f64, outer: f64) -> i32,
        /// Gives the workspace another layout. The windows stay where they are until it computes.
        pub workspace_set_layout: unsafe extern "C" fn(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace, spec: *const GnoblinStdLayoutSpec) -> i32,
        /// Tells the workspace that its layout changed in a way its revision did not show: it computes again.
        pub workspace_invalidate: unsafe extern "C" fn(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace) -> i32,
        /// Writes the collection of the workspace's clients, which the shell may change.
        pub workspace_clients: unsafe extern "C" fn(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace, out: *mut GnoblinStdCollection) -> i32,
        /// Shows the workspace on an output, swapping it with the one that output showed, and slides the windows when
        /// `duration_ms` asks. `E_REFUSED` when the workspace is not added or no output can show it.
        pub workspace_show: unsafe extern "C" fn(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace, show: *const GnoblinStdShow) -> i32,
        /// Moves the keyboard `step` windows along the workspace's windows: nothing moves past either end.
        pub workspace_walk: unsafe extern "C" fn(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace, step: i64) -> i32,

        /// Sets how the outputs share workspaces.
        pub policy_outputs: unsafe extern "C" fn(ctx: GnoblinCtx, policy: *const GnoblinStdOutputPolicy) -> i32,
        /// Sets where workspaces come from.
        pub policy_workspaces: unsafe extern "C" fn(ctx: GnoblinCtx, policy: *const GnoblinStdWorkspacePolicy) -> i32,
        /// Writes the workspace an output shows, or zero.
        pub output_workspace: unsafe extern "C" fn(ctx: GnoblinCtx, output: GnoblinNode, out: *mut GnoblinStdWorkspace) -> i32,

        /// Writes what the library remembers about a client: frame inset, size limits, floating rectangle and snap.
        pub client_state: unsafe extern "C" fn(ctx: GnoblinCtx, client: GnoblinNode, out: *mut GnoblinStdClientState) -> i32,
        /// Sets the frame inset, the size limits and the floating rectangle of a client. A change of the inset makes the client's
        /// workspace compute again.
        pub client_set_state: unsafe extern "C" fn(ctx: GnoblinCtx, client: GnoblinNode, state: *const GnoblinStdClientState) -> i32,
        /// Tiles a client to `zone` (`STD_SNAP_*`) of `area`, less its frame inset: 1 when it snapped, 0 when it was refused
        /// (`edge_tiling` is zero, or the zone does not tile). A maximised client floats first; a client keeps the rectangle it
        /// first floated at while it stays snapped.
        pub client_snap: unsafe extern "C" fn(ctx: GnoblinCtx, client: GnoblinNode, zone: u32, area: *const GnoblinStdRect, edge_tiling: u32) -> i32,
        /// Puts a snapped client back where it floated: 1 when it was snapped, 0 when it was not. With `forget` the snapped state
        /// ends without moving the client.
        pub client_unsnap: unsafe extern "C" fn(ctx: GnoblinCtx, client: GnoblinNode, forget: u32) -> i32,

        /// The divider that a drag which pressed `client` at (`x`, `y`) would move, from the layout of the workspace the client is
        /// on: 1 when it wrote one to `out`, 0 when the layout has none there (or none at all).
        pub workspace_find_split: unsafe extern "C" fn(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace, client: GnoblinNode, x: f64, y: f64, out: *mut GnoblinStdSplit) -> i32,
        /// Moves the dividers of the drag `token` of the workspace's layout and makes the workspace compute again.
        pub workspace_set_split: unsafe extern "C" fn(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace, token: u64, ratio: f64, cross_ratio: f64) -> i32,
        /// Puts the dividers of the drag `token` back as they were and makes the workspace compute again.
        pub workspace_reset_split: unsafe extern "C" fn(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace, token: u64) -> i32,
    }
}

abi_exports! {
    /// `void gnoblin_std_abi_version(major, minor, patch)`: the std ABI version of the installed library. Never fails.
    StdAbiVersionFn => gnoblin_std_abi_version = unsafe extern "C" fn(major: *mut u32, minor: *mut u32, patch: *mut u32);
    /// `int32_t gnoblin_std_get_api(want_major, built_minor, out)`: the function table. A different major returns `E_ABI` with
    /// the two versions in `gnoblin_std_last_error`; a newer `built_minor` than the library's still succeeds and the table's
    /// `size` says which functions exist.
    StdGetApiFn => gnoblin_std_get_api = unsafe extern "C" fn(want_major: u32, built_minor: u32, out: *mut *const GnoblinStdApi) -> i32;
    /// `size_t gnoblin_std_last_error(buf, cap)`: copies the text of the last failure of a std call on this thread, NUL
    /// terminated and truncated to `cap`, and returns its full length.
    StdLastErrorFn => gnoblin_std_last_error = unsafe extern "C" fn(buf: *mut u8, cap: usize) -> usize;
}
