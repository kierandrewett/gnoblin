import { Binding } from "gnoblin-core/bindings/binding";
import type { ControllerTriggerParts } from "gnoblin-core/bindings/trigger_types";
import type { DragEvent } from "gnoblin-core/bindings/types";
import { Transform3D } from "gnoblin-core/math/transform";
import { Vector3 } from "gnoblin-core/math/vector";
import type { Plane, Ray } from "gnoblin-core/math/vector";
import type { View } from "gnoblin-std/scene/view";
import type { XRController } from "gnoblin-std/extensions/xr/controller";
import type { XRRoom } from "gnoblin-std/extensions/xr/room";
/**
 * Parses an XR trigger: `Grip+Drag`, `Right.Grip+Drag`, `Trigger+Grip+Drag`. The buttons are Trigger, Grip,
 * Thumbstick, Primary and Secondary, optionally prefixed `Left.` or `Right.` (one hand for the whole trigger), and
 * the trigger ends in `Drag`. It throws a `TypeError` that names the wrong token.
 */
export declare function parseControllerTrigger(trigger: unknown, what?: string): ControllerTriggerParts;
/**
 * What a hook of an XR binding is given: a `DragEvent` whose `position` fields are absent, because a controller has a
 * ray and no pointer position. `device` is null; the hand is `controller`.
 */
export interface XRDragEvent extends DragEvent {
    /** The session's input context, which is the `xr` seat. */
    readonly context: object | null;
    /** Controller that started the drag. */
    readonly controller: XRController;
    /** The panel under the ray when the drag started. It does not change for the drag. */
    readonly view?: View;
    /** Controller ray in the room's coordinate space. */
    readonly ray: Ray;
    /** Where the ray struck the panel's plane when the drag started, in the room's frame. */
    readonly hit?: Vector3;
    /** Controller hand name, such as `left` or `right`. */
    readonly hand: string;
    /** XR room containing the panel. */
    readonly room: XRRoom;
    /** Metres per room unit. */
    readonly unit: number;
}
/** Identity, trigger and enablement for a controller drag binding. */
export interface XRBindingOptions {
    /** Stable binding id. */
    id: string;
    /** Controller button chord ending in `+Drag`. */
    trigger: string;
    /** Whether the binding responds to its trigger; defaults to true. */
    enabled?: boolean;
    /** Order used to resolve competing bindings. */
    priority?: number;
}
/**
 * The base of the bindings that follow a controller from the press of its buttons to their release. The hooks are
 * the `Binding` ones: `onStart` returns a state object or false, `onUpdate` runs once a frame with the latest pose,
 * and `onEnd` or `onCancel` ends the drag. A drag starts when exactly the trigger's buttons have been down together
 * for the session's settle time, so a two-button chord is not taken for its first button. The runtime pins the panel
 * under the ray while the drag runs (the ring leaves a pinned panel alone) and leaves it pinned when the drag ends; a
 * cancelled drag gives it back to the ring unless it was pinned before, so `onCancel` only has to restore its pose.
 */
export declare class XRControllerDragBinding extends Binding {
    /** Parses a trigger string and returns its controller buttons and hand. */
    static readTrigger(trigger: unknown, what: string): ControllerTriggerParts;
    constructor(options: XRBindingOptions);
    /** The grammar's reading of `trigger`. */
    get parsed(): ControllerTriggerParts;
    /** Called when the trigger starts; return false to decline the drag. */
    onStart(_event: XRDragEvent): object | false;
    /** Called on each update while the controller drag is active. */
    onUpdate(_event: XRDragEvent, _state: object): void;
    /** Called when the controller drag ends. */
    onEnd(_event: XRDragEvent, _state: object): void;
    /** Called when the controller drag is cancelled. */
    onCancel(_event: XRDragEvent, _state: object): void;
    /**
     * The inputs of the controller this binding takes from the window under the ray for as long as its drag runs: by
     * default the buttons of its trigger, so a `Grip+Drag` does not also right-click the panel it moves. The runtime
     * claims them as the drag starts and releases them when it ends or is cancelled, however it ends.
     */
    claims(): readonly string[];
}
/** What a drag found: the panel and its placement, which a cancel puts back. */
interface Grab {
    view: View;
    startLocal: Transform3D;
    startWorld: Transform3D;
}
/** Initial tracking mode and rotation behavior for a panel move. */
export interface ViewMoveOptions extends Partial<XRBindingOptions> {
    /** `controller`: the panel follows the hand (6-DoF). `ray`: the panel follows where the ray points. */
    tracking?: "controller" | "ray";
    /** `follow`: the panel turns with the hand. `keep`: it keeps its own rotation. */
    rotation?: "follow" | "keep";
    /** Only `preserve`: a panel moved by ray keeps its distance from the controller. */
    distance?: "preserve";
}
interface MoveState extends Grab {
    startController: Transform3D;
    /** Ray tracking: how far along the ray the grabbed point was, and where the panel's centre is from it. */
    distance: number;
    offset: Vector3;
}
/**
 * Moves a panel with the controller or ray while preserving its desktop window size. The default is a 6-DoF grab
 * that keeps the panel in the hand's relative position.
 */
export declare class ViewMoveBinding extends XRControllerDragBinding {
    /** Movement tracking mode, defaulting to `controller`. */
    tracking: "controller" | "ray";
    /** Whether the panel follows hand rotation; defaults to `follow`. */
    rotation: "follow" | "keep";
    constructor(options?: ViewMoveOptions);
    /** Starts the move when the ray hits a tracked controller's panel. */
    onStart(event: XRDragEvent): MoveState | false;
    /** Updates the panel pose from the controller pose or ray. */
    onUpdate(event: XRDragEvent, state: object): void;
    /** Restores the panel's starting pose after cancellation. */
    onCancel(_event: XRDragEvent, state: object): void;
}
/** Initial scale and minimum dimensions for a panel corner resize. */
export interface ViewResizeOptions extends Partial<XRBindingOptions> {
    /** How many pixels of the client one metre of the panel is, for the size asked of it on release. */
    pixelsPerMetre?: number;
    /** The smallest the panel may be made, in metres. */
    minWidth?: number;
    /** Minimum panel height in metres. */
    minHeight?: number;
    /** The corner a drag pulls: `quadrant` is the one nearest the ray where the drag started. Default `quadrant`. */
    edges?: ResizeEdges;
}
/** The corners of a panel as the signs of its local x (right) and y (up). */
declare const CORNERS: {
    readonly "top-left": {
        readonly x: -1;
        readonly y: 1;
    };
    readonly "top-right": {
        readonly x: 1;
        readonly y: 1;
    };
    readonly "bottom-left": {
        readonly x: -1;
        readonly y: -1;
    };
    readonly "bottom-right": {
        readonly x: 1;
        readonly y: -1;
    };
};
/** `quadrant`, or one corner that every drag pulls. */
export type ResizeEdges = "quadrant" | keyof typeof CORNERS;
interface ResizeState extends Grab {
    plane: Plane;
    /** The corner under the ray when the drag started: -1 or 1 on each axis. */
    corner: {
        x: number;
        y: number;
    };
    /** The opposite corner, in the panel's own pixels at the start. It stays where it is. */
    anchor: {
        x: number;
        y: number;
    };
    /**
     * Where the ray struck, in the panel's pixels at the start: the drag grows the panel by how far from the anchor
     * it is.
     */
    first: {
        x: number;
        y: number;
    };
    /** The panel's pixel size at the start. */
    width: number;
    height: number;
    /** The size the hand asks for, in metres. */
    wanted: {
        width: number;
        height: number;
    };
}
/**
 * Resizes a panel by dragging a corner. During the drag it updates the View's size, uniform scale and position while
 * keeping the opposite corner still. It does not resize the client during movement. On release, it requests one new
 * client size if the aspect ratio changes by more than 2% or the picture would use under 85% of the available pixels.
 */
export declare class ViewResizeBinding extends XRControllerDragBinding {
    /** Client pixels per metre of panel size requested at the end of a resize. */
    pixelsPerMetre: number;
    /** Minimum panel width in metres. */
    minWidth: number;
    /** Minimum panel height in metres. */
    minHeight: number;
    /** Corner selected for the resize. */
    edges: ResizeEdges;
    constructor(options?: ViewResizeOptions);
    /** Starts resizing when the ray hits the panel. */
    onStart(event: XRDragEvent): ResizeState | false;
    /** Updates the panel size while keeping the opposite corner fixed. */
    onUpdate(event: XRDragEvent, state: object): void;
    /** Requests a client resize when the final picture needs more pixels. */
    onEnd(event: XRDragEvent, state: object): void;
    /** Restores the starting pose after a cancelled resize. */
    onCancel(_event: XRDragEvent, state: object): void;
}
interface RotateState extends Grab {
    /** The hand's aim pose when the drag started. */
    startController: Transform3D;
}
/** Rotates a panel about its centre by the controller's rotation since the drag began. */
export declare class ViewRotateBinding extends XRControllerDragBinding {
    constructor(options?: Partial<XRBindingOptions>);
    /** Starts rotating when the ray hits a tracked panel. */
    onStart(event: XRDragEvent): RotateState | false;
    /** Rotates the panel by the controller's change in rotation. */
    onUpdate(event: XRDragEvent, state: object): void;
    /** Restores the starting pose after a cancelled rotation. */
    onCancel(_event: XRDragEvent, state: object): void;
}
/** Initial limits of a panel scale. */
export interface ViewScaleOptions extends Partial<XRBindingOptions> {
    /** The smallest scale a drag may leave the panel at. Default 0.2. */
    minScale?: number;
    /** The largest. Default 3. */
    maxScale?: number;
}
interface ScaleState extends Grab {
    /** The hand's position when the drag started, and the direction its ray pointed in then. */
    start: Vector3;
    aim: Vector3;
}
/**
 * Scales a panel based on hand movement along the initial pointing direction. Pushing away grows the panel; pulling
 * back shrinks it. The result stays between `minScale` and `maxScale`.
 */
export declare class ViewScaleBinding extends XRControllerDragBinding {
    /** Minimum allowed panel scale. */
    minScale: number;
    /** Maximum allowed panel scale. */
    maxScale: number;
    constructor(options?: ViewScaleOptions);
    /** Starts scaling when the ray hits a tracked panel. */
    onStart(event: XRDragEvent): ScaleState | false;
    /** Changes scale based on hand movement along the starting aim. */
    onUpdate(event: XRDragEvent, state: object): void;
    /** Restores the starting pose after a cancelled scale. */
    onCancel(_event: XRDragEvent, state: object): void;
}
export {};
