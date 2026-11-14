// XR controller bindings: the trigger grammar, the drag-binding base and the stock panel manipulations.
// They read a controller's pose and ray and write a View's placement; the drag loop that calls them is `XRRuntime`
// in `xr.ts`, which imports this module, so this one imports only the types of `xr.ts`.
// Read `xr.ts` first for the objects and their units.

import { Binding } from "gnoblin/bindings";
import type { ControllerButton, ControllerTriggerParts, DragEvent } from "gnoblin/bindings";
import { Transform3D, Vector3 } from "gnoblin/scene";
import type { Node, Plane, Ray, View } from "gnoblin/scene";
import type { XRClient, XRController, XRRoom } from "gnoblin/xr";

// ---- Triggers -----------------------------------------------------------------------------------------------------

const BUTTON_ORDER: readonly ControllerButton[] = ["Trigger", "Grip", "Thumbstick", "Primary", "Secondary"];
const BUTTON_NAMES: Readonly<Record<string, ControllerButton>> = {
    trigger: "Trigger",
    grip: "Grip",
    thumbstick: "Thumbstick",
    primary: "Primary",
    secondary: "Secondary",
};

/**
 * Parses an XR trigger: `Grip+Drag`, `Right.Grip+Drag`, `Trigger+Grip+Drag`. The buttons are Trigger, Grip,
 * Thumbstick, Primary and Secondary, optionally prefixed `Left.` or `Right.` (one hand for the whole trigger), and
 * the trigger ends in `Drag`. It throws a `TypeError` that names the wrong token.
 */
export function parseControllerTrigger(trigger: unknown, what = "trigger"): ControllerTriggerParts {
    if (typeof trigger !== "string" || trigger === "") {
        throw new TypeError(`${what} must be an XR trigger such as "Grip+Drag", got ${String(trigger)}`);
    }
    const parts = trigger.split("+");
    const last = parts[parts.length - 1];
    if (last.toLowerCase() !== "drag") {
        throw new TypeError(`${what}: an XR trigger ends in Drag (token "${last}" in "${trigger}")`);
    }
    const buttons = new Set<ControllerButton>();
    let hand: "Left" | "Right" | null = null;
    for (const token of parts.slice(0, -1)) {
        const dot = token.indexOf(".");
        const prefix = dot < 0 ? "" : token.slice(0, dot).toLowerCase();
        const name = (dot < 0 ? token : token.slice(dot + 1)).toLowerCase();
        if (dot >= 0) {
            if (prefix !== "left" && prefix !== "right") {
                throw new TypeError(`${what}: "${token}" names no hand; write Left. or Right. (in "${trigger}")`);
            }
            const named = prefix === "left" ? "Left" : "Right";
            if (hand !== null && hand !== named) {
                throw new TypeError(
                    `${what}: "${token}" is on the other hand; one trigger has one hand (in "${trigger}")`,
                );
            }
            hand = named;
        }
        const button = Object.hasOwn(BUTTON_NAMES, name) ? BUTTON_NAMES[name] : undefined;
        if (button === undefined) {
            throw new TypeError(
                `${what}: "${token}" is not an XR button; use Trigger, Grip, Thumbstick, Primary or Secondary (in "${trigger}")`,
            );
        }
        if (buttons.has(button)) {
            throw new TypeError(`${what}: "${token}" repeats ${button} (in "${trigger}")`);
        }
        buttons.add(button);
    }
    if (buttons.size === 0) {
        throw new TypeError(`${what}: an XR trigger names at least one button before Drag (in "${trigger}")`);
    }
    const ordered = BUTTON_ORDER.filter((button) => buttons.has(button));
    const text = `${hand === null ? "" : `${hand}.`}${ordered.join("+")}+Drag`;
    return { kind: "controller", text, modifiers: [], hand, buttons: ordered, action: "drag" };
}

// ---- Dragging a panel ---------------------------------------------------------------------------------------------

/**
 * What a hook of an XR binding is given: a `DragEvent` whose `position` fields are absent, because a controller has a
 * ray and no pointer position. `device` is null; the hand is `controller`.
 */
export interface XRDragEvent extends DragEvent {
    /** The session's input context, which is the `xr` seat. */
    readonly context: object | null;
    readonly controller: XRController;
    /** The panel under the ray when the drag started. It does not change for the drag. */
    readonly view?: View;
    readonly ray: Ray;
    /** Where the ray struck the panel's plane when the drag started, in the room's frame. */
    readonly hit?: Vector3;
    readonly hand: string;
    readonly room: XRRoom;
    /** Metres per room unit. */
    readonly unit: number;
}

/** Identity, trigger and enablement for a controller drag binding. */
export interface XRBindingOptions {
    id: string;
    trigger: string;
    enabled?: boolean;
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
export class XRControllerDragBinding extends Binding {
    static override readTrigger(trigger: unknown, what: string): ControllerTriggerParts {
        return parseControllerTrigger(trigger, what);
    }

    constructor(options: XRBindingOptions) {
        super(options);
    }

    /** The grammar's reading of `trigger`. */
    override get parsed(): ControllerTriggerParts {
        return super.parsed as ControllerTriggerParts;
    }

    override onStart(_event: XRDragEvent): object | false {
        return false;
    }

    override onUpdate(_event: XRDragEvent, _state: object): void {}

    override onEnd(_event: XRDragEvent, _state: object): void {}

    override onCancel(_event: XRDragEvent, _state: object): void {}

    /**
     * The inputs of the controller this binding takes from the window under the ray for as long as its drag runs: by
     * default the buttons of its trigger, so a `Grip+Drag` does not also right-click the panel it moves. The runtime
     * claims them as the drag starts and releases them when it ends or is cancelled, however it ends.
     */
    claims(): readonly string[] {
        return this.parsed.buttons.map((button) => button.toLowerCase());
    }
}

/** The placement of the room's parent chain above `node`, so a world pose can be turned into the node's own. */
function aboveTransform(node: Node): Transform3D {
    const above = node.parent;
    return above === null ? Transform3D.identity() : above.worldTransform;
}

/** What a drag found: the panel and its placement, which a cancel puts back. */
interface Grab {
    view: View;
    startLocal: Transform3D;
    startWorld: Transform3D;
}

/** Starts a drag of the panel under the ray, or null when the ray strikes none. */
function grab(event: XRDragEvent): Grab | null {
    const view = event.view;
    if (view === undefined) return null;
    return { view, startLocal: view.transform, startWorld: view.worldTransform };
}

/** Puts the panel at the world pose `target`; the view is in the room's space, so it goes back through its parents. */
function setWorld(view: View, target: Transform3D): void {
    view.transform = aboveTransform(view).inverse().multiply(target);
}

/** A pose that is only `pose`'s rotation: what turning the hand by it would do to a direction. */
function turnOf(pose: Transform3D): Transform3D {
    return new Transform3D(new Vector3(), pose.rotation.toArray());
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
 * The 6-DoF grab (spec snippet 067): the panel keeps its place in the hand it was grabbed in. It writes the View's
 * placement and nothing else, so the client's own rectangle on the desktop does not change.
 */
export class ViewMoveBinding extends XRControllerDragBinding {
    tracking: "controller" | "ray";
    rotation: "follow" | "keep";

    constructor(options: ViewMoveOptions = {}) {
        super({ id: options.id ?? "move", trigger: options.trigger ?? "Grip+Drag", ...pick(options) });
        this.tracking = options.tracking ?? "controller";
        this.rotation = options.rotation ?? "follow";
        if (this.tracking !== "controller" && this.tracking !== "ray") {
            throw new TypeError(`ViewMoveBinding: tracking "${String(this.tracking)}" must be "controller" or "ray"`);
        }
        if (this.rotation !== "follow" && this.rotation !== "keep") {
            throw new TypeError(`ViewMoveBinding: rotation "${String(this.rotation)}" must be "follow" or "keep"`);
        }
        if (options.distance !== undefined && options.distance !== "preserve") {
            throw new TypeError(`ViewMoveBinding: distance "${String(options.distance)}" must be "preserve"`);
        }
    }

    override onStart(event: XRDragEvent): MoveState | false {
        const view = event.view;
        if (view === undefined || event.hit === undefined || !event.controller.tracked) {
            return false;
        }
        const startWorld = view.worldTransform;
        return {
            view,
            startLocal: view.transform,
            startWorld,
            startController: event.controller.pose,
            // Measured to the point the ray struck, so the panel does not jump to the ray's end.
            distance: event.hit.distanceTo(event.ray.origin),
            offset: startWorld.position.subtract(event.hit),
        };
    }

    override onUpdate(event: XRDragEvent, state: object): void {
        const s = state as MoveState;
        let target: Transform3D;
        if (this.tracking === "controller") {
            const delta = event.controller.pose.multiply(s.startController.inverse());
            target = delta.multiply(s.startWorld);
            if (this.rotation === "keep") {
                target.rotation = s.startWorld.rotation;
            }
        } else {
            target = s.startWorld.clone();
            const direction = event.ray.direction.normalize();
            target.position = event.ray.origin.add(direction.scale(s.distance)).add(s.offset);
        }
        setWorld(s.view, target);
    }

    override onCancel(_event: XRDragEvent, state: object): void {
        const s = state as MoveState;
        s.view.transform = s.startLocal;
    }
}

function pick(options: Partial<XRBindingOptions>): Partial<XRBindingOptions> {
    const out: Partial<XRBindingOptions> = {};
    if (options.enabled !== undefined) out.enabled = options.enabled;
    if (options.priority !== undefined) out.priority = options.priority;
    return out;
}

/** Initial scale and minimum dimensions for a panel corner resize. */
export interface ViewResizeOptions extends Partial<XRBindingOptions> {
    /** How many pixels of the client one metre of the panel is, for the size asked of it on release. */
    pixelsPerMetre?: number;
    /** The smallest the panel may be made, in metres. */
    minWidth?: number;
    minHeight?: number;
    /** The corner a drag pulls: `quadrant` is the one nearest the ray where the drag started. Default `quadrant`. */
    edges?: ResizeEdges;
}

/** The corners of a panel as the signs of its local x (right) and y (up). */
const CORNERS = {
    "top-left": { x: -1, y: 1 },
    "top-right": { x: 1, y: 1 },
    "bottom-left": { x: -1, y: -1 },
    "bottom-right": { x: 1, y: -1 },
} as const;

/** `quadrant`, or one corner that every drag pulls. */
export type ResizeEdges = "quadrant" | keyof typeof CORNERS;

/** The corner names, for the message of a bad `edges`. */
const CORNER_NAMES = Object.keys(CORNERS).join(", ");

interface ResizeState extends Grab {
    plane: Plane;
    /** The corner under the ray when the drag started: -1 or 1 on each axis. */
    corner: { x: number; y: number };
    /** The opposite corner, in the panel's own pixels at the start. It stays where it is. */
    anchor: { x: number; y: number };
    /** Where the ray struck, in the panel's pixels at the start: the drag grows the panel by how far from the anchor it is. */
    first: { x: number; y: number };
    /** The panel's pixel size at the start. */
    width: number;
    height: number;
    /** The size the hand asks for, in metres. */
    wanted: { width: number; height: number };
}

/**
 * The resize of a panel by its corner (spec snippet 069). During the drag it writes the View's own size, the uniform
 * `scale` the program reads, and the position that keeps the opposite corner still, on every update. It writes
 * nothing to the client, so the client gets no configure while the hand moves. On release, if the size asked for
 * changes the aspect by more than 2% or the client's picture would be under 85% of the pixels the new size shows,
 * it asks the client for one new size, in one transaction.
 */
export class ViewResizeBinding extends XRControllerDragBinding {
    pixelsPerMetre: number;
    minWidth: number;
    minHeight: number;
    edges: ResizeEdges;

    constructor(options: ViewResizeOptions = {}) {
        super({ id: options.id ?? "resize", trigger: options.trigger ?? "Grip+Trigger+Drag", ...pick(options) });
        this.pixelsPerMetre = options.pixelsPerMetre ?? 1200;
        this.minWidth = options.minWidth ?? 0.25;
        this.minHeight = options.minHeight ?? 0.15;
        this.edges = options.edges ?? "quadrant";
        if (this.edges !== "quadrant" && !Object.hasOwn(CORNERS, this.edges)) {
            throw new TypeError(
                `ViewResizeBinding: edges "${String(this.edges)}" must be "quadrant" or one of ${CORNER_NAMES}`,
            );
        }
        for (const [name, value] of [
            ["pixelsPerMetre", this.pixelsPerMetre],
            ["minWidth", this.minWidth],
            ["minHeight", this.minHeight],
        ] as const) {
            if (!Number.isFinite(value) || value <= 0) {
                throw new RangeError(`ViewResizeBinding: ${name} must be a number above 0`);
            }
        }
    }

    override onStart(event: XRDragEvent): ResizeState | false {
        const view = event.view;
        if (view === undefined || event.hit === undefined) {
            return false;
        }
        const startWorld = view.worldTransform;
        const local = startWorld.inverse().transformPoint(event.hit);
        // The panel's local y points up, so the top corners are the ones with y above the centre.
        const fixed: { x: number; y: number } | undefined = this.edges === "quadrant" ? undefined : CORNERS[this.edges];
        const corner = fixed ?? { x: local.x >= 0 ? 1 : -1, y: local.y >= 0 ? 1 : -1 };
        const width = view.width;
        const height = view.height;
        return {
            view,
            startLocal: view.transform,
            startWorld,
            plane: startWorld.plane(),
            corner,
            anchor: { x: (-corner.x * width) / 2, y: (-corner.y * height) / 2 },
            first: { x: local.x, y: local.y },
            width,
            height,
            wanted: { width: width * startWorld.scale * event.unit, height: height * startWorld.scale * event.unit },
        };
    }

    override onUpdate(event: XRDragEvent, state: object): void {
        const s = state as ResizeState;
        const point = s.plane.intersect(event.ray);
        if (point === null) {
            return;
        }
        // The panel grows by how far the ray is from the anchor compared with where it started, so the first update
        // does not jump. A metre is `1 / (scale * unit)` of the start's pixels.
        const perMetre = 1 / (s.startWorld.scale * event.unit);
        const at = s.startWorld.inverse().transformPoint(point);
        const kx = (at.x - s.anchor.x) / (s.first.x - s.anchor.x);
        const ky = (at.y - s.anchor.y) / (s.first.y - s.anchor.y);
        const width = Math.max(s.width * kx, this.minWidth * perMetre);
        const height = Math.max(s.height * ky, this.minHeight * perMetre);
        s.wanted = { width: width / perMetre, height: height / perMetre };
        // A panel has one scale, so the preview grows evenly to the larger of the two asked-for sizes.
        const k = Math.max(width / s.width, height / s.height);
        const anchorWorld = s.startWorld.transformPoint(new Vector3(s.anchor.x, s.anchor.y, 0));
        const centre = anchorWorld.add(s.startWorld.position.subtract(anchorWorld).scale(k));
        const target = s.startWorld.clone();
        target.position = centre;
        target.scale = s.startWorld.scale * k;
        setWorld(s.view, target);
    }

    override onEnd(event: XRDragEvent, state: object): void {
        const s = state as ResizeState;
        const client = s.view.content as XRClient | null;
        if (client === null || !client.alive) {
            return;
        }
        const desired = {
            width: Math.round(s.wanted.width * this.pixelsPerMetre),
            height: Math.round(s.wanted.height * this.pixelsPerMetre),
        };
        if (desired.width < 1 || desired.height < 1) {
            return;
        }
        const aspectChanged = Math.abs(desired.width / desired.height / (client.width / client.height) - 1) > 0.02;
        const tooCoarse = client.width < desired.width * 0.85 || client.height < desired.height * 0.85;
        if (!aspectChanged && !tooCoarse) {
            return;
        }
        // One request: both sides go in the frame's one transaction, so the client gets one configure.
        client.width = desired.width;
        client.height = desired.height;
        // The picture will have `desired` pixels, and the panel keeps the metres the hand drew.
        const world = s.view.worldTransform;
        const scale = 1 / (this.pixelsPerMetre * event.unit);
        const target = world.clone();
        target.scale = scale;
        setWorld(s.view, target);
    }

    override onCancel(_event: XRDragEvent, state: object): void {
        const s = state as ResizeState;
        s.view.transform = s.startLocal;
    }
}

// ---- Turning and scaling a panel ----------------------------------------------------------------------------------

interface RotateState extends Grab {
    /** The hand's aim pose when the drag started. */
    startController: Transform3D;
}

/**
 * The turn of a panel by the hand (spec snippet 070): the panel turns about its own centre by what the hand has turned
 * since the drag began. It writes the View's rotation and nothing else.
 */
export class ViewRotateBinding extends XRControllerDragBinding {
    constructor(options: Partial<XRBindingOptions> = {}) {
        super({ id: options.id ?? "rotate", trigger: options.trigger ?? "Trigger+Thumbstick+Drag", ...pick(options) });
    }

    override onStart(event: XRDragEvent): RotateState | false {
        const grabbed = grab(event);
        if (grabbed === null || !event.controller.tracked) {
            return false;
        }
        return { ...grabbed, startController: event.controller.pose };
    }

    override onUpdate(event: XRDragEvent, state: object): void {
        const s = state as RotateState;
        const hand = turnOf(event.controller.pose.multiply(s.startController.inverse()));
        const target = s.startWorld.clone();
        target.rotation = hand.multiply(turnOf(s.startWorld)).rotation;
        setWorld(s.view, target);
    }

    override onCancel(_event: XRDragEvent, state: object): void {
        const s = state as RotateState;
        s.view.transform = s.startLocal;
    }
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
 * The scaling of a panel by the hand (spec snippet 070). The panel's scale becomes its scale at the start plus the
 * metres the hand has moved along the direction it pointed in when the drag began: pushing away grows the panel,
 * pulling back shrinks it, and the result stays between `minScale` and `maxScale`. It writes the View's scale only.
 */
export class ViewScaleBinding extends XRControllerDragBinding {
    minScale: number;
    maxScale: number;

    constructor(options: ViewScaleOptions = {}) {
        super({ id: options.id ?? "scale", trigger: options.trigger ?? "Grip+Thumbstick+Drag", ...pick(options) });
        this.minScale = options.minScale ?? 0.2;
        this.maxScale = options.maxScale ?? 3;
        for (const [name, value] of [
            ["minScale", this.minScale],
            ["maxScale", this.maxScale],
        ] as const) {
            if (!Number.isFinite(value) || value <= 0) {
                throw new RangeError(`ViewScaleBinding: ${name} must be a number above 0`);
            }
        }
        if (this.minScale > this.maxScale) {
            throw new RangeError("ViewScaleBinding: minScale must not be above maxScale");
        }
    }

    override onStart(event: XRDragEvent): ScaleState | false {
        const grabbed = grab(event);
        if (grabbed === null || !event.controller.tracked) {
            return false;
        }
        return { ...grabbed, start: event.controller.pose.position, aim: event.ray.direction.normalize() };
    }

    override onUpdate(event: XRDragEvent, state: object): void {
        const s = state as ScaleState;
        const metres = event.controller.pose.position.subtract(s.start).dot(s.aim) * event.unit;
        const target = s.startWorld.clone();
        target.scale = Math.min(this.maxScale, Math.max(this.minScale, s.startWorld.scale + metres));
        setWorld(s.view, target);
    }

    override onCancel(_event: XRDragEvent, state: object): void {
        const s = state as ScaleState;
        s.view.transform = s.startLocal;
    }
}
