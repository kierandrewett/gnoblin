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
    /// A transition (a set of timed writes that play together), by the library's own id. Zero is nil; an id is never reused in
    /// one process.
    pub type GnoblinStdTransition = u64;
    /// The style of one scope (a collection of clients, or one client), by the library's own id. Zero is nil.
    pub type GnoblinStdStyle = u64;
    /// A built-in layout that belongs to no workspace: one made by `builtin_new`, which computes what it is asked about and keeps
    /// the state its kind keeps (a dwindle's tree). Owned by the generation that made it; `builtin_free` ends it, and the
    /// generation's retirement ends the rest. Zero is nil.
    pub type GnoblinStdBuiltin = u64;
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
    /// A pointer binding's callback: one step of a drag. For `STD_DRAG_START` the return value answers the question (1 takes the
    /// drag, anything else declines it and leaves the press a press); for the other phases it is `E_OK`, or `E_PANIC` when the SDK
    /// trampoline caught a panic. `event` is valid until the callback returns.
    pub type GnoblinStdDragFn = unsafe extern "C" fn(user: *mut c_void, event: *const GnoblinStdDragEvent) -> i32;
    /// A gesture binding's callback: one step of a touchpad gesture. For `STD_GESTURE_BEGIN` the return value answers the question
    /// (1 takes the gesture, anything else declines it); for the other phases it is `E_OK`, or `E_PANIC`. `event` is valid until the
    /// callback returns.
    pub type GnoblinStdGestureFn = unsafe extern "C" fn(user: *mut c_void, event: *const GnoblinStdGestureEvent) -> i32;
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

    /// `GnoblinStdFloatingQuery.flags`: after the spot is chosen, a window that would sit exactly on the top-left corner of another is
    /// moved back toward the area's origin one `step` at a time, so a cascade that ran out of room does not pile windows on one point.
    pub const STD_FLOAT_APART: u32 = 1;

    /// `GnoblinStdPlacement.flags`: `bounds` holds the client's rectangle; without it the client stays where it is.
    pub const STD_PLACEMENT_BOUNDS: u32 = 1;
    /// `GnoblinStdPlacement.flags`: the client is hidden (a monocle's other windows).
    pub const STD_PLACEMENT_HIDDEN: u32 = 2;
    /// `GnoblinStdPlacement.flags`: the client is in a tiled layout, so it drops the border and rounding it draws itself.
    pub const STD_PLACEMENT_TILED: u32 = 4;
    /// `GnoblinStdPlacement.flags`: `offset_x` and `offset_y` move the rectangle (or, without `bounds`, the client's own size
    /// centred in the area).
    pub const STD_PLACEMENT_OFFSET: u32 = 8;

    /// `GnoblinStdLayoutQuery.flags`: `pointer_x` and `pointer_y` hold the seat's pointer in the layout's coordinates.
    pub const STD_QUERY_POINTER: u32 = 1;
    /// `GnoblinStdLayoutQuery.flags`: `focused_width` and `focused_height` hold the size the focused client has with its frame.
    pub const STD_QUERY_FOCUSED_SIZE: u32 = 2;

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
    /// `GnoblinStdClientState.flags`: the `fitted_*` fields hold the frame inset the client was last fitted to. A shell that restores
    /// the library after a reload gives it back, so a frame that appeared across the reload is noticed.
    pub const STD_CLIENT_HAS_FITTED: u32 = 8;
    /// `GnoblinStdClientState.flags`, read only: the layout that last placed the client wants it hidden (a monocle's other windows),
    /// whether or not its workspace is shown.
    pub const STD_CLIENT_LAYOUT_HIDES: u32 = 16;

    /// `GnoblinStdOutputInfo.flags`: the output has a desk, so it can show a workspace (it is on and mirrors nothing).
    pub const STD_OUTPUT_HAS_DESK: u32 = 1;
    /// `GnoblinStdOutputInfo.flags`: the output's desk waits parked for an output to return.
    pub const STD_OUTPUT_PARKED: u32 = 2;
    /// `GnoblinStdOutputInfo.flags`: the output has a size, so `rect` and `area` hold it.
    pub const STD_OUTPUT_HAS_RECT: u32 = 4;
    /// `GnoblinStdAreas.flags`: `output` holds the output rectangle the client is on.
    pub const STD_AREAS_HAS_OUTPUT: u32 = 1;
    /// `GnoblinStdAreas.flags`: `area` holds the work area of that output.
    pub const STD_AREAS_HAS_AREA: u32 = 2;

    /// `client_snap` zone and `GnoblinStdClientState.snap_zone`: the client is not snapped.
    pub const STD_SNAP_NONE: u32 = 0;
    /// A snap zone: the left half of the work area.
    pub const STD_SNAP_LEFT: u32 = 1;
    /// A snap zone: the right half of the work area.
    pub const STD_SNAP_RIGHT: u32 = 2;
    /// A snap zone: all of the work area.
    pub const STD_SNAP_TOP: u32 = 3;
    /// `GnoblinStdEvent.kind`: a transition ended. `member` is the transition, `flag` is 1 when it did not reach its target. The
    /// listener hears it once.
    pub const STD_EVENT_TRANSITION_FINISHED: u32 = 11;

    /// `easing` argument: no easing.
    pub const STD_EASING_LINEAR: u32 = 0;
    /// `easing` argument: the CSS `ease` curve.
    pub const STD_EASING_EASE: u32 = 1;
    /// `easing` argument: the CSS `ease-in` curve.
    pub const STD_EASING_EASE_IN: u32 = 2;
    /// `easing` argument: the CSS `ease-out` curve (the default of a transition).
    pub const STD_EASING_EASE_OUT: u32 = 3;
    /// `easing` argument: the CSS `ease-in-out` curve.
    pub const STD_EASING_EASE_IN_OUT: u32 = 4;

    /// `GnoblinStdProp.kind`: nothing: a property that inherits (a style read) or is not there.
    pub const STD_PROP_UNSET: u32 = 0;
    /// `GnoblinStdProp.kind`: a number in `a`.
    pub const STD_PROP_NUMBER: u32 = 1;
    /// `GnoblinStdProp.kind`: a boolean in `a` (0 or 1).
    pub const STD_PROP_BOOL: u32 = 2;
    /// `GnoblinStdProp.kind`: three numbers in `a`, `b` and `c`.
    pub const STD_PROP_TRIPLE: u32 = 3;
    /// `GnoblinStdProp.kind`: text in `text`.
    pub const STD_PROP_TEXT: u32 = 4;
    /// `GnoblinStdProp.kind`: four corner radii in `a`, `b`, `c` and `d` (top left, top right, bottom right, bottom left).
    pub const STD_PROP_CORNERS: u32 = 5;
    /// `GnoblinStdProp.kind`: a value that is null (a style's background of none).
    pub const STD_PROP_NULL: u32 = 6;
    /// `GnoblinStdProp.kind`: a node in `node`.
    pub const STD_PROP_NODE: u32 = 7;

    /// `GnoblinStdTransitionInfo.state`: the transition has tracks that have not ended.
    pub const STD_TRANSITION_RUNNING: u32 = 0;
    /// `GnoblinStdTransitionInfo.state`: every track reached its target.
    pub const STD_TRANSITION_FINISHED: u32 = 1;
    /// `GnoblinStdTransitionInfo.state`: it was cancelled, or a newer write took every property, or it ran back to its start.
    pub const STD_TRANSITION_CANCELLED: u32 = 2;

    /// `transition_control` operation: let the tracks run on towards the target.
    pub const STD_CONTROL_PLAY: u32 = 1;
    /// `transition_control` operation: hold the tracks where they are.
    pub const STD_CONTROL_PAUSE: u32 = 2;
    /// `transition_control` operation: run the tracks back towards their start.
    pub const STD_CONTROL_REVERSE: u32 = 3;
    /// `transition_control` operation: move every track to the fraction of its duration the value gives, 0 to 1.
    pub const STD_CONTROL_SEEK: u32 = 4;
    /// `transition_control` operation: end every track at its target value now.
    pub const STD_CONTROL_CANCEL: u32 = 5;

    /// `style_scope` scope: the collection of clients whose handle is the target (the root, a workspace's clients or a selection).
    pub const STD_STYLE_SCOPE_COLLECTION: u32 = 1;
    /// `style_scope` scope: the client whose node is the target.
    pub const STD_STYLE_SCOPE_CLIENT: u32 = 2;

    /// Style property: opacity, a number from 0 to 1.
    pub const STD_STYLE_OPACITY: u32 = 1;
    /// Style property: the corner radius, a number or four corners.
    pub const STD_STYLE_CORNER_RADIUS: u32 = 2;
    /// Style property: the background colour, or null.
    pub const STD_STYLE_BACKGROUND: u32 = 3;
    /// Style property: the list of borders.
    pub const STD_STYLE_BORDERS: u32 = 4;
    /// Style property: the list of shadows.
    pub const STD_STYLE_SHADOWS: u32 = 5;
    /// Style property: the list of filters of the window's own content.
    pub const STD_STYLE_FILTER: u32 = 6;
    /// Style property: the list of filters of what is behind the window.
    pub const STD_STYLE_BACKDROP_FILTER: u32 = 7;

    /// `GnoblinStdStyleEntry.kind`: a border (`a` width, `b` offset, `color`, `inset`).
    pub const STD_ENTRY_BORDER: u32 = 1;
    /// `GnoblinStdStyleEntry.kind`: a shadow (`a` x, `b` y, `c` blur, `d` spread, `color`, `inset`).
    pub const STD_ENTRY_SHADOW: u32 = 2;
    /// `GnoblinStdStyleEntry.kind`: a blur filter (`a` radius).
    pub const STD_ENTRY_BLUR: u32 = 3;
    /// `GnoblinStdStyleEntry.kind`: a saturate filter (`a` amount).
    pub const STD_ENTRY_SATURATE: u32 = 4;
    /// `GnoblinStdStyleEntry.kind`: a brightness filter (`a` amount).
    pub const STD_ENTRY_BRIGHTNESS: u32 = 5;
    /// `GnoblinStdStyleEntry.kind`: a contrast filter (`a` amount).
    pub const STD_ENTRY_CONTRAST: u32 = 6;

    /// `scene_new` kind: a plain node.
    pub const STD_SCENE_NODE: u32 = 1;
    /// `scene_new` kind: a view, which draws a client without moving it.
    pub const STD_SCENE_VIEW: u32 = 2;
    /// `scene_new` kind: a camera that projects its children in three dimensions.
    pub const STD_SCENE_CAMERA3D: u32 = 3;
    /// `scene_new` kind: a flat camera: a node whose position, zoom and roll say where a viewport looks.
    pub const STD_SCENE_CAMERA2D: u32 = 4;

    /// `workspace_set_layout_kind` kind: the master-stack layout (`layouts::LAYOUT_KINDS`).
    pub const STD_BUILTIN_MASTER_STACK: u32 = 1;
    /// `workspace_set_layout_kind` kind: the grid.
    pub const STD_BUILTIN_GRID: u32 = 2;
    /// `workspace_set_layout_kind` kind: the monocle.
    pub const STD_BUILTIN_MONOCLE: u32 = 3;
    /// `workspace_set_layout_kind` kind: the dwindle.
    pub const STD_BUILTIN_DWINDLE: u32 = 4;
    /// `workspace_set_layout_kind` kind: the strip of columns.
    pub const STD_BUILTIN_STRIP: u32 = 5;
    /// `workspace_set_layout_kind` kind: the arc.
    pub const STD_BUILTIN_ARC: u32 = 6;
    /// `workspace_set_layout_kind` kind: the 3D carousel.
    pub const STD_BUILTIN_CAROUSEL: u32 = 7;

    /// `GnoblinStdOption.kind`: no value (an optional option that is not set, or an action that answers nothing).
    pub const STD_VALUE_NONE: u32 = 0;
    /// `GnoblinStdOption.kind`: a boolean, in `flag`.
    pub const STD_VALUE_BOOL: u32 = 1;
    /// `GnoblinStdOption.kind`: a number, in `number` (a whole number for an integer option).
    pub const STD_VALUE_NUMBER: u32 = 2;
    /// `GnoblinStdOption.kind`: the number of an enumeration member, in `number`.
    pub const STD_VALUE_ENUM: u32 = 3;
    /// `GnoblinStdOption.kind`: a window, by node id, in `node`.
    pub const STD_VALUE_NODE: u32 = 4;
    /// `GnoblinStdOption.kind`: text, in `text`.
    pub const STD_VALUE_TEXT: u32 = 5;

    /// `binding_builtin` kind: drags the window under the pointer (`layouts::BINDING_KINDS`).
    pub const STD_BINDING_MOVE: u32 = 1;
    /// `binding_builtin` kind: resizes the window under the pointer.
    pub const STD_BINDING_RESIZE: u32 = 2;
    /// `binding_builtin` kind: swaps a tile with the tile it is dropped on.
    pub const STD_BINDING_SWAP: u32 = 3;
    /// `binding_builtin` kind: drags the divider of a tiling layout.
    pub const STD_BINDING_SPLIT_RESIZE: u32 = 4;

    /// `GnoblinStdDragEvent.phase` and the callback's question: the pointer went past the threshold and the binding is asked to take
    /// the drag (answer 1 to take it, anything else declines).
    pub const STD_DRAG_START: u32 = 1;
    /// `GnoblinStdDragEvent.phase`: the pointer moved.
    pub const STD_DRAG_UPDATE: u32 = 2;
    /// `GnoblinStdDragEvent.phase`: the button was released.
    pub const STD_DRAG_END: u32 = 3;
    /// `GnoblinStdDragEvent.phase`: the compositor ended the drag, or the window under it closed. The binding undoes what it did.
    pub const STD_DRAG_CANCEL: u32 = 4;
    /// `GnoblinStdDragEvent.flags`: `local_x` and `local_y` hold the press in the window's own coordinates.
    pub const STD_DRAG_HAS_LOCAL: u32 = 1;
    /// `GnoblinStdDragEvent.flags`: `work_area` holds the rectangle windows are kept inside.
    pub const STD_DRAG_HAS_WORK_AREA: u32 = 2;
    /// `GnoblinStdDragEvent.flags`: the seat's `edge_tiling` is on.
    pub const STD_DRAG_EDGE_TILING: u32 = 4;
    /// `GnoblinStdDragEvent.constraint`: the window stays inside the work area.
    pub const STD_CONSTRAINT_WORK_AREA: u32 = 0;
    /// `GnoblinStdDragEvent.constraint`: the window may leave the work area but a strip of it stays reachable.
    pub const STD_CONSTRAINT_MINIMUM_STRIP: u32 = 1;

    /// `GnoblinStdGestureEvent.phase`: the gesture began (or, for a directed binding, went the right way): the binding is asked to
    /// take it (answer 1 to take it, anything else declines).
    pub const STD_GESTURE_BEGIN: u32 = 1;
    /// `GnoblinStdGestureEvent.phase`: the fingers moved.
    pub const STD_GESTURE_UPDATE: u32 = 2;
    /// `GnoblinStdGestureEvent.phase`: the fingers lifted.
    pub const STD_GESTURE_END: u32 = 3;
    /// `GnoblinStdGestureEvent.phase`: the compositor ended the gesture, or the window under it closed.
    pub const STD_GESTURE_CANCEL: u32 = 4;
    /// `GnoblinStdGestureEvent.kind`: fingers moving together.
    pub const STD_GESTURE_SWIPE: u32 = 1;
    /// `GnoblinStdGestureEvent.kind`: fingers moving apart or together.
    pub const STD_GESTURE_PINCH: u32 = 2;
    /// `GnoblinStdGestureEvent.flags`: `x` and `y` hold where the pointer was when the gesture began.
    pub const STD_GESTURE_HAS_POSITION: u32 = 1;
    /// A way a gesture went (`direction`, `way`): none yet.
    pub const STD_WAY_NONE: u32 = 0;
    /// A way a gesture went: a swipe to the left.
    pub const STD_WAY_LEFT: u32 = 1;
    /// A way a gesture went: a swipe to the right.
    pub const STD_WAY_RIGHT: u32 = 2;
    /// A way a gesture went: a swipe up.
    pub const STD_WAY_UP: u32 = 3;
    /// A way a gesture went: a swipe down.
    pub const STD_WAY_DOWN: u32 = 4;
    /// A way a gesture went: a pinch that closes.
    pub const STD_WAY_IN: u32 = 5;
    /// A way a gesture went: a pinch that opens.
    pub const STD_WAY_OUT: u32 = 6;

    /// `GnoblinStdDragStep.client_flags` (and a tile's `flags`): the client is fullscreen.
    pub const STD_DRAG_CLIENT_FULLSCREEN: u32 = 1;
    /// `GnoblinStdDragStep.client_flags`: the client is maximised.
    pub const STD_DRAG_CLIENT_MAXIMISED: u32 = 2;
    /// `GnoblinStdDragStep.client_flags`: a layout places the client in a tile.
    pub const STD_DRAG_CLIENT_TILED: u32 = 4;
    /// `GnoblinStdDragStep.client_flags`: the shell floated the client, so a layout leaves it where it is.
    pub const STD_DRAG_CLIENT_FLOATING: u32 = 8;
    /// `GnoblinStdDragStep.client_flags`: the client is a dialog of another window.
    pub const STD_DRAG_CLIENT_DIALOG: u32 = 16;
    /// `GnoblinStdDragStep.client_flags` (tiles): the client is minimised.
    pub const STD_DRAG_CLIENT_MINIMISED: u32 = 32;
    /// `GnoblinStdDragStep.client_flags` (tiles): the client is not shown.
    pub const STD_DRAG_CLIENT_HIDDEN: u32 = 64;
    /// `GnoblinStdDragStep.client_flags`: the client is on a workspace, whose clients are the step's tiles.
    pub const STD_DRAG_CLIENT_ON_WORKSPACE: u32 = 128;

    /// How many pixels of a window stay on screen under `STD_CONSTRAINT_MINIMUM_STRIP`.
    pub const STD_DRAG_STRIP: u32 = 75;
    /// How near an edge of the work area, in pixels, a drop must be to snap the window there.
    pub const STD_SNAP_MARGIN: u32 = 4;

    /// A side of a window a resize moves (`drag_resize`): the left side. A corner is two of them.
    pub const STD_SIDE_LEFT: u32 = 1;
    /// A side of a window a resize moves: the right side.
    pub const STD_SIDE_RIGHT: u32 = 2;
    /// A side of a window a resize moves: the top.
    pub const STD_SIDE_TOP: u32 = 4;
    /// A side of a window a resize moves: the bottom.
    pub const STD_SIDE_BOTTOM: u32 = 8;

    /// `GnoblinStdDragOutcome.flags`: the binding took the drag (`STD_DRAG_START`).
    pub const STD_OUTCOME_TOOK: u32 = 1;
    /// `GnoblinStdDragOutcome.flags`: `bounds` is the rectangle the client is to have.
    pub const STD_OUTCOME_HAS_BOUNDS: u32 = 2;
    /// `GnoblinStdDragOutcome.flags`: `floating` is the rectangle the client floats at from now on.
    pub const STD_OUTCOME_HAS_FLOATING: u32 = 4;
    /// `GnoblinStdDragOutcome.flags`: the client is no longer snapped to a zone, and stays where it is.
    pub const STD_OUTCOME_FORGET_SNAP: u32 = 8;
    /// `GnoblinStdDragOutcome.flags`: `swap_with` is the tile the dragged client trades places with.
    pub const STD_OUTCOME_HAS_SWAP: u32 = 16;
    /// `GnoblinStdDragOutcome.flags`: `ratio` is the position the divider is to have.
    pub const STD_OUTCOME_HAS_RATIO: u32 = 32;
    /// `GnoblinStdDragOutcome.flags`: `cross_ratio` is the position the second divider is to have.
    pub const STD_OUTCOME_HAS_CROSS_RATIO: u32 = 64;
    /// `GnoblinStdDragOutcome.flags`: the dividers go back to where they were when the drag began.
    pub const STD_OUTCOME_RESET_SPLIT: u32 = 128;
    /// `GnoblinStdDragOutcome.flags`: `sides` names the sides of the window the drag moves.
    pub const STD_OUTCOME_HAS_SIDES: u32 = 256;
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
        /// With `STD_CLIENT_HAS_FITTED`: the room above the content the client's frame had when the library last fitted it.
        pub fitted_top: f64,
        /// With `STD_CLIENT_HAS_FITTED`: the room to the right.
        pub fitted_right: f64,
        /// With `STD_CLIENT_HAS_FITTED`: the room below.
        pub fitted_bottom: f64,
        /// With `STD_CLIENT_HAS_FITTED`: the room to the left.
        pub fitted_left: f64,
    }

    /// What the desk knows about one output: filled by `output_info`.
    pub struct GnoblinStdOutputInfo {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// `STD_OUTPUT_*` flags.
        pub flags: u32,
        /// The workspace the output shows (the one its span shows, for an output a span covers), or zero.
        pub workspace: GnoblinStdWorkspace,
        /// With `STD_OUTPUT_HAS_RECT`: the output's rectangle in the compositor's logical pixels.
        pub rect: GnoblinStdRect,
        /// With `STD_OUTPUT_HAS_RECT`: what a layout tiles in on this output alone: its rectangle less the room the shell's bars
        /// take (`output_set_insets`).
        pub area: GnoblinStdRect,
    }

    /// The outputs a workspace is stretched over: the argument of `output_span`. A struct that starts with `size` may grow.
    pub struct GnoblinStdSpan {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// Zero.
        pub reserved: u32,
        /// The output whose desk shows the one workspace over all the members.
        pub lead: GnoblinNode,
        /// The outputs the span covers, the lead first. Fewer than two, or none, ends the lead's span.
        pub members: *const GnoblinNode,
        /// Number of members.
        pub n_members: usize,
    }

    /// The output a client is on and its work area: filled by `client_areas`.
    pub struct GnoblinStdAreas {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// `STD_AREAS_*` flags.
        pub flags: u32,
        /// With `STD_AREAS_HAS_OUTPUT`: the rectangle of the output the client is on, or the primary output's when its workspace
        /// is on none.
        pub output: GnoblinStdRect,
        /// With `STD_AREAS_HAS_AREA`: what a layout tiles in on that output.
        pub area: GnoblinStdRect,
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

    /// What a built-in layout that belongs to no workspace is asked about: the clients it is to place, where, and what the shell
    /// knows that the layout reads. A copy: the library keeps nothing of it past the call. The clients are ids the caller chooses
    /// and keeps stable for as long as the layout lives (a layout that keeps a tree of them remembers which is which).
    pub struct GnoblinStdLayoutQuery {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// `STD_QUERY_*` flags.
        pub flags: u32,
        /// The clients, in order.
        pub children: *const GnoblinNode,
        /// Number of children.
        pub n_children: usize,
        /// The rectangle the layout fills. A tiling layout insets it by `gap_outer` itself.
        pub area: GnoblinStdRect,
        /// The gap between tiles.
        pub gap_inner: f64,
        /// The gap between the tiles and the edge of the area.
        pub gap_outer: f64,
        /// The child that has the keyboard, or zero.
        pub focused: GnoblinNode,
        /// With `STD_QUERY_POINTER`: the pointer's x.
        pub pointer_x: f64,
        /// With `STD_QUERY_POINTER`: the pointer's y.
        pub pointer_y: f64,
        /// With `STD_QUERY_FOCUSED_SIZE`: the focused client's width with its frame.
        pub focused_width: f64,
        /// With `STD_QUERY_FOCUSED_SIZE`: the focused client's height with its frame.
        pub focused_height: f64,
    }

    /// What `floating_place` is asked: where a new window of a size goes among the windows that are already there. A copy: the library
    /// keeps nothing of it past the call.
    pub struct GnoblinStdFloatingQuery {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// `STD_FLOAT_*` flags.
        pub flags: u32,
        /// How the window is placed: `STD_PLACE_*`.
        pub mode: u32,
        /// The distance between cascade positions; zero reads as 32.
        pub step: f64,
        /// The window's width, frame included. Below 1 it is read as 1.
        pub width: f64,
        /// The window's height, frame included. Below 1 it is read as 1.
        pub height: f64,
        /// The rectangle the window goes in.
        pub area: GnoblinStdRect,
        /// The rectangles of the windows already there, frame included.
        pub occupied: *const GnoblinStdRect,
        /// Number of occupied rectangles.
        pub n_occupied: usize,
    }

    /// An array of placements the library lends for the call: `ptr` points at `len` placements, valid until the next call into the
    /// library on this thread.
    pub struct GnoblinStdPlacements {
        /// First placement, or null when `len` is zero.
        pub ptr: *const GnoblinStdPlacement,
        /// Number of placements.
        pub len: usize,
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

    /// One named, typed value: an option of a built-in layout, an argument of one of its actions, or what an action answers.
    /// `layouts::LAYOUT_KINDS` declares which names exist and what type each has; the library refuses any other.
    pub struct GnoblinStdOption {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// `STD_VALUE_*`: which of the fields below holds the value.
        pub kind: u32,
        /// The value of a `STD_VALUE_BOOL` option (0 or 1).
        pub flag: u32,
        /// Zero.
        pub reserved: u32,
        /// The option's or argument's name; empty for what an action answers.
        pub name: GnoblinStr,
        /// The value of a `STD_VALUE_TEXT` option.
        pub text: GnoblinStr,
        /// The value of a `STD_VALUE_NUMBER` or `STD_VALUE_ENUM` option.
        pub number: f64,
        /// The value of a `STD_VALUE_NODE` option: a window's node id.
        pub node: u64,
    }

    /// One card of the ring that `layout_ring` computes.
    pub struct GnoblinStdRingSlot {
        /// The projected rectangle.
        pub rect: GnoblinStdRect,
        /// Fade from 0.4 at the back to 1 at the front.
        pub opacity: f64,
        /// Degrees: 0 at the front, 180 at the back, positive to the right of the front card.
        pub rotation: f64,
        /// Negative in front of the camera, more so for the back of the ring.
        pub z: f64,
    }

    /// What a pointer binding is told at each step of a drag. The library runs the press, the threshold and the start-or-decline
    /// rules itself; a binding sees only the steps of a drag it took.
    pub struct GnoblinStdDragEvent {
        /// Size of this struct, set by the library.
        pub size: u32,
        /// `STD_DRAG_START`, `STD_DRAG_UPDATE`, `STD_DRAG_END` or `STD_DRAG_CANCEL`.
        pub phase: u32,
        /// `STD_DRAG_*` flags.
        pub flags: u32,
        /// `STD_CONSTRAINT_*`: the seat's rule for how far a dragged window may leave the work area.
        pub constraint: u32,
        /// The chord's button number (1 left, 2 middle, 3 right).
        pub button: i64,
        /// The window the press landed on, or zero.
        pub client: GnoblinNode,
        /// Where the pointer is. On the start, where the press was.
        pub x: f64,
        /// Where the pointer is. On the start, where the press was.
        pub y: f64,
        /// Where the press was.
        pub origin_x: f64,
        /// Where the press was.
        pub origin_y: f64,
        /// With `STD_DRAG_HAS_LOCAL`: the press in the window's own coordinates.
        pub local_x: f64,
        /// With `STD_DRAG_HAS_LOCAL`: the press in the window's own coordinates.
        pub local_y: f64,
        /// With `STD_DRAG_HAS_WORK_AREA`: the rectangle of the output the press was on.
        pub work_area: GnoblinStdRect,
    }

    /// What a gesture binding is told at each step of a touchpad gesture.
    pub struct GnoblinStdGestureEvent {
        /// Size of this struct, set by the library.
        pub size: u32,
        /// `STD_GESTURE_BEGIN`, `STD_GESTURE_UPDATE`, `STD_GESTURE_END` or `STD_GESTURE_CANCEL`.
        pub phase: u32,
        /// `STD_GESTURE_*` flags.
        pub flags: u32,
        /// `STD_GESTURE_SWIPE` or `STD_GESTURE_PINCH`.
        pub kind: u32,
        /// How many fingers, 2 to 5.
        pub fingers: u32,
        /// The way the binding's trigger names (`STD_WAY_*`), or `STD_WAY_NONE` for a trigger with no direction.
        pub direction: u32,
        /// The way the fingers have gone so far (the longer axis of a swipe; a pinch's scale from 1), or `STD_WAY_NONE` while they
        /// have gone less than 12 touchpad units or 0.1 of scale.
        pub way: u32,
        /// Zero.
        pub reserved: u32,
        /// The window under the pointer when the gesture began, or zero.
        pub client: GnoblinNode,
        /// Horizontal travel of the fingers' centre since the begin, in touchpad units.
        pub dx: f64,
        /// Vertical travel.
        pub dy: f64,
        /// A pinch's scale: 1 at the begin.
        pub scale: f64,
        /// A pinch's rotation in degrees.
        pub rotation: f64,
        /// How far a directed gesture has gone, 0 to 1: 400 units of swipe, or a scale change of 0.5, in the trigger's own
        /// direction. For a trigger with no direction, how far the fingers have gone in any way.
        pub progress: f64,
        /// With `STD_GESTURE_HAS_POSITION`: where the pointer was when the gesture began.
        pub x: f64,
        /// With `STD_GESTURE_HAS_POSITION`: where the pointer was when the gesture began.
        pub y: f64,
    }

    /// A binding of the shell's own: the argument of `binding_drag` and `binding_gesture`.
    pub struct GnoblinStdBindingSpec {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// Zero.
        pub reserved: u32,
        /// For a drag, the pointer chord (`Super+Button1`); for a gesture, `Swipe3`, `Swipe4Left`, `Pinch2In` (any case).
        pub trigger: GnoblinStr,
        /// Among the bindings of one trigger the higher priority is asked first, and one that declines leaves the drag or gesture to
        /// the next. Two bindings of one trigger and one priority are refused with `E_BUSY`.
        pub priority: f64,
        /// For a drag: how far the pointer must travel before the binding is asked to start; zero reads as 4 logical pixels.
        pub threshold: f64,
    }

    /// One tile of a workspace as a swap drag sees it: a client, what it is, and where it is.
    pub struct GnoblinStdDragTile {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// `STD_DRAG_CLIENT_*` flags of the client.
        pub flags: u32,
        /// The client, by an id the caller chooses and reads back from `GnoblinStdDragOutcome.swap_with`.
        pub id: GnoblinNode,
        /// The client's content rectangle.
        pub rect: GnoblinStdRect,
    }

    /// What `drag_step` is given for one step of a drag the caller matched: the pointer and where the press was, the window it
    /// pressed, and what the caller knows of that window and kept from the drag's start. A copy: the library keeps nothing of it
    /// past the call, and nothing of the drag between calls.
    pub struct GnoblinStdDragStep {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// `STD_DRAG_CLIENT_*` flags of the window the press landed on; read when the event names a client.
        pub client_flags: u32,
        /// The step, as the library tells it to a binding of the shell's own: its phase, the pointer, where the press was, the work
        /// area. On every step but the start the caller gives the press as it was on the start.
        pub event: *const GnoblinStdDragEvent,
        /// The pressed window's content rectangle now.
        pub bounds: GnoblinStdRect,
        /// The pressed window's content rectangle when the drag started: what an update moves it from and a cancel puts back.
        pub start: GnoblinStdRect,
        /// A resize's update: the `STD_SIDE_*` sides the drag moves, as the start's outcome gave them.
        pub sides: u32,
        /// Zero.
        pub reserved: u32,
        /// The pressed window's frame inset and size limits (`client_state` fills it); null when it has none.
        pub state: *const GnoblinStdClientState,
        /// For a split-resize drag: the divider a layout's `find_split` gave for the press, with `ratio` and `cross_ratio` as they
        /// were then; null when it gave none.
        pub split: *const GnoblinStdSplit,
        /// For a swap drag's updates and end: the tiles of the dragged window's workspace, in the workspace's order.
        pub tiles: *const GnoblinStdDragTile,
        /// Number of tiles.
        pub n_tiles: usize,
    }

    /// What a drag step asks the caller to do. A drag that moves a window gives its rectangle, one that trades places gives the
    /// tile, one that moves a divider gives its ratio: the caller writes them to the objects it holds.
    pub struct GnoblinStdDragOutcome {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// `STD_OUTCOME_*` flags: which of the fields below mean something.
        pub flags: u32,
        /// A `STD_SNAP_*` zone the dropped window is to be tiled to (`snap_tile` gives its rectangle), or `STD_SNAP_NONE`.
        pub snap_zone: u32,
        /// Zero.
        pub reserved: u32,
        /// With `STD_OUTCOME_HAS_BOUNDS`: the rectangle the window is to have.
        pub bounds: GnoblinStdRect,
        /// With `STD_OUTCOME_HAS_FLOATING`: the rectangle the window floats at from now on.
        pub floating: GnoblinStdRect,
        /// With `STD_OUTCOME_HAS_SWAP`: the tile, by the id the step gave it. On an update, the tile the pointer is over now.
        pub swap_with: GnoblinNode,
        /// With `STD_OUTCOME_HAS_SIDES`: the `STD_SIDE_*` sides a resize that started moves for the rest of the drag.
        pub sides: u32,
        /// Zero.
        pub reserved2: u32,
        /// With `STD_OUTCOME_HAS_RATIO`: the divider's position.
        pub ratio: f64,
        /// With `STD_OUTCOME_HAS_CROSS_RATIO`: the second divider's position.
        pub cross_ratio: f64,
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

    /// A value the library takes or gives for a property: the `kind` (`STD_PROP_*`) says which fields are read. Text it gives is
    /// valid until the next call into the library on this thread.
    pub struct GnoblinStdProp {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// One of the `STD_PROP_*` kinds.
        pub kind: u32,
        /// A number, a boolean (0 or 1), the first of three numbers or the first corner.
        pub a: f64,
        /// The second number or corner.
        pub b: f64,
        /// The third number or corner.
        pub c: f64,
        /// The fourth corner.
        pub d: f64,
        /// Text.
        pub text: GnoblinStr,
        /// A node.
        pub node: GnoblinNode,
    }

    /// How a transition plays: the argument of `transition_begin` and `transition_animate`. A struct that starts with `size` may
    /// grow.
    pub struct GnoblinStdTransitionOptions {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// Nonzero plays at once; zero holds the tracks at the start until `STD_CONTROL_PLAY` or `STD_CONTROL_REVERSE`.
        pub autoplay: u32,
        /// `STD_EASING_*`.
        pub easing: u32,
        /// Zero.
        pub reserved: u32,
        /// Milliseconds, 0 or more. Zero commits the change at once, still as one transaction.
        pub duration_ms: f64,
        /// Milliseconds the compositor holds the start state before the animation begins, 0 or more.
        pub delay_ms: f64,
    }

    /// What a transition is now: filled by `transition_info`.
    pub struct GnoblinStdTransitionInfo {
        /// Size of this struct, set by the caller.
        pub size: u32,
        /// `STD_TRANSITION_*`.
        pub state: u32,
        /// The transition's number, which grows with every transition of the process.
        pub id: u64,
        /// The milliseconds it takes, or 0 under reduced motion.
        pub duration_ms: f64,
        /// Where the tracks were last put, 0 to 1 (what a seek wrote).
        pub progress: f64,
        /// Nonzero while the tracks are held.
        pub paused: u32,
        /// Nonzero while the tracks run back to their start.
        pub reversed: u32,
    }

    /// One element of a style list: a border, a shadow or a filter. The `kind` (`STD_ENTRY_*`) says which fields are read.
    pub struct GnoblinStdStyleEntry {
        /// Size of this struct, set by whoever builds it.
        pub size: u32,
        /// One of the `STD_ENTRY_*` kinds.
        pub kind: u32,
        /// Nonzero for an inset border or shadow.
        pub inset: u32,
        /// Zero.
        pub reserved: u32,
        /// Border width, shadow x, filter radius or amount.
        pub a: f64,
        /// Border offset, shadow y.
        pub b: f64,
        /// Shadow blur.
        pub c: f64,
        /// Shadow spread.
        pub d: f64,
        /// The colour as `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.
        pub color: GnoblinStr,
    }

    /// An array of style entries the library lends for the call: `ptr` points at `len` entries, valid until the next call into the
    /// library on this thread.
    pub struct GnoblinStdStyleEntries {
        /// First entry, or null when `len` is zero.
        pub ptr: *const GnoblinStdStyleEntry,
        /// Number of entries.
        pub len: usize,
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
        /// Writes `easing(t)` to `out`: the CSS timing function `easing` (`STD_EASING_*`) at progress `t`.
        pub easing_sample: unsafe extern "C" fn(ctx: GnoblinCtx, easing: u32, t: f64, out: *mut f64) -> i32,
        /// Starts staging: property writes (the shell's through `transition_set`, and the library's own layout and style passes)
        /// are held until `transition_commit`. `E_BUSY` while another transition is being staged.
        pub transition_begin: unsafe extern "C" fn(ctx: GnoblinCtx, options: *const GnoblinStdTransitionOptions) -> i32,
        /// Stages a write of `key` of `node` (checked at once). Without a transition being staged the write goes straight to the tree.
        pub transition_set: unsafe extern "C" fn(ctx: GnoblinCtx, node: GnoblinNode, key: *const u8, key_len: usize, value: *const GnoblinStdProp) -> i32,
        /// Reads `key` of `node`: the staged value while a transition is staged, else the tree's.
        pub transition_get: unsafe extern "C" fn(ctx: GnoblinCtx, node: GnoblinNode, key: *const u8, key_len: usize, out: *mut GnoblinStdProp) -> i32,
        /// Runs the layout and style passes against the staged state, ends staging and sends the writes as timed writes joined to
        /// one `tween` node. Writes the transition to `out`.
        pub transition_commit: unsafe extern "C" fn(ctx: GnoblinCtx, out: *mut GnoblinStdTransition) -> i32,
        /// Ends staging and drops what was staged.
        pub transition_abort: unsafe extern "C" fn(ctx: GnoblinCtx) -> i32,
        /// One timed write of its own: writes `key` of `node` over the options' duration, as a transition of that one property.
        pub transition_animate: unsafe extern "C" fn(ctx: GnoblinCtx, node: GnoblinNode, key: *const u8, key_len: usize, value: *const GnoblinStdProp, options: *const GnoblinStdTransitionOptions, out: *mut GnoblinStdTransition) -> i32,
        /// Plays, pauses, reverses, seeks or cancels a transition (`STD_CONTROL_*`; `value` is the seek fraction). `E_BUSY` while a
        /// transition is being staged.
        pub transition_control: unsafe extern "C" fn(ctx: GnoblinCtx, transition: GnoblinStdTransition, op: u32, value: f64) -> i32,
        /// Fills `out` with what the transition is now.
        pub transition_info: unsafe extern "C" fn(ctx: GnoblinCtx, transition: GnoblinStdTransition, out: *mut GnoblinStdTransitionInfo) -> i32,
        /// Calls `cb` once with a `STD_EVENT_TRANSITION_FINISHED` event when the transition ends (at once, from the next turn, when it
        /// has ended already). Zero on failure, with `user` not adopted.
        pub transition_finished: unsafe extern "C" fn(ctx: GnoblinCtx, transition: GnoblinStdTransition, cb: GnoblinStdEventFn, user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> GnoblinStdReg,

        /// Writes the style of a scope to `out`, making it the first time it is asked for. For `STD_STYLE_SCOPE_COLLECTION` the
        /// target is a collection of clients (`E_TYPE` for any other).
        pub style_scope: unsafe extern "C" fn(ctx: GnoblinCtx, scope: u32, target: u64, out: *mut GnoblinStdStyle) -> i32,
        /// Sets a scalar style property (`STD_STYLE_OPACITY`, `STD_STYLE_CORNER_RADIUS`, `STD_STYLE_BACKGROUND`); a value of kind
        /// `STD_PROP_UNSET` takes it back to inheriting. A bad value fails with the property's path and changes nothing.
        pub style_set: unsafe extern "C" fn(ctx: GnoblinCtx, style: GnoblinStdStyle, prop: u32, value: *const GnoblinStdProp) -> i32,
        /// Replaces a list style property (borders, shadows, filter, backdrop filter) with `n` entries, checked first.
        pub style_set_list: unsafe extern "C" fn(ctx: GnoblinCtx, style: GnoblinStdStyle, prop: u32, entries: *const GnoblinStdStyleEntry, n: usize) -> i32,
        /// Takes a property back to inheriting; property zero takes every property.
        pub style_unset: unsafe extern "C" fn(ctx: GnoblinCtx, style: GnoblinStdStyle, prop: u32) -> i32,
        /// Writes what the scope itself sets of a scalar property: `STD_PROP_UNSET` when it inherits.
        pub style_get: unsafe extern "C" fn(ctx: GnoblinCtx, style: GnoblinStdStyle, prop: u32, out: *mut GnoblinStdProp) -> i32,
        /// Lends the list a scope has for a property: its own, else a copy of what it inherits.
        pub style_list: unsafe extern "C" fn(ctx: GnoblinCtx, style: GnoblinStdStyle, prop: u32, out: *mut GnoblinStdStyleEntries) -> i32,
        /// Writes the resolved value of a scalar property for a client.
        pub style_computed: unsafe extern "C" fn(ctx: GnoblinCtx, client: GnoblinNode, prop: u32, out: *mut GnoblinStdProp) -> i32,
        /// Lends the resolved list of a property for a client.
        pub style_computed_list: unsafe extern "C" fn(ctx: GnoblinCtx, client: GnoblinNode, prop: u32, out: *mut GnoblinStdStyleEntries) -> i32,
        /// Lends the name of the layer a property of a client resolved from: `default`, `global`, `workspace:<id>`,
        /// `where(<query>)` or `client`.
        pub style_origin: unsafe extern "C" fn(ctx: GnoblinCtx, client: GnoblinNode, prop: u32, out: *mut GnoblinStr) -> i32,
        /// Lends what a client's style asks for that is not drawn, one text per line, empty when all of it is.
        pub style_notes: unsafe extern "C" fn(ctx: GnoblinCtx, client: GnoblinNode, out: *mut GnoblinStr) -> i32,

        /// Makes a scene node of `kind` (`STD_SCENE_*`) and writes it to `out`. It is in no parent until `scene_insert`.
        pub scene_new: unsafe extern "C" fn(ctx: GnoblinCtx, kind: u32, out: *mut GnoblinNode) -> i32,
        /// Destroys a scene node and everything under it.
        pub scene_destroy: unsafe extern "C" fn(ctx: GnoblinCtx, node: GnoblinNode) -> i32,
        /// Puts `node` under `parent` at `index` (negative is last); a node already in the tree moves.
        pub scene_insert: unsafe extern "C" fn(ctx: GnoblinCtx, parent: GnoblinNode, node: GnoblinNode, index: i64) -> i32,
        /// Writes the root of the scene.
        pub scene_root: unsafe extern "C" fn(ctx: GnoblinCtx, out: *mut GnoblinNode) -> i32,
        /// Writes a property of a scene node, checked as the TypeScript scene checks it (finite numbers, the projection names, a
        /// zoom above zero). A camera takes `projection`, `metres`, `near`, `far`, `fov`, `ipd` and `size`; a flat camera `zoom` and
        /// `roll`; a view `source`, `camera`, `width` and `height`; every node `name`, `x`, `y`, `z`, `rotation`, `scale`,
        /// `visible`, `opacity` and `input`.
        pub scene_set: unsafe extern "C" fn(ctx: GnoblinCtx, node: GnoblinNode, key: *const u8, key_len: usize, value: *const GnoblinStdProp) -> i32,
        /// Reads a property of a scene node, including the ones the library keeps (a camera's `size`).
        pub scene_get: unsafe extern "C" fn(ctx: GnoblinCtx, node: GnoblinNode, key: *const u8, key_len: usize, out: *mut GnoblinStdProp) -> i32,

        /// Gives the workspace the built-in layout `kind` (`STD_BUILTIN_*`) with the options given; an option not given has the
        /// default `layouts::LAYOUT_KINDS` declares. Replaces the layout; its windows stay where they are until it computes.
        /// `E_NO_SUCH_MEMBER` for an unknown option, `E_TYPE` for a value of the wrong type or out of range.
        pub workspace_set_layout_kind: unsafe extern "C" fn(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace, kind: u32, options: *const GnoblinStdOption, n_options: usize) -> i32,
        /// Changes one option of the workspace's built-in layout. The tree the layout keeps is kept; the workspace computes again.
        /// `E_REFUSED` when the workspace's layout is not a built-in one.
        pub workspace_layout_set: unsafe extern "C" fn(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace, option: *const GnoblinStdOption) -> i32,
        /// Writes one option of the workspace's built-in layout to `out`.
        pub workspace_layout_get: unsafe extern "C" fn(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace, name: *const u8, len: usize, out: *mut GnoblinStdOption) -> i32,
        /// Runs an action of the workspace's built-in layout with the arguments given and writes what it answers to `out`
        /// (`STD_VALUE_NONE` when it answers nothing). The workspace computes again when the action changed the layout.
        pub workspace_layout_act: unsafe extern "C" fn(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace, action: *const u8, len: usize, args: *const GnoblinStdOption, n_args: usize, out: *mut GnoblinStdOption) -> i32,
        /// Writes the `STD_BUILTIN_*` kind of the workspace's layout, or zero when it is the floating layout or a foreign one.
        pub workspace_layout_kind: unsafe extern "C" fn(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace, out: *mut u32) -> i32,
        /// Computes the ring of `count` cards with the 1-based `focus` at the front inside `area`, with the options of the carousel
        /// kind (`fov`, `card_width`, `card_height`), and writes one slot per card to `out`, which has room for `count` slots.
        /// `E_TYPE` when `focus` is not in `1..=count`.
        pub layout_ring: unsafe extern "C" fn(ctx: GnoblinCtx, count: usize, focus: usize, area: *const GnoblinStdRect, options: *const GnoblinStdOption, n_options: usize, out: *mut GnoblinStdRingSlot) -> i32,

        /// Claims the spec's pointer chord and calls `cb` for each drag of it, as `GnoblinStdBindingSpec` and the callback describe.
        /// Several bindings may share a chord (one claim serves them): the library asks them in the order of their priority and
        /// then of their registration, and the first to take the drag has it. A drag in flight is cancelled when the window it
        /// pressed closes or the binding is removed. Zero on failure (`E_BUSY` for a chord and a priority that are taken, or a chord
        /// another shell claimed), with `user` not adopted. Never called inside this call. `remove` ends it.
        pub binding_drag: unsafe extern "C" fn(ctx: GnoblinCtx, spec: *const GnoblinStdBindingSpec, cb: GnoblinStdDragFn, user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> GnoblinStdReg,
        /// Claims the spec's touchpad gesture and calls `cb` for each gesture that matches, with the library's rules: a binding with a
        /// direction waits until the fingers have one (12 touchpad units of swipe or 0.1 of scale) and is asked only if the gesture
        /// went its way; one that declines leaves the gesture to the next. Bindings of one finger count share a claim. As for
        /// `binding_drag` otherwise.
        pub binding_gesture: unsafe extern "C" fn(ctx: GnoblinCtx, spec: *const GnoblinStdBindingSpec, cb: GnoblinStdGestureFn, user: *mut c_void, drop_user: Option<GnoblinDropFn>) -> GnoblinStdReg,
        /// Starts a pointer binding the library runs itself: `kind` is `STD_BINDING_*`, the options are those `layouts::BINDING_KINDS`
        /// declares for it (`trigger`, `priority`, and the kind's own). Zero on failure. `remove` ends it.
        pub binding_builtin: unsafe extern "C" fn(ctx: GnoblinCtx, kind: u32, options: *const GnoblinStdOption, n_options: usize) -> GnoblinStdReg,

        /// Makes the built-in layout `kind` (`STD_BUILTIN_*`) with the options given, as `workspace_set_layout_kind` does, but for no
        /// workspace, and writes its handle to `out`. `E_NO_SUCH_MEMBER` for an unknown option, `E_TYPE` for a value of the wrong
        /// type or out of range. The `builtin_*` functions work on a generation that never called `open`: the library's desktop (its
        /// default workspace, its adoption of windows, its layout pass) starts with `open`, and a shell that keeps its own workspaces
        /// asks for layouts only. The layouts go with the generation.
        pub builtin_new: unsafe extern "C" fn(ctx: GnoblinCtx, kind: u32, options: *const GnoblinStdOption, n_options: usize, out: *mut GnoblinStdBuiltin) -> i32,
        /// Frees a built-in layout, and the state its kind kept.
        pub builtin_free: unsafe extern "C" fn(ctx: GnoblinCtx, builtin: GnoblinStdBuiltin) -> i32,
        /// Changes one option of the layout. The state the layout keeps (a dwindle's tree) is kept.
        pub builtin_set: unsafe extern "C" fn(ctx: GnoblinCtx, builtin: GnoblinStdBuiltin, option: *const GnoblinStdOption) -> i32,
        /// Writes one option of the layout to `out`.
        pub builtin_get: unsafe extern "C" fn(ctx: GnoblinCtx, builtin: GnoblinStdBuiltin, name: *const u8, len: usize, out: *mut GnoblinStdOption) -> i32,
        /// Computes where the query's clients go and lends the placements, one for each client the layout places. A layout that keeps
        /// a tree adds the clients it has not seen and drops the ones it no longer is given, as a workspace's layout does.
        pub builtin_compute: unsafe extern "C" fn(ctx: GnoblinCtx, builtin: GnoblinStdBuiltin, query: *const GnoblinStdLayoutQuery, out: *mut GnoblinStdPlacements) -> i32,
        /// Runs an action of the layout with the arguments given, against the clients and the focus the query holds, and writes what
        /// it answers to `out` (`STD_VALUE_NONE` when it answers nothing). A layout that hands the keyboard on writes the client that
        /// is to get it to `focus` (zero when there is none).
        pub builtin_act: unsafe extern "C" fn(ctx: GnoblinCtx, builtin: GnoblinStdBuiltin, action: *const u8, len: usize, values: *const GnoblinStdOption, n_values: usize, query: *const GnoblinStdLayoutQuery, focus: *mut GnoblinNode, out: *mut GnoblinStdOption) -> i32,
        /// Tells the layout that `client` left the workspace it lays out. Writes the client the layout hands the keyboard to (zero
        /// when it hands it to none, or is not a layout that does).
        pub builtin_client_removed: unsafe extern "C" fn(ctx: GnoblinCtx, builtin: GnoblinStdBuiltin, query: *const GnoblinStdLayoutQuery, client: GnoblinNode, focus: *mut GnoblinNode) -> i32,
        /// Tells the layout that two clients traded places in the workspace's order.
        pub builtin_clients_swapped: unsafe extern "C" fn(ctx: GnoblinCtx, builtin: GnoblinStdBuiltin, a: GnoblinNode, b: GnoblinNode) -> i32,
        /// The divider that a drag which pressed `client` at (`x`, `y`) would move, from the layout of its last compute: 1 when it
        /// wrote one to `out`, 0 when the layout has none there (or none at all).
        pub builtin_find_split: unsafe extern "C" fn(ctx: GnoblinCtx, builtin: GnoblinStdBuiltin, client: GnoblinNode, x: f64, y: f64, out: *mut GnoblinStdSplit) -> i32,
        /// Moves the dividers of the drag that `builtin_find_split` named `token`.
        pub builtin_set_split: unsafe extern "C" fn(ctx: GnoblinCtx, builtin: GnoblinStdBuiltin, token: u64, ratio: f64, cross_ratio: f64) -> i32,
        /// Puts the dividers of the drag named `token` back as they were when `builtin_find_split` returned them.
        pub builtin_reset_split: unsafe extern "C" fn(ctx: GnoblinCtx, builtin: GnoblinStdBuiltin, token: u64) -> i32,
        /// A write of `key` of `node` was made outside the library, by a shell that keeps a mirror of the tree of its own (the
        /// TypeScript engine): a transition that played the property lets it go. A shell that makes every write through the library
        /// never needs it.
        pub transition_release: unsafe extern "C" fn(ctx: GnoblinCtx, node: GnoblinNode, key: *const u8, key_len: usize) -> i32,

        /// Gives the drag binding `kind` (`STD_BINDING_*`, with the options `layouts::BINDING_KINDS` declares for it; `trigger` and
        /// `priority` are read and ignored: the caller matches the chord) one step of a drag the caller matched itself, and writes
        /// what the binding wants done to `out`. The steps are the four a binding of the shell's own is told, in order
        /// (`STD_DRAG_START`, any number of `STD_DRAG_UPDATE`, then `STD_DRAG_END` or `STD_DRAG_CANCEL`); a start that
        /// `STD_OUTCOME_TOOK` is not set on is a binding that declined, and the caller offers the drag to the next one. The library
        /// keeps nothing between steps: the caller keeps what a step answers that a later one needs (a resize's sides) and gives
        /// the drag's start back with each. A function of its inputs; it reads nothing of the library's state and touches nothing.
        /// `E_NO_SUCH_MEMBER` for an unknown option, `E_TYPE` for a value of the wrong type or out of range. Works on a generation
        /// that never called `open`.
        pub drag_step: unsafe extern "C" fn(ctx: GnoblinCtx, kind: u32, options: *const GnoblinStdOption, n_options: usize, step: *const GnoblinStdDragStep, out: *mut GnoblinStdDragOutcome) -> i32,
        /// Writes where a window of `size` along one axis goes when it is moved to `pos` inside the area that starts at `start` and is
        /// `span` long, as the `constraint` (`STD_CONSTRAINT_*`) says: a window larger than the area goes to its start.
        pub drag_clamp: unsafe extern "C" fn(ctx: GnoblinCtx, pos: f64, size: f64, start: f64, span: f64, constraint: u32, out: *mut f64) -> i32,
        /// Writes the rectangle `start` becomes when the `sides` (`STD_SIDE_*`) of it follow the pointer by (`dx`, `dy`). The opposite
        /// side is the anchor: a rectangle held at a limit stops there and does not slide. The limits are the least and most size
        /// `limits` carries (`STD_CLIENT_HAS_MIN` and `STD_CLIENT_HAS_MAX`); a null `limits`, or one with no least size, leaves the
        /// smallest size every resize leaves (100 by 80).
        pub drag_resize: unsafe extern "C" fn(ctx: GnoblinCtx, start: *const GnoblinStdRect, sides: u32, dx: f64, dy: f64, limits: *const GnoblinStdClientState, out: *mut GnoblinStdRect) -> i32,
        /// Writes the `STD_SNAP_*` zone of the edge of `area` that the point (`x`, `y`) is within `margin` pixels of
        /// (`STD_SNAP_NONE` when it is within none). Left and right win over top at the corners.
        pub drag_zone: unsafe extern "C" fn(ctx: GnoblinCtx, x: f64, y: f64, area: *const GnoblinStdRect, margin: f64, out: *mut u32) -> i32,
        /// 1 when it wrote to `out` the part of `area` a window snapped to `zone` (`STD_SNAP_*`) fills, 0 when the zone does not tile.
        /// The halves are whole pixels.
        pub drag_tile: unsafe extern "C" fn(ctx: GnoblinCtx, zone: u32, area: *const GnoblinStdRect, out: *mut GnoblinStdRect) -> i32,
        /// Writes the way a gesture has gone (`STD_WAY_*`): for a swipe (`kind` `STD_GESTURE_SWIPE`) the direction of the longer axis
        /// of (`dx`, `dy`), a tie going to the horizontal, `STD_WAY_NONE` while the fingers have moved less than `threshold` touchpad
        /// units; for a pinch the way `scale` has gone from 1, `STD_WAY_NONE` while it is within `threshold` of 1. The distance at
        /// which a binding of the library's own decides is 12 units and 0.1.
        pub gesture_way: unsafe extern "C" fn(ctx: GnoblinCtx, kind: u32, dx: f64, dy: f64, scale: f64, threshold: f64, out: *mut u32) -> i32,
        /// Fills `out` with what the desk knows about an output: whether it can show a workspace, which one it shows, its rectangle and
        /// the area a layout tiles in on it. `E_NO_SUCH_NODE` for a node that is not an output the library tracks.
        pub output_info: unsafe extern "C" fn(ctx: GnoblinCtx, output: GnoblinNode, out: *mut GnoblinStdOutputInfo) -> i32,
        /// Sets the room the shell's own bars take from the top and the bottom of an output. A layout tiles in what is left, so
        /// every workspace computes again when it changes. `E_NO_SUCH_NODE` for a node that is not an output the library tracks.
        pub output_set_insets: unsafe extern "C" fn(ctx: GnoblinCtx, output: GnoblinNode, top: f64, bottom: f64) -> i32,
        /// Makes the desk of the span's lead show one workspace over its members (the desks of the other members show nothing of
        /// their own), or ends the lead's span. Asking for the span that holds changes nothing.
        pub output_span: unsafe extern "C" fn(ctx: GnoblinCtx, span: *const GnoblinStdSpan) -> i32,
        /// Writes the output a client is on and its work area. A client whose workspace no output shows is on the primary output;
        /// zero asks for the primary output itself.
        pub client_areas: unsafe extern "C" fn(ctx: GnoblinCtx, client: GnoblinNode, out: *mut GnoblinStdAreas) -> i32,
        /// Puts a client on a workspace as a reload gives it back: it moves like `client_move_to_workspace` (its dialogs go with it), but
        /// it is not refitted, and a layout is told it arrived only when `arrived` is nonzero. 1 when it moved, 0 when it was refused
        /// (the client is gone or already there, or the workspace is not added).
        pub workspace_restore_client: unsafe extern "C" fn(ctx: GnoblinCtx, workspace: GnoblinStdWorkspace, client: GnoblinNode, arrived: u32) -> i32,

        /// Gives a client the snapped state a reload takes back: `zone` (`STD_SNAP_*`) and the rectangle `client_unsnap` puts it back to.
        /// Nothing moves: the caller put the window where it was. `E_NO_SUCH_NODE` for a node that is not a client.
        pub client_restore_snap: unsafe extern "C" fn(ctx: GnoblinCtx, client: GnoblinNode, zone: u32, floating: *const GnoblinStdRect) -> i32,

        /// Writes to `out` the rectangle a new window goes at, as the floating layout places one (free space, centred or cascading), and
        /// with `STD_FLOAT_APART` as it keeps windows off one another's corner. Reads nothing of the library's state: it never starts
        /// the desktop. `E_TYPE` for a mode that is not `STD_PLACE_*` or a number that is not finite.
        pub floating_place: unsafe extern "C" fn(ctx: GnoblinCtx, query: *const GnoblinStdFloatingQuery, out: *mut GnoblinStdRect) -> i32,
        /// Gives the style of a `where` selection a rank: among the selections that style a client the higher rank beats the lower, and
        /// equal ranks go by the order the selections first got a style. `E_TYPE` for the style of anything but a selection.
        pub style_rank: unsafe extern "C" fn(ctx: GnoblinCtx, style: GnoblinStdStyle, rank: f64) -> i32,
        /// Writes the children of a node of the tree to `out`, in tree order, lent until the next call into the library on this thread.
        /// The library reads the tree as the shell's own writes leave it, so a node made a moment ago is there.
        pub scene_children: unsafe extern "C" fn(ctx: GnoblinCtx, node: GnoblinNode, out: *mut GnoblinStdMembers) -> i32,
        /// Writes the parent of a node to `out`, or zero for a node that has none.
        pub scene_parent: unsafe extern "C" fn(ctx: GnoblinCtx, node: GnoblinNode, out: *mut GnoblinNode) -> i32,

        /// ABI 1.1 (test builds only): milliseconds since the library was first asked for its table.
        #[cfg(feature = "next-minor")]
        pub uptime_ms: unsafe extern "C" fn() -> u64,
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
    /// `int32_t gnoblin_std_init(ctx)`: registers std's default providers (the window manager, the portal dialog UI and the
    /// configuration observer) with the core through `provider_register` on `ctx`, a core context that is either a shell's
    /// generation or one of the host's own. A provider kind another registrant already holds is left to it. Returns `E_OK`, or
    /// an error with the reason in `gnoblin_std_last_error`. Call it once per context; the providers live as long as `ctx`.
    StdInitFn => gnoblin_std_init = unsafe extern "C" fn(ctx: GnoblinCtx) -> i32;
}
