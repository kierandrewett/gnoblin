/** The `STD_COLLECTION_*` constants. */
export declare const CollectionKind: {
    readonly CLIENTS: 1;
    readonly OUTPUTS: 2;
    readonly WORKSPACES: 3;
};
export type CollectionKind = (typeof CollectionKind)[keyof typeof CollectionKind];
/** The `STD_EVENT_*` constants. */
export declare const EventKind: {
    readonly ADDED: 1;
    readonly REMOVED: 2;
    readonly TITLE_CHANGED: 3;
    readonly APP_ID_CHANGED: 4;
    readonly FOCUS_CHANGED: 5;
    readonly GEOMETRY_CHANGED: 6;
    readonly CLOSED: 7;
    readonly READY: 8;
    readonly DECORATION_CHANGED: 9;
    readonly MODE_CHANGED: 10;
    readonly TRANSITION_FINISHED: 11;
    readonly DISPLAYS: 20;
    readonly APPLIED: 21;
    readonly NOTE: 22;
};
export type EventKind = (typeof EventKind)[keyof typeof EventKind];
/** The `STD_TERM_*` constants. */
export declare const TermKind: {
    readonly TEXT: 1;
    readonly NUMBER: 2;
    readonly BOOL: 3;
};
export type TermKind = (typeof TermKind)[keyof typeof TermKind];
/** The `STD_LAYOUT_*` constants. */
export declare const LayoutKind: {
    readonly FLOATING: 1;
    readonly FOREIGN: 2;
};
export type LayoutKind = (typeof LayoutKind)[keyof typeof LayoutKind];
/** The `STD_PLACE_*` constants. */
export declare const PlaceKind: {
    readonly SMART: 0;
    readonly CENTER: 1;
    readonly CASCADE: 2;
};
export type PlaceKind = (typeof PlaceKind)[keyof typeof PlaceKind];
/** The `STD_PLACEMENT_*` constants. */
export declare const PlacementFlags: {
    readonly BOUNDS: 1;
    readonly HIDDEN: 2;
    readonly TILED: 4;
    readonly OFFSET: 8;
};
export type PlacementFlags = (typeof PlacementFlags)[keyof typeof PlacementFlags];
/** The `STD_QUERY_*` constants. */
export declare const QueryFlags: {
    readonly POINTER: 1;
    readonly FOCUSED_SIZE: 2;
};
export type QueryFlags = (typeof QueryFlags)[keyof typeof QueryFlags];
/** The `STD_OUTPUTS_*` constants. */
export declare const OutputsKind: {
    readonly SEPARATE: 0;
    readonly DUPLICATE: 1;
    readonly EXTEND: 2;
};
export type OutputsKind = (typeof OutputsKind)[keyof typeof OutputsKind];
/** The `STD_WORKSPACES_*` constants. */
export declare const WorkspacesKind: {
    readonly MANUAL: 0;
    readonly PER_OUTPUT: 1;
};
export type WorkspacesKind = (typeof WorkspacesKind)[keyof typeof WorkspacesKind];
/** The `STD_AXIS_*` constants. */
export declare const AxisKind: {
    readonly X: 0;
    readonly Y: 1;
};
export type AxisKind = (typeof AxisKind)[keyof typeof AxisKind];
/** The `STD_CLIENT_*` constants. */
export declare const ClientFlags: {
    readonly HAS_FLOATING: 1;
    readonly HAS_MIN: 2;
    readonly HAS_MAX: 4;
    readonly HAS_FITTED: 8;
    readonly LAYOUT_HIDES: 16;
};
export type ClientFlags = (typeof ClientFlags)[keyof typeof ClientFlags];
/** The `STD_OUTPUT_*` constants. */
export declare const OutputFlags: {
    readonly HAS_DESK: 1;
    readonly PARKED: 2;
    readonly HAS_RECT: 4;
};
export type OutputFlags = (typeof OutputFlags)[keyof typeof OutputFlags];
/** The `STD_AREAS_*` constants. */
export declare const AreasFlags: {
    readonly HAS_OUTPUT: 1;
    readonly HAS_AREA: 2;
};
export type AreasFlags = (typeof AreasFlags)[keyof typeof AreasFlags];
/** The `STD_SNAP_*` constants. */
export declare const SnapKind: {
    readonly NONE: 0;
    readonly LEFT: 1;
    readonly RIGHT: 2;
    readonly TOP: 3;
    readonly MARGIN: 4;
};
export type SnapKind = (typeof SnapKind)[keyof typeof SnapKind];
/** The `STD_PRESENT_*` constants. */
export declare const PresentKind: {
    readonly SINGLE: 0;
    readonly MIRROR: 1;
    readonly SPAN: 2;
};
export type PresentKind = (typeof PresentKind)[keyof typeof PresentKind];
/** The `STD_SELECT_*` constants. */
export declare const SelectKind: {
    readonly PROFILE: 1;
    readonly OUTPUT: 2;
};
export type SelectKind = (typeof SelectKind)[keyof typeof SelectKind];
/** The `STD_PROFILE_*` constants. */
export declare const ProfileFlags: {
    readonly HAS_SCALE: 1;
    readonly HAS_SUPERSAMPLE: 2;
    readonly HAS_VRR: 4;
    readonly VRR_ON: 8;
    readonly HAS_ENABLED: 16;
    readonly ENABLED_ON: 32;
};
export type ProfileFlags = (typeof ProfileFlags)[keyof typeof ProfileFlags];
/** The `STD_POSITION_*` constants. */
export declare const PositionKind: {
    readonly NONE: 0;
    readonly ABSOLUTE: 1;
    readonly RIGHT_OF: 2;
    readonly LEFT_OF: 3;
    readonly ABOVE: 4;
    readonly BELOW: 5;
};
export type PositionKind = (typeof PositionKind)[keyof typeof PositionKind];
/** The `STD_ALIGN_*` constants. */
export declare const AlignKind: {
    readonly START: 0;
    readonly CENTER: 1;
    readonly END: 2;
};
export type AlignKind = (typeof AlignKind)[keyof typeof AlignKind];
/** The `STD_DISPLAY_*` constants. */
export declare const DisplayFlags: {
    readonly IMPLICIT: 1;
    readonly LISTED: 2;
    readonly HAS_DESK: 4;
};
export type DisplayFlags = (typeof DisplayFlags)[keyof typeof DisplayFlags];
/** The `STD_EASING_*` constants. */
export declare const EasingKind: {
    readonly LINEAR: 0;
    readonly EASE: 1;
    readonly EASE_IN: 2;
    readonly EASE_OUT: 3;
    readonly EASE_IN_OUT: 4;
};
export type EasingKind = (typeof EasingKind)[keyof typeof EasingKind];
/** The `STD_PROP_*` constants. */
export declare const PropKind: {
    readonly UNSET: 0;
    readonly NUMBER: 1;
    readonly BOOL: 2;
    readonly TRIPLE: 3;
    readonly TEXT: 4;
    readonly CORNERS: 5;
    readonly NULL: 6;
    readonly NODE: 7;
};
export type PropKind = (typeof PropKind)[keyof typeof PropKind];
/** The `STD_TRANSITION_*` constants. */
export declare const TransitionKind: {
    readonly RUNNING: 0;
    readonly FINISHED: 1;
    readonly CANCELLED: 2;
};
export type TransitionKind = (typeof TransitionKind)[keyof typeof TransitionKind];
/** The `STD_CONTROL_*` constants. */
export declare const ControlKind: {
    readonly PLAY: 1;
    readonly PAUSE: 2;
    readonly REVERSE: 3;
    readonly SEEK: 4;
    readonly CANCEL: 5;
};
export type ControlKind = (typeof ControlKind)[keyof typeof ControlKind];
/** The `STD_STYLE_*` constants. */
export declare const StyleKind: {
    readonly SCOPE_COLLECTION: 1;
    readonly SCOPE_CLIENT: 2;
    readonly OPACITY: 1;
    readonly CORNER_RADIUS: 2;
    readonly BACKGROUND: 3;
    readonly BORDERS: 4;
    readonly SHADOWS: 5;
    readonly FILTER: 6;
    readonly BACKDROP_FILTER: 7;
};
export type StyleKind = (typeof StyleKind)[keyof typeof StyleKind];
/** The `STD_ENTRY_*` constants. */
export declare const EntryKind: {
    readonly BORDER: 1;
    readonly SHADOW: 2;
    readonly BLUR: 3;
    readonly SATURATE: 4;
    readonly BRIGHTNESS: 5;
    readonly CONTRAST: 6;
    readonly SHADER: 7;
};
export type EntryKind = (typeof EntryKind)[keyof typeof EntryKind];
/** The `STD_SCENE_*` constants. */
export declare const SceneKind: {
    readonly NODE: 1;
    readonly VIEW: 2;
    readonly CAMERA3D: 3;
    readonly CAMERA2D: 4;
};
export type SceneKind = (typeof SceneKind)[keyof typeof SceneKind];
/** The `STD_BUILTIN_*` constants. */
export declare const BuiltinKind: {
    readonly MASTER_STACK: 1;
    readonly GRID: 2;
    readonly MONOCLE: 3;
    readonly DWINDLE: 4;
    readonly STRIP: 5;
    readonly ARC: 6;
    readonly CAROUSEL: 7;
};
export type BuiltinKind = (typeof BuiltinKind)[keyof typeof BuiltinKind];
/** The `STD_VALUE_*` constants. */
export declare const ValueKind: {
    readonly NONE: 0;
    readonly BOOL: 1;
    readonly NUMBER: 2;
    readonly ENUM: 3;
    readonly NODE: 4;
    readonly TEXT: 5;
};
export type ValueKind = (typeof ValueKind)[keyof typeof ValueKind];
/** The `STD_BINDING_*` constants. */
export declare const BindingKind: {
    readonly MOVE: 1;
    readonly RESIZE: 2;
    readonly SWAP: 3;
    readonly SPLIT_RESIZE: 4;
};
export type BindingKind = (typeof BindingKind)[keyof typeof BindingKind];
/** The `STD_DRAG_*` constants. */
export declare const DragKind: {
    readonly START: 1;
    readonly UPDATE: 2;
    readonly END: 3;
    readonly CANCEL: 4;
    readonly HAS_LOCAL: 1;
    readonly HAS_WORK_AREA: 2;
    readonly EDGE_TILING: 4;
    readonly CLIENT_FULLSCREEN: 1;
    readonly CLIENT_MAXIMISED: 2;
    readonly CLIENT_TILED: 4;
    readonly CLIENT_FLOATING: 8;
    readonly CLIENT_DIALOG: 16;
    readonly CLIENT_MINIMISED: 32;
    readonly CLIENT_HIDDEN: 64;
    readonly CLIENT_ON_WORKSPACE: 128;
    readonly STRIP: 75;
};
export type DragKind = (typeof DragKind)[keyof typeof DragKind];
/** The `STD_CONSTRAINT_*` constants. */
export declare const ConstraintKind: {
    readonly WORK_AREA: 0;
    readonly MINIMUM_STRIP: 1;
};
export type ConstraintKind = (typeof ConstraintKind)[keyof typeof ConstraintKind];
/** The `STD_GESTURE_*` constants. */
export declare const GestureKind: {
    readonly BEGIN: 1;
    readonly UPDATE: 2;
    readonly END: 3;
    readonly CANCEL: 4;
    readonly SWIPE: 1;
    readonly PINCH: 2;
    readonly HAS_POSITION: 1;
};
export type GestureKind = (typeof GestureKind)[keyof typeof GestureKind];
/** The `STD_WAY_*` constants. */
export declare const WayKind: {
    readonly NONE: 0;
    readonly LEFT: 1;
    readonly RIGHT: 2;
    readonly UP: 3;
    readonly DOWN: 4;
    readonly IN: 5;
    readonly OUT: 6;
};
export type WayKind = (typeof WayKind)[keyof typeof WayKind];
/** The `STD_SIDE_*` constants. */
export declare const SideFlags: {
    readonly LEFT: 1;
    readonly RIGHT: 2;
    readonly TOP: 4;
    readonly BOTTOM: 8;
};
export type SideFlags = (typeof SideFlags)[keyof typeof SideFlags];
/** The `STD_OUTCOME_*` constants. */
export declare const OutcomeFlags: {
    readonly TOOK: 1;
    readonly HAS_BOUNDS: 2;
    readonly HAS_FLOATING: 4;
    readonly FORGET_SNAP: 8;
    readonly HAS_SWAP: 16;
    readonly HAS_RATIO: 32;
    readonly HAS_CROSS_RATIO: 64;
    readonly RESET_SPLIT: 128;
    readonly HAS_SIDES: 256;
};
export type OutcomeFlags = (typeof OutcomeFlags)[keyof typeof OutcomeFlags];
export declare const STD_LISTEN_REPLAY = 1;
export declare const STD_FLOAT_APART = 1;
export declare const STD_SPLIT_CROSS = 1;
