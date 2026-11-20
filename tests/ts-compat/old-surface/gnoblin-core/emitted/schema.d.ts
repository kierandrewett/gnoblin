/** A node of the tree, by id. */
export type NodeHandle = number;
/** A fingerprint of the catalog this file was generated from: `gnoblin_catalog_hash` of a core built from the same schema. */
export declare const CATALOG_HASH = "0xca6644e05cab7919";
/** One property of a class, as the schema declares it. */
export interface PropertyInfo {
    /** The schema's type: `bool`, `int`, `uint`, `double`, `string`, `bytes`, `object:<class>` or `list<...>`. */
    readonly type: string;
    readonly readonly: boolean;
    readonly animated: boolean;
    /** The value before any write, when the schema gives one. */
    readonly default?: unknown;
}
/** The properties of a `node` node. */
export interface NodeProperties {
    readonly id: number;
    readonly parent: NodeHandle | null;
    name: string;
    x: number;
    y: number;
    z: number;
    rotation: number[];
    scale: number[];
    width: number;
    height: number;
    opacity: number;
    corner_radius: number;
    visible: boolean;
    clip: boolean;
    input: boolean;
    input_mode: string;
    secure: boolean;
    tags: string[];
    effects: Record<string, unknown>[];
    readonly skipped: string[];
}
/** The methods of a `node` node. */
export type NodeMethods = Record<never, never>;
/** The signals of a `node` node: each name and its payload. */
export interface NodeSignals {
    destroyed: [];
}
/** The properties of a `camera` node. */
export interface CameraProperties extends NodeProperties {
    projection: string;
    metres: number;
    near: number;
    far: number;
    fov: number;
    ipd: number;
    fov_left: number;
    fov_right: number;
    fov_up: number;
    fov_down: number;
    readonly layer_budget: number;
}
/** The methods of a `camera` node. */
export type CameraMethods = NodeMethods;
/** The signals of a `camera` node: each name and its payload. */
export type CameraSignals = NodeSignals;
/** The properties of a `canvas` node. */
export interface CanvasProperties extends NodeProperties {
    content: Uint8Array;
    placement: string;
}
/** The methods of a `canvas` node. */
export interface CanvasMethods extends NodeMethods {
    measure(): void;
}
/** The signals of a `canvas` node: each name and its payload. */
export type CanvasSignals = NodeSignals;
/** The properties of a `controller` node. */
export interface ControllerProperties extends NodeProperties {
    readonly hand: string;
    readonly tracked: boolean;
    readonly trigger: number;
    readonly grip: number;
    readonly thumbstick_x: number;
    readonly thumbstick_y: number;
    readonly thumbstick_click: boolean;
    readonly primary: boolean;
    readonly secondary: boolean;
    claimed: string[];
}
/** The methods of a `controller` node. */
export type ControllerMethods = NodeMethods;
/** The signals of a `controller` node: each name and its payload. */
export type ControllerSignals = NodeSignals;
/** The properties of a `lock` node. */
export interface LockProperties extends NodeProperties {
    readonly state: string;
    engaged: boolean;
}
/** The methods of a `lock` node. */
export interface LockMethods extends NodeMethods {
    lock(): void;
    heartbeat(): void;
}
/** The signals of a `lock` node: each name and its payload. */
export interface LockSignals extends NodeSignals {
    requested: [];
}
/** The properties of a `seat` node. */
export interface SeatProperties extends NodeProperties {
    focus: NodeHandle | null;
    readonly screen: NodeHandle | null;
    readonly pointer_x: number;
    readonly pointer_y: number;
    readonly device: string[];
    readonly edge_tiling: boolean;
    readonly drag_constraint: string;
    readonly focus_mode: string;
    readonly focus_new_windows: string;
    readonly raise_on_click: boolean;
    readonly auto_raise: boolean;
    readonly auto_raise_delay: number;
    readonly focus_change_on_pointer_rest: boolean;
    readonly double_click: string;
    readonly middle_click: string;
    readonly right_click: string;
    readonly workspaces: string[];
    readonly reduced_motion: boolean;
    readonly effect_kinds: string[];
    readonly user_shaders: boolean;
    decorations_available: boolean;
    builtin_motion: boolean;
    readonly grabbed: string;
    cursor_shape: string;
    cursor_image: Uint8Array;
    cursor_hotspot_x: number;
    cursor_hotspot_y: number;
    cursor_hidden: boolean;
}
/** The methods of a `seat` node. */
export interface SeatMethods extends NodeMethods {
    warp(): void;
    claim(id: number, chord: string, grab: boolean, device: string): void;
    unclaim(id: number): void;
    claim_gesture(id: number, kind: string, fingers: number, device: string): void;
    claim_device(id: number, device: string): void;
    grab(id: number, node: number, scope: string): void;
    ungrab(id: number): void;
    claim_pointer(id: number, device: string): void;
}
/** The signals of a `seat` node: each name and its payload. */
export interface SeatSignals extends NodeSignals {
    chord: [id: number, chord: string, state: string];
    gesture: [id: number, kind: string, fingers: number, phase: string, dx: number, dy: number, scale: number, rotation: number];
    pointer_chord: [phase: string, claim: number, x: number, y: number, dx: number, dy: number, button: number, modifiers: string, time: number];
    pointer_button: [claim: number, button: number, state: string, x: number, y: number, time: number];
}
/** The properties of a `surface` node. */
export interface SurfaceProperties extends NodeProperties {
    readonly role: string;
    readonly mapped: boolean;
    readonly app_id: string;
    placement_override: boolean;
    stack: number;
    reserve: number[];
    readonly namespace: string;
    readonly layer: string;
    readonly anchor: string[];
    readonly margin: number[];
    readonly requested_size: number[];
    readonly exclusive_zone: number;
}
/** The methods of a `surface` node. */
export type SurfaceMethods = NodeMethods;
/** The signals of a `surface` node: each name and its payload. */
export type SurfaceSignals = NodeSignals;
/** The properties of a `tween` node. */
export interface TweenProperties extends NodeProperties {
    progress: number;
    paused: boolean;
    reverse: boolean;
}
/** The methods of a `tween` node. */
export type TweenMethods = NodeMethods;
/** The signals of a `tween` node: each name and its payload. */
export type TweenSignals = NodeSignals;
/** The properties of a `view` node. */
export interface ViewProperties extends NodeProperties {
    source: NodeHandle | null;
    fps: number;
    camera: NodeHandle | null;
}
/** The methods of a `view` node. */
export interface ViewMethods extends NodeMethods {
    capture(): void;
}
/** The signals of a `view` node: each name and its payload. */
export type ViewSignals = NodeSignals;
/** The properties of a `screen` node. */
export interface ScreenProperties extends ViewProperties {
    readonly make: string;
    readonly model: string;
    readonly screen_x: number;
    readonly screen_y: number;
    readonly screen_width: number;
    readonly screen_height: number;
    readonly output_scale: number;
    readonly transform: string;
    primary: boolean;
    readonly enabled: boolean;
    readonly mode: string;
    readonly connector: string;
    readonly identity: string;
    readonly mirror_of: string;
    readonly supersample: number;
    readonly gpu_memory_target_bytes: number;
    readonly gpu_memory_scratch_bytes: number;
    readonly gpu_memory_staging_bytes: number;
    readonly gpu_memory_views_bytes: number;
    readonly gpu_memory_capture_bytes: number;
    readonly gpu_memory_bytes: number;
    readonly gpu_memory_clients_bytes: number;
    readonly gpu_memory_budget_bytes: number;
    readonly gpu_memory_refused_total: number;
    readonly gpu_memory_evicted_total: number;
}
/** The methods of a `screen` node. */
export interface ScreenMethods extends ViewMethods {
    apply(text: string): void;
}
/** The signals of a `screen` node: each name and its payload. */
export interface ScreenSignals extends ViewSignals {
    mode_changed: [];
}
/** The properties of a `window` node. */
export interface WindowProperties extends NodeProperties {
    readonly title: string;
    readonly app_id: string;
    readonly focused: boolean;
    fullscreen: boolean;
    maximised: boolean;
    minimised: boolean;
    tiled: boolean;
    readonly unresponsive: boolean;
    readonly parent_window: NodeHandle | null;
    readonly from_launcher: boolean;
    readonly decoration: string;
    readonly decoration_wanted: string;
    frame_policy: string;
    frame_rate: number;
    report_presented: boolean;
    readonly presented: number;
}
/** The methods of a `window` node. */
export interface WindowMethods extends NodeMethods {
    focus(): void;
    close(): void;
    kill(): void;
}
/** The signals of a `window` node: each name and its payload. */
export type WindowSignals = NodeSignals;
/** Every node class by its schema name. */
export interface TreeClasses {
    node: {
        properties: NodeProperties;
        methods: NodeMethods;
        signals: NodeSignals;
    };
    camera: {
        properties: CameraProperties;
        methods: CameraMethods;
        signals: CameraSignals;
    };
    canvas: {
        properties: CanvasProperties;
        methods: CanvasMethods;
        signals: CanvasSignals;
    };
    controller: {
        properties: ControllerProperties;
        methods: ControllerMethods;
        signals: ControllerSignals;
    };
    lock: {
        properties: LockProperties;
        methods: LockMethods;
        signals: LockSignals;
    };
    seat: {
        properties: SeatProperties;
        methods: SeatMethods;
        signals: SeatSignals;
    };
    surface: {
        properties: SurfaceProperties;
        methods: SurfaceMethods;
        signals: SurfaceSignals;
    };
    tween: {
        properties: TweenProperties;
        methods: TweenMethods;
        signals: TweenSignals;
    };
    view: {
        properties: ViewProperties;
        methods: ViewMethods;
        signals: ViewSignals;
    };
    screen: {
        properties: ScreenProperties;
        methods: ScreenMethods;
        signals: ScreenSignals;
    };
    window: {
        properties: WindowProperties;
        methods: WindowMethods;
        signals: WindowSignals;
    };
}
/** The schema name of a node class. */
export type TreeClassName = keyof TreeClasses;
/** Every node class: its base and the property table, so a script can ask what a class has without a node of it. */
export declare const TREE_CLASSES: {
    readonly [Name in TreeClassName]: {
        readonly base: TreeClassName | null;
        readonly properties: {
            readonly [property: string]: PropertyInfo;
        };
    };
};
