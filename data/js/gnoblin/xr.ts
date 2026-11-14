// XR shell integration: this module owns room placement, controller bindings and session wrappers, not the XR runtime.
// The separate XR program owns tracking and layers; this module reads and writes scene nodes through `XRWorld`.
// Units are millimetres internally and metres at the script-facing layout boundary.
// Read `scene.ts` first for node and transform semantics, then this module for room behavior.

import { Binding, BindingCollection } from "gnoblin/bindings";
import type { ControllerButton, ControllerTriggerParts, DragEvent } from "gnoblin/bindings";
import { Camera3D, Node, Plane, Ray, SceneError, Transform3D, Vector3, View, mountOnRoot } from "gnoblin/scene";
import type { Camera3DOptions, NodeOptions, SceneBackend } from "gnoblin/scene";

export { Camera3D, Node, Plane, Ray, Transform3D, Vector3, View };

type NodeNumber = number;

/** What the XR module reads of a client. The shell's `Client` has all of it. */
export interface XRClient {
    readonly alive: boolean;
    readonly secure: boolean;
    readonly minimised: boolean;
    width: number;
    height: number;
}

/** The node and name of the shell's XR seat. */
export interface XRSeat {
    readonly node: NodeNumber;
    readonly name: string;
}

/** What the module needs of the shell. The host module implements it. */
export interface XRWorld {
    readonly scene: SceneBackend;
    /** Every node of a tree class (`Controller`, `Node`), parents before children. */
    nodes(cls: string): NodeNumber[];
    /** The lowercase class of a node, or undefined once it is gone. */
    kind(node: NodeNumber): string | undefined;
    seats(): XRSeat[];
    /** The client that holds the keyboard focus of the main seat. */
    focusedClient(): XRClient | null;
    clients(): XRClient[];
    /** The lowercase class of the node a client sits under, or undefined when it has none. */
    parentKind(client: XRClient): string | undefined;
    /** The `InputContext` of a seat. */
    inputContext(seat: NodeNumber): object | null;
    /** Whether the session declared XR, so the module imports before the headset's session has started. */
    declared(): boolean;
    /** Milliseconds on a clock that only goes forward. */
    now(): number;
    report(site: string, error: unknown): void;
}

// ---- The ring -----------------------------------------------------------------------------------------------------

/** The ring's numbers, in millimetres and radians. `SpatialLayout` states them in metres. */
export interface XRRingParams {
    /** The distance from the viewer to the centre of a panel. */
    radius: number;
    /** The height of a panel's centre above the room's floor. */
    eye: number;
    /** One window pixel is this many millimetres. */
    mmPerPx: number;
    /** The focused panel is this much larger ... */
    focusScale: number;
    /** ... and this much closer to the viewer. */
    focusPull: number;
    /** The angle between neighbouring panels. */
    gap: number;
    /** The most the panels reach to either side of the front: panels shrink to fit. */
    halfArc: number;
    /** Panels on the ring; the rest stack behind it. */
    maxPanels: number;
    /** Each tier of stacked panels is this much further back. */
    stackStep: number;
}

/** Default ring dimensions in millimetres and radians. */
export const RING_DEFAULTS: Readonly<XRRingParams> = Object.freeze({
    radius: 1600,
    eye: 1400,
    mmPerPx: 0.75,
    focusScale: 1.15,
    focusPull: 100,
    gap: (3 * Math.PI) / 180,
    halfArc: (150 * Math.PI) / 180,
    maxPanels: 12,
    stackStep: 250,
});

/** What one panel needs of the ring: its pixel size. `id` is anything comparable. */
export interface RingView<I> {
    readonly id: I;
    readonly width: number;
    readonly height: number;
}

/** One panel's ring position in millimetres, its scale and its stack tier. */
export interface RingPose {
    x: number;
    y: number;
    z: number;
    /** Rotation about each axis in radians; only the turn about y is used. */
    rotation: [number, number, number];
    /** Millimetres per pixel for the panel. */
    scale: number;
    focused: boolean;
    /** 0 on the ring and the tier (1, 2, ...) of a stacked panel. */
    behind: number;
}

/** Ring poses in input order and their far-to-near rendering order. */
export interface RingResult {
    /** One pose per view, in the order of the views. */
    poses: RingPose[];
    /** The view indices from the farthest to the nearest: the order the room must list the Views in. */
    order: number[];
}

/**
 * Places panels around a ring from their pixel sizes and the focused id. The focused panel is at the front, with
 * neighbours in creation order on either side (the right side holds the extra one), wrapping around; without a focus,
 * the first panel is at the front.
 */
export function arrangeRing<I>(
    views: readonly RingView<I>[],
    focus: I | null,
    params: Readonly<XRRingParams> = RING_DEFAULTS,
): RingResult {
    const n = views.length;
    const poses: RingPose[] = new Array<RingPose>(n);
    const order: number[] = [];
    if (n === 0) {
        return { poses, order };
    }
    const p = params;
    const focusAt = focus === null ? -1 : views.findIndex((view) => view.id === focus);
    const hasFocus = focusAt >= 0;
    const m = Math.min(n, p.maxPanels);
    const seq = views.map((_, at) => at);
    if (hasFocus && focusAt >= m) {
        [seq[m - 1], seq[focusAt]] = [seq[focusAt], seq[m - 1]];
    }
    let centreSlot = 0;
    if (hasFocus) {
        for (let k = 0; k < m; k++) {
            if (seq[k] === focusAt) centreSlot = k;
        }
    }
    const wrap = (k: number): number => ((k % m) + m) % m;
    const right: number[] = [];
    const left: number[] = [];
    const nr = Math.floor(m / 2);
    for (let j = 1; j <= nr; j++) right.push(seq[wrap(centreSlot + j)]);
    for (let j = 1; j <= m - 1 - nr; j++) left.push(seq[wrap(centreSlot - j)]);
    const centre = seq[centreSlot];
    const span = (i: number, focused: boolean): number =>
        ((focused ? p.focusScale : 1) * Math.max(views[i].width, 0) * p.mmPerPx) / p.radius;
    const aCentre = span(centre, hasFocus);
    // Both sides must fit in halfArc, gaps included; the panels shrink together until they do.
    let shrink = 1;
    for (const side of [right, left]) {
        let need = aCentre / 2;
        for (const i of side) need += span(i, false);
        const allowed = p.halfArc - side.length * p.gap;
        if (need > allowed) shrink = Math.min(shrink, allowed / need);
    }
    const phi = new Map<number, number>([[centre, 0]]);
    for (const [sign, side] of [
        [1, right],
        [-1, left],
    ] as const) {
        let edge = (shrink * aCentre) / 2;
        for (const i of side) {
            const half = (shrink * span(i, false)) / 2;
            const at = edge + p.gap + half;
            phi.set(i, sign * at);
            edge = at + half;
        }
    }
    const put = (i: number, angle: number, radius: number, focused: boolean, tier: number): void => {
        poses[i] = {
            x: radius * Math.sin(angle),
            y: p.eye,
            z: -radius * Math.cos(angle),
            rotation: [0, 0 - angle, 0],
            scale: shrink * (focused ? p.focusScale : 1) * p.mmPerPx,
            focused,
            behind: tier,
        };
    };
    for (let k = 0; k < m; k++) {
        const i = seq[k];
        const focused = hasFocus && i === centre;
        put(i, phi.get(i) ?? 0, focused ? p.radius - p.focusPull : p.radius, focused, 0);
    }
    // Extras sit behind the ring panel at the same angle, a tier further back for every full turn of the ring.
    const tiers: number[][] = [];
    for (let j = 1; j <= n - m; j++) {
        const tier = Math.floor((j - 1) / m) + 1;
        const i = seq[m + j - 1];
        put(i, phi.get(seq[wrap(j - 1)]) ?? 0, p.radius + p.stackStep * tier, false, tier);
        (tiers[tier] ??= []).push(i);
    }
    for (let tier = tiers.length - 1; tier >= 1; tier--) {
        order.push(...(tiers[tier] ?? []));
    }
    const ring: number[] = [];
    for (let k = 0; k < m; k++) {
        if (!(hasFocus && seq[k] === centre)) ring.push(seq[k]);
    }
    ring.sort((a, b) => a - b);
    order.push(...ring);
    if (hasFocus) {
        order.push(centre);
    }
    return { poses, order };
}

// ---- Controllers --------------------------------------------------------------------------------------------------

/** What a shell may claim on a controller, in the order the node lists it: the names `controller.claimed` accepts. */
export const CLAIMABLE_INPUTS: readonly string[] = ["trigger", "grip", "thumbstick", "primary", "secondary"];

/**
 * The longest settle time a chord may have, in milliseconds. The program holds a press back from the window for 150 ms
 * (`xr_input::CLAIM_HOLD`) to wait for a claim; the claim is made after the settle time and needs about as long again
 * to travel shell, compositor and program, so a settle time past 60 would claim after the hold.
 */
const MAX_CHORD_SETTLE_MS = 60;

const BUTTON_ORDER: readonly ControllerButton[] = ["Trigger", "Grip", "Thumbstick", "Primary", "Secondary"];
const BUTTON_NAMES: Readonly<Record<string, ControllerButton>> = {
    trigger: "Trigger",
    grip: "Grip",
    thumbstick: "Thumbstick",
    primary: "Primary",
    secondary: "Secondary",
};

/** A trigger pulled to this much is down, and it stays down until it falls to `RELEASE`. */
const PRESS = 0.75;
const RELEASE = 0.25;

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

/** One tracked hand: a `controller` node the XR program writes. Read it; it cannot be written. */
export class XRController {
    readonly #world: XRWorld;
    readonly #node: NodeNumber;
    readonly #down = new Set<ControllerButton>();
    /** What each owner (a binding, a script) has claimed here; the node says the union. */
    readonly #claims = new Map<object, ReadonlySet<string>>();
    readonly #hand: string;

    /** @internal Made by `XRSession`. */
    constructor(world: XRWorld, node: NodeNumber) {
        this.#world = world;
        this.#node = node;
        // Read once: a hand does not change, and a controller that has gone still says which it was.
        this.#hand = (world.scene.get(node, "hand") as string | undefined) ?? "";
    }

    get id(): string {
        return String(this.#node);
    }

    get alive(): boolean {
        return this.#world.kind(this.#node) === "controller";
    }

    #number(key: string): number {
        return (this.#world.scene.get(this.#node, key) as number | undefined) ?? 0;
    }

    /** `left` or `right`. */
    get hand(): string {
        return this.#hand;
    }

    /** Whether the runtime tracks the controller now. An untracked controller starts no drag and ends the one it has. */
    get tracked(): boolean {
        return (this.#world.scene.get(this.#node, "tracked") as boolean | undefined) === true;
    }

    /** The pose of the aim ray in the room's placement frame, in room units. */
    get pose(): Transform3D {
        const scene = this.#world.scene;
        const rotation = (scene.get(this.#node, "rotation") as number[] | undefined) ?? [0, 0, 0];
        return new Transform3D(new Vector3(this.#number("x"), this.#number("y"), this.#number("z")), rotation);
    }

    /** The aim ray: from the pose's position along its negative z, as the program points it. */
    get ray(): Ray {
        const pose = this.pose;
        return new Ray(pose.position, pose.transformDirection(new Vector3(0, 0, -1)));
    }

    /** How far the trigger is pulled, 0 to 1. */
    get trigger(): number {
        return this.#number("trigger");
    }

    /** How far the grip is squeezed, 0 to 1. */
    get grip(): number {
        return this.#number("grip");
    }

    get thumbstick(): { x: number; y: number } {
        return { x: this.#number("thumbstick_x"), y: this.#number("thumbstick_y") };
    }

    /** The buttons that are down, as of the last frame. */
    get buttons(): ReadonlySet<ControllerButton> {
        return this.#down;
    }

    /** The inputs the shell withholds from the window under the ray now: the union of every claim, in the order of `CLAIMABLE_INPUTS`. */
    get claimed(): readonly string[] {
        return (this.#world.scene.get(this.#node, "claimed") as string[] | undefined) ?? [];
    }

    /**
     * Claims `inputs` (any of `trigger`, `grip`, `thumbstick`, `primary`, `secondary`, lowercase) for `owner`, so the XR
     * program does not pass them to the window under the ray while the claim stands. A claim replaces the same owner's
     * earlier one; the claims of other owners stay, and the node says the union of them all. It throws a `TypeError`
     * naming `Controller#id.claimed` for a list that is not one of those names, and a `SceneError` when the controller
     * is gone or the shell refuses the write (the owner then holds nothing).
     */
    claim(owner: object, inputs: readonly string[]): void {
        const target = `Controller#${this.id}.claimed`;
        if (typeof owner !== "object" || owner === null) {
            throw new TypeError(`${target}: a claim needs an owner object, got ${String(owner)}`);
        }
        if (!Array.isArray(inputs)) {
            throw new TypeError(`${target}: claim a list of input names such as ["grip"], got ${String(inputs)}`);
        }
        for (const name of inputs) {
            if (typeof name !== "string") {
                throw new TypeError(`${target}: an input name is a string, got ${String(name)}`);
            }
            if (name !== name.toLowerCase()) {
                throw new TypeError(
                    `${target}: input names are lowercase; write "${name.toLowerCase()}", not "${name}"`,
                );
            }
            if (!CLAIMABLE_INPUTS.includes(name)) {
                throw new TypeError(`${target}: "${name}" is not an input; use ${CLAIMABLE_INPUTS.join(", ")}`);
            }
        }
        if (!this.alive) {
            throw new SceneError("destroyed", target, "the controller is gone");
        }
        const before = this.#claims.get(owner);
        if (inputs.length === 0) {
            this.#claims.delete(owner);
        } else {
            this.#claims.set(owner, new Set(inputs));
        }
        try {
            this.#flush(target);
        } catch (error) {
            if (before === undefined) this.#claims.delete(owner);
            else this.#claims.set(owner, before);
            throw error;
        }
    }

    /** Drops `owner`'s claim. The claims of other owners stand. A controller that is gone holds none, so it is not an error. */
    release(owner: object): void {
        if (!this.#claims.delete(owner) || !this.alive) {
            return;
        }
        this.#flush(`Controller#${this.id}.claimed`);
    }

    /** Writes the union of the claims, once, when it differs from what the node says. */
    #flush(target: string): void {
        const union = CLAIMABLE_INPUTS.filter((name) => [...this.#claims.values()].some((held) => held.has(name)));
        const now = this.claimed;
        if (now.length === union.length && now.every((name, at) => name === union[at])) {
            return;
        }
        const code = this.#world.scene.set(this.#node, "claimed", union);
        if (code !== undefined) {
            throw new SceneError(code, target, `the shell refused the write (${code})`);
        }
    }

    /** @internal Reads the values once and settles which buttons are down (a pull is down at 0.75, up at 0.25). */
    update(): void {
        const scene = this.#world.scene;
        const flag = (key: string): boolean => (scene.get(this.#node, key) as boolean | undefined) === true;
        const analog: [ControllerButton, number][] = [
            ["Trigger", this.trigger],
            ["Grip", this.grip],
        ];
        for (const [button, value] of analog) {
            if (this.#down.has(button)) {
                if (value <= RELEASE) this.#down.delete(button);
            } else if (value >= PRESS) {
                this.#down.add(button);
            }
        }
        const digital: [ControllerButton, boolean][] = [
            ["Thumbstick", flag("thumbstick_click")],
            ["Primary", flag("primary")],
            ["Secondary", flag("secondary")],
        ];
        for (const [button, down] of digital) {
            if (down) this.#down.add(button);
            else this.#down.delete(button);
        }
    }
}

/** A list the XR module keeps current, which a listener can follow. */
export class XRList<T extends { readonly id: string }> implements Iterable<T> {
    readonly #items: T[] = [];
    readonly #listeners: object[] = [];
    readonly #report: (site: string, error: unknown) => void;

    constructor(report: (site: string, error: unknown) => void) {
        this.#report = report;
    }

    get size(): number {
        return this.#items.length;
    }

    get(id: string): T | undefined {
        return this.#items.find((item) => item.id === id);
    }

    at(index: number): T | undefined {
        return this.#items[index];
    }

    /** Calls `onAdded(item)` and `onRemoved(item)` on `listener` for later changes. */
    addListener(listener: object): void {
        if (typeof listener !== "object" || listener === null) {
            throw new TypeError("addListener: the listener must be an object");
        }
        this.#listeners.push(listener);
    }

    removeListener(listener: object): void {
        const at = this.#listeners.indexOf(listener);
        if (at >= 0) this.#listeners.splice(at, 1);
    }

    /** @internal */
    add(item: T): void {
        this.#items.push(item);
        this.#tell("onAdded", item);
    }

    /** @internal */
    remove(item: T): void {
        const at = this.#items.indexOf(item);
        if (at >= 0) {
            this.#items.splice(at, 1);
            this.#tell("onRemoved", item);
        }
    }

    #tell(method: "onAdded" | "onRemoved", item: T): void {
        for (const listener of [...this.#listeners]) {
            const handler = Reflect.get(listener, method);
            if (typeof handler === "function") {
                try {
                    handler.call(listener, item);
                } catch (error) {
                    this.#report(`XRList.${method}`, error);
                }
            }
        }
    }

    [Symbol.iterator](): Iterator<T> {
        return [...this.#items][Symbol.iterator]();
    }
}

// ---- The camera, the viewport and the head ------------------------------------------------------------------------

/** Initial head-camera tracking and clipping values; near and far are metres. */
export interface XRCamera3DOptions extends Omit<Camera3DOptions, "near" | "far"> {
    /** The head the camera follows. The program writes the pose onto the camera named `head`. */
    tracking?: XRTrackedPose;
    /** Only `local` is tracked: the program runs one OpenXR local space. */
    referenceSpace?: "local";
    /** In metres, as the spec's `new XRCamera3D({ near: 0.05, far: 100 })`. */
    near?: number;
    far?: number;
}

/** The stereo camera that follows the headset: the camera named `head`, which the XR program writes. */
export class XRCamera3D extends Camera3D {
    readonly tracking: XRTrackedPose | null;
    readonly referenceSpace = "local";

    constructor(options: XRCamera3DOptions = {}) {
        if (options.referenceSpace !== undefined && options.referenceSpace !== "local") {
            throw new TypeError(
                `XRCamera3D: referenceSpace "${String(options.referenceSpace)}" is not tracked; only "local" is`,
            );
        }
        const metres = options.metres ?? 0.001;
        super({
            name: "head",
            x: options.x,
            y: options.y,
            z: options.z,
            rotation: options.rotation,
            scale: options.scale,
            visible: options.visible,
            opacity: options.opacity,
            input: options.input,
            fov: options.fov,
            ipd: options.ipd,
            projection: "stereo",
            metres,
            near: options.near === undefined ? undefined : options.near / metres,
            far: options.far === undefined ? undefined : options.far / metres,
        });
        this.tracking = options.tracking ?? null;
    }
}

/** How the XR runtime composes panels: automatically, as layers or through the projection camera. */
export type XRComposition = "auto" | "layers" | "projection";

/** The scene root, camera and composition the headset should render. */
export interface XRViewportOptions {
    /** The room: the node the XR program reads. */
    content: Node;
    camera: Camera3D;
    composition?: XRComposition;
}

/**
 * What the headset shows: the room and the camera it is seen through. In this model that is two names. The program
 * reads the node named `room` and the camera named `head`, so assigning a viewport to `session.viewport` names them.
 * `composition: "projection"` draws every panel through the head camera as a projection layer; `"layers"` and `"auto"`
 * leave the panels flat quads.
 */
export class XRViewport {
    readonly content: Node;
    readonly camera: Camera3D;
    composition: XRComposition;

    constructor(options: XRViewportOptions) {
        if (!(options?.content instanceof Node)) {
            throw new TypeError("XRViewport: content must be the room node");
        }
        if (!(options.camera instanceof Camera3D)) {
            throw new TypeError("XRViewport: camera must be a camera");
        }
        const composition = options.composition ?? "auto";
        if (composition !== "auto" && composition !== "layers" && composition !== "projection") {
            throw new TypeError(
                `XRViewport: composition "${String(composition)}" must be "auto", "layers" or "projection"`,
            );
        }
        this.content = options.content;
        this.camera = options.camera;
        this.composition = composition;
    }

    /** @internal Names the nodes for the program and puts the camera under the room. */
    apply(): void {
        if (this.content.parent === null) {
            mountOnRoot(this.content);
        }
        this.content.name = "room";
        if (this.content instanceof XRRoom && this.content.head !== this.camera) {
            // One camera is the head: the room's own steps aside, and goes if nothing hangs from it.
            const old = this.content.head;
            this.content.head = this.camera;
            if (old.children.length === 0) old.destroy();
            else old.name = "";
        }
        this.camera.name = "head";
        if (this.camera.parent !== this.content) {
            this.content.add(this.camera);
        }
        for (const child of this.content.children) {
            if (child instanceof View && child.parent === this.content) {
                child.camera = this.composition === "projection" ? this.camera : null;
            }
        }
    }
}

/** The head: the pose the program writes onto the camera named `head`, in the room's placement frame. */
export class XRTrackedPose {
    readonly #world: XRWorld;
    readonly #camera: () => NodeNumber | undefined;

    /** @internal */
    constructor(world: XRWorld, camera: () => NodeNumber | undefined) {
        this.#world = world;
        this.#camera = camera;
    }

    /** Whether a head camera exists for the program to write. */
    get available(): boolean {
        return this.#camera() !== undefined;
    }

    /**
     * The head's pose, or null with no head camera. The program writes the pose onto the camera with the scene's y
     * mirrored and the turns about x and z reversed (`xr_scene::head`); this undoes that, so the pose is in the same
     * frame as a View's and a controller's. Positions are in room units.
     */
    get pose(): Transform3D | null {
        const camera = this.#camera();
        if (camera === undefined) return null;
        const scene = this.#world.scene;
        const number = (key: string): number => (scene.get(camera, key) as number | undefined) ?? 0;
        const rotation = (scene.get(camera, "rotation") as number[] | undefined) ?? [0, 0, 0];
        return new Transform3D(new Vector3(number("x"), -number("y"), number("z")), [
            -rotation[0],
            rotation[1],
            -rotation[2],
        ]);
    }
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
 * for the session's settle time, so a two-button chord is not taken for its first button.
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

/** Where a ray struck a panel. */
export interface XRHit {
    readonly view: View;
    /** In the room's frame. */
    readonly point: Vector3;
    /** In the panel's own pixels, from its centre: x to the right, y up. */
    readonly local: { x: number; y: number };
}

/** The panel the ray strikes: the last child of the room first, because the room lists its Views back to front. */
export function pickView(room: XRRoom, ray: Ray): XRHit | null {
    const views = room.children.filter((child): child is View => child instanceof View);
    for (let at = views.length - 1; at >= 0; at--) {
        const view = views[at];
        if (!view.visible || !view.input) continue;
        const world = view.worldTransform;
        const plane = world.plane();
        // A quad is one-sided: its front is the +z side, so the ray must travel towards it.
        if (plane.normal.dot(ray.direction) >= 0) continue;
        const point = plane.intersect(ray);
        if (point === null) continue;
        const local = world.inverse().transformPoint(point);
        if (Math.abs(local.x) <= view.width / 2 && Math.abs(local.y) <= view.height / 2) {
            return { view, point, local: { x: local.x, y: local.y } };
        }
    }
    return null;
}

/** The placement of the room's parent chain above `node`, so a world pose can be turned into the node's own. */
function aboveTransform(node: Node): Transform3D {
    const above = node.parent;
    return above === null ? Transform3D.identity() : above.worldTransform;
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

interface MoveState {
    view: View;
    /** Whether the panel was pinned before the drag, so a cancel can put that back too. */
    wasPinned: boolean;
    startLocal: Transform3D;
    startWorld: Transform3D;
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
            wasPinned: event.room.isPinned(view),
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
        // The view lives in the room's space, so the world pose goes back through its parents.
        s.view.transform = aboveTransform(s.view).inverse().multiply(target);
        event.room.pin(s.view);
    }

    override onCancel(event: XRDragEvent, state: object): void {
        const s = state as MoveState;
        s.view.transform = s.startLocal;
        if (!s.wasPinned) event.room.unpin(s.view);
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
}

interface ResizeState {
    view: View;
    wasPinned: boolean;
    startLocal: Transform3D;
    startWorld: Transform3D;
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

    constructor(options: ViewResizeOptions = {}) {
        super({ id: options.id ?? "resize", trigger: options.trigger ?? "Grip+Trigger+Drag", ...pick(options) });
        this.pixelsPerMetre = options.pixelsPerMetre ?? 1200;
        this.minWidth = options.minWidth ?? 0.25;
        this.minHeight = options.minHeight ?? 0.15;
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
        const corner = { x: local.x >= 0 ? 1 : -1, y: local.y >= 0 ? 1 : -1 };
        const width = view.width;
        const height = view.height;
        return {
            view,
            wasPinned: event.room.isPinned(view),
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
        s.view.transform = aboveTransform(s.view).inverse().multiply(target);
        event.room.pin(s.view);
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
        s.view.transform = aboveTransform(s.view).inverse().multiply(target);
    }

    override onCancel(event: XRDragEvent, state: object): void {
        const s = state as ResizeState;
        s.view.transform = s.startLocal;
        if (!s.wasPinned) event.room.unpin(s.view);
    }
}

// ---- The room -----------------------------------------------------------------------------------------------------

export interface SpatialLayoutOptions {
    /** The distance from the viewer to the centre of a panel, in metres. Default 1.6. */
    radius?: number;
    /** The height of a panel's centre above the floor, in metres. Default 1.4. */
    eye?: number;
    /** The angle between neighbouring panels, in radians. Default 3 degrees. */
    spacing?: number;
    /** The metres of panel for one pixel of window. Default 0.00075. */
    metresPerPixel?: number;
    /** The focused panel is this much larger. Default 1.15. */
    focusScale?: number;
    /** ... and this many metres closer. Default 0.1. */
    focusPull?: number;
    /** The most the panels reach to either side of the front, in radians. Default 150 degrees. */
    halfArc?: number;
    /** Panels on the ring; the rest stack behind it. Default 12. */
    maxPanels?: number;
    /** Each tier of stacked panels is this many metres further back. Default 0.25. */
    stackStep?: number;
    /** The triggers of the stock bindings by id; `null` leaves a binding out. */
    triggers?: { move?: string | null; resize?: string | null };
}

const LAYOUT_TRIGGERS = { move: "Grip+Drag", resize: "Grip+Trigger+Drag" } as const;

/**
 * The room's arrangement: panels on a ring round the viewer, and the bindings that move and resize them. The ring is
 * `arrangeRing`; `bindings` is the layout's collection, so `layout.bindings.remove("move")` turns a binding off and
 * `layout.bindings.add(new ViewMoveBinding({ tracking: "ray" }))` replaces it.
 */
export class SpatialLayout {
    readonly bindings = new BindingCollection();
    readonly ring: XRRingParams;

    constructor(options: SpatialLayoutOptions = {}) {
        const metre = 1000;
        const ring: XRRingParams = { ...RING_DEFAULTS };
        const set = (key: keyof XRRingParams, value: number | undefined, factor = 1): void => {
            if (value === undefined) return;
            if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) {
                throw new RangeError(`SpatialLayout: ${key} must be a number above 0, got ${String(value)}`);
            }
            ring[key] = value * factor;
        };
        set("radius", options.radius, metre);
        set("eye", options.eye, metre);
        set("gap", options.spacing);
        set("mmPerPx", options.metresPerPixel, metre);
        set("focusScale", options.focusScale);
        set("focusPull", options.focusPull, metre);
        set("halfArc", options.halfArc);
        set("maxPanels", options.maxPanels);
        set("stackStep", options.stackStep, metre);
        this.ring = ring;
        const triggers: Record<keyof typeof LAYOUT_TRIGGERS, string | null> = { ...LAYOUT_TRIGGERS };
        for (const name of Object.keys(options.triggers ?? {})) {
            if (!Object.hasOwn(LAYOUT_TRIGGERS, name)) {
                throw new TypeError(
                    `SpatialLayout: unknown trigger "${name}"; expected one of ${Object.keys(LAYOUT_TRIGGERS).join(", ")}`,
                );
            }
            const value = (options.triggers as Record<string, string | null | undefined>)[name];
            if (value === undefined) continue;
            if (value !== null) parseControllerTrigger(value, `SpatialLayout trigger "${name}"`);
            triggers[name as keyof typeof LAYOUT_TRIGGERS] = value;
        }
        if (triggers.move !== null) this.bindings.add(new ViewMoveBinding({ trigger: triggers.move }));
        if (triggers.resize !== null) this.bindings.add(new ViewResizeBinding({ trigger: triggers.resize }));
    }

    arrange<I>(views: readonly RingView<I>[], focus: I | null): RingResult {
        return arrangeRing(views, focus, this.ring);
    }
}

/** Initial layout and client filter for an XR room. */
export interface XRRoomOptions extends NodeOptions {
    layout?: SpatialLayout | null;
    /** Whether a client gets a panel. Secure clients never do. Default: every client. */
    accept?: (client: XRClient) => boolean;
}

/** Position and scale of a panel fixed to the head; distances are metres. */
export interface HeadPanelOptions {
    /** Metres right of, above and in front of the head. Default `0, 0, -1.2`. */
    x?: number;
    y?: number;
    z?: number;
    /** Millimetres of panel for one pixel of window. Default the ring's. */
    scale?: number;
}

/**
 * The node the XR program reads, with the head camera under it and one `View` for each client, placed by `layout`.
 * Panels follow the clients: a client that maps gets one, one that closes or turns secure loses it, a minimised
 * client's panel is hidden. A panel the user moved or resized is pinned: the layout leaves its place alone until
 * `unpin`. The room never changes a client.
 */
export class XRRoom extends Node {
    /** The camera the program writes the head pose onto: the one named `head`. */
    head: Camera3D;
    layout: SpatialLayout | null;
    accept: (client: XRClient) => boolean;
    readonly #views = new Map<XRClient, View>();
    /** The clients in the order they gained a View: the ring's order. */
    readonly #order: XRClient[] = [];
    readonly #pinned = new WeakSet<View>();
    readonly #headPanels = new Map<View, XRClient>();

    constructor(options: XRRoomOptions = {}) {
        const { layout, accept, ...rest } = options;
        const created = layout === undefined ? new SpatialLayout() : layout;
        super({ name: "room", y: -(created?.ring.eye ?? RING_DEFAULTS.eye), ...rest });
        this.layout = created;
        this.accept = accept ?? (() => true);
        this.head = new XRCamera3D();
        this.add(this.head);
    }

    /** Metres per room unit: the head camera's. */
    get unit(): number {
        return this.head.metres;
    }

    /** The panel that shows `client`, if it has one. */
    viewOf(client: XRClient): View | undefined {
        return this.#views.get(client);
    }

    /** Every panel of the room's ring, in the order the clients gained them. */
    get views(): View[] {
        return this.#order.flatMap((client) => this.#views.get(client) ?? []);
    }

    pin(view: View): void {
        this.#pinned.add(view);
    }

    unpin(view: View): void {
        this.#pinned.delete(view);
    }

    isPinned(view: View): boolean {
        return this.#pinned.has(view);
    }

    /** Shows `client` in a panel fixed to the head, in metres in front of the eyes. */
    pinToHead(client: XRClient, options: HeadPanelOptions = {}): View {
        const metres = this.unit;
        const scale = options.scale ?? this.layout?.ring.mmPerPx ?? RING_DEFAULTS.mmPerPx;
        const view = new View({
            content: client,
            x: (options.x ?? 0) / metres,
            y: (options.y ?? 0) / metres,
            z: (options.z ?? -1.2) / metres,
            scale: [scale, scale, scale],
            width: client.width,
            height: client.height,
        });
        this.head.add(view);
        this.#headPanels.set(view, client);
        return view;
    }

    #addView(client: XRClient): void {
        const view = new View({
            content: client,
            width: client.width,
            height: client.height,
            visible: !client.minimised,
        });
        view.input = true;
        this.add(view);
        this.#views.set(client, view);
        this.#order.push(client);
    }

    #dropView(client: XRClient): void {
        const view = this.#views.get(client);
        this.#views.delete(client);
        const at = this.#order.indexOf(client);
        if (at >= 0) this.#order.splice(at, 1);
        view?.destroy();
    }

    /**
     * Brings the panels up to date with `clients` and places them. `dialog` says whether a client is a dialog (drawn
     * inside its parent's panel). Writes only what changed.
     */
    reconcile(clients: readonly XRClient[], focus: XRClient | null, dialog: (client: XRClient) => boolean): void {
        const layout = this.layout;
        if (layout === null) {
            return;
        }
        const wanted = new Set<XRClient>();
        for (const client of clients) {
            if (client.alive && !client.secure && !dialog(client) && this.accept(client)) wanted.add(client);
        }
        for (const client of [...this.#views.keys()]) {
            const view = this.#views.get(client);
            if (!wanted.has(client) || view === undefined || !view.alive || view.orphaned) {
                this.#dropView(client);
            }
        }
        for (const client of clients) {
            if (wanted.has(client) && !this.#views.has(client)) this.#addView(client);
        }
        const hidden: View[] = [];
        const ring: { client: XRClient; view: View }[] = [];
        const pinned: View[] = [];
        for (const client of this.#order) {
            const view = this.#views.get(client);
            if (view === undefined) continue;
            view.width = client.width;
            view.height = client.height;
            view.visible = !client.minimised;
            if (client.minimised) hidden.push(view);
            else if (this.#pinned.has(view)) pinned.push(view);
            else ring.push({ client, view });
        }
        const { poses, order } = layout.arrange(
            ring.map(({ client, view }) => ({ id: client, width: view.width, height: view.height })),
            focus,
        );
        ring.forEach(({ view }, at) => {
            const pose = poses[at];
            view.x = pose.x;
            view.y = pose.y;
            view.z = pose.z;
            view.rotation = pose.rotation;
            view.scale = [pose.scale, pose.scale, pose.scale];
        });
        // Back to front: the program submits Views in tree order and nothing depth-tests, so the nearest is last.
        // A pinned panel goes where its distance puts it among the ring's.
        const back: View[] = order.map((at) => ring[at].view);
        const distance = (view: View): number => view.worldTransform.position.length();
        for (const view of pinned) {
            const own = distance(view);
            const at = back.findIndex((other) => distance(other) < own);
            back.splice(at < 0 ? back.length : at, 0, view);
        }
        const want = [...hidden, ...back];
        const mine = new Set<View>(want);
        const current = this.children.filter((child): child is View => child instanceof View && mine.has(child));
        if (current.length !== want.length || current.some((view, at) => view !== want[at])) {
            for (const view of want) this.add(view);
        }
        for (const [panel, client] of [...this.#headPanels]) {
            if (!client.alive || client.secure || !panel.alive || panel.orphaned) {
                this.#headPanels.delete(panel);
                panel.destroy();
                continue;
            }
            panel.width = client.width;
            panel.height = client.height;
            panel.visible = !client.minimised;
        }
    }
}

// ---- The session and the extension ---------------------------------------------------------------------------------

/** The hooks of a listener of the extension. Subclass it and override what you need. */
export class XRListener {
    /** A session began: the program made the `xr` seat. */
    onSessionStarted(_session: XRSession): void {}
    /** The session ended: the seat is gone. `session.alive` is already false. */
    onSessionEnded(_session: XRSession): void {}
}

/** One headset session, with its head pose, controllers and optional viewport. */
export class XRSession {
    readonly #world: XRWorld;
    readonly #seat: NodeNumber;
    #alive = true;
    #viewport: XRViewport | null = null;
    #onViewport: (viewport: XRViewport) => void;
    readonly controllers: XRList<XRController>;
    readonly head: XRTrackedPose;
    #chordSettleMs = 50;

    /**
     * How long, in milliseconds, a chord must have been down before a drag starts, and so before the drag claims its
     * buttons. Default 50, at most `MAX_CHORD_SETTLE_MS` (60): a larger value reads back as 60.
     *
     * The XR program holds button presses back from the window under the ray for 150 ms while it waits for a claim.
     * The shell must claim within this settle interval for a drag to own those inputs before a click reaches the window.
     * Longer waits would outlast the hold, so `chordSettleMs` is capped at 60 ms.
     */
    get chordSettleMs(): number {
        return this.#chordSettleMs;
    }

    set chordSettleMs(ms: number) {
        if (typeof ms !== "number" || Number.isNaN(ms)) {
            throw new TypeError(`session.chordSettleMs must be a number of milliseconds, got ${String(ms)}`);
        }
        this.#chordSettleMs = Math.min(Math.max(ms, 0), MAX_CHORD_SETTLE_MS);
    }

    /** @internal Made by the runtime when the seat appears. */
    constructor(world: XRWorld, seat: NodeNumber, onViewport: (viewport: XRViewport) => void) {
        this.#world = world;
        this.#seat = seat;
        this.#onViewport = onViewport;
        this.controllers = new XRList<XRController>(world.report);
        this.head = new XRTrackedPose(world, () => this.#headCamera());
    }

    #headCamera(): NodeNumber | undefined {
        return this.#world.nodes("Camera").find((node) => this.#world.scene.get(node, "name") === "head");
    }

    get id(): string {
        return String(this.#seat);
    }

    /** False once the program ended the session (the seat is gone). A dead session rejects writes. */
    get alive(): boolean {
        return this.#alive;
    }

    /** Whether the program is connected: the same as `alive`, because the seat is the session. */
    get connected(): boolean {
        return this.#alive;
    }

    /** `tracked` while a controller is tracked, else `untracked`. The tree does not say whether the head is. */
    get tracking(): "tracked" | "untracked" {
        for (const controller of this.controllers) {
            if (controller.tracked) return "tracked";
        }
        return "untracked";
    }

    /** The seat the program made, as an input context: gaze and the pointer ray arrive here as pointer input. */
    get input(): object | null {
        return this.#alive ? this.#world.inputContext(this.#seat) : null;
    }

    /** The number of layers the runtime allows in a frame, as the program reports it on the head camera; 0 if not yet. */
    get layerBudget(): number {
        const camera = this.#headCamera();
        return camera === undefined ? 0 : ((this.#world.scene.get(camera, "layer_budget") as number | undefined) ?? 0);
    }

    get viewport(): XRViewport | null {
        return this.#viewport;
    }

    set viewport(value: XRViewport) {
        if (!this.#alive) {
            throw new Error("XRSession.viewport: the session has ended");
        }
        if (!(value instanceof XRViewport)) {
            throw new TypeError("XRSession.viewport: expected an XRViewport");
        }
        this.#onViewport(value);
        this.#viewport = value;
    }

    /** @internal */
    end(): void {
        this.#alive = false;
    }

    /** @internal Brings `controllers` up to date with the controller nodes in the tree. */
    syncControllers(): void {
        const nodes = this.#world.nodes("Controller");
        for (const controller of [...this.controllers]) {
            if (!nodes.includes(Number(controller.id))) this.controllers.remove(controller);
        }
        for (const node of nodes) {
            if (this.controllers.get(String(node)) === undefined) {
                this.controllers.add(new XRController(this.#world, node));
            }
        }
        for (const controller of this.controllers) controller.update();
    }
}

/** `gnoblin.extensions.xr`: present while the program has a session. */
export class XRExtension {
    readonly available = true;
    readonly sessions: XRList<XRSession>;
    readonly Camera3D = XRCamera3D;
    readonly Viewport = XRViewport;
    readonly SpatialLayout = SpatialLayout;
    readonly ViewMoveBinding = ViewMoveBinding;
    readonly ViewResizeBinding = ViewResizeBinding;
    readonly Room = XRRoom;
    /** The room the stock shell built, or the one a script assigned as the viewport's content. */
    room: XRRoom | null = null;

    /** @internal */
    constructor(sessions: XRList<XRSession>) {
        this.sessions = sessions;
    }

    /** The live session, or null. */
    get session(): XRSession | null {
        for (const session of this.sessions) {
            if (session.alive) return session;
        }
        return null;
    }
}

export interface XROptions {
    /** Whether the stock shell builds a room when a session starts. Default true. */
    room?: boolean;
    /** The ring and the bindings of the stock room. */
    layout?: SpatialLayoutOptions;
    accept?: (client: XRClient) => boolean;
}

interface Run {
    readonly controller: XRController;
    readonly binding: XRControllerDragBinding;
    readonly state: object;
    readonly view: View;
    readonly hit: Vector3;
    last: number;
}

/**
 * The shell's side of XR, one per shell. It follows the seat: while the program has a session it builds the room, keeps
 * the panels current and runs the drags; when the session ends it takes all of it down.
 */
export class XRRuntime {
    readonly #world: XRWorld;
    readonly #sessions: XRList<XRSession>;
    readonly #listeners: object[] = [];
    readonly #runs = new Map<XRController, Run>();
    readonly #declined = new Set<XRController>();
    readonly #since = new Map<XRController, { key: string; at: number }>();
    #extension: XRExtension | null = null;
    #session: XRSession | null = null;
    #stock: XRRoom | null = null;
    options: XROptions = {};

    constructor(world: XRWorld) {
        this.#world = world;
        this.#sessions = new XRList<XRSession>(world.report);
    }

    get extension(): XRExtension | null {
        return this.#extension;
    }

    addListener(listener: object): void {
        if (typeof listener !== "object" || listener === null) {
            throw new TypeError("addListener: the listener must be an object");
        }
        this.#listeners.push(listener);
    }

    removeListener(listener: object): void {
        const at = this.#listeners.indexOf(listener);
        if (at >= 0) this.#listeners.splice(at, 1);
    }

    #tell(method: "onSessionStarted" | "onSessionEnded", session: XRSession): void {
        for (const listener of [...this.#listeners]) {
            const handler = Reflect.get(listener, method);
            if (typeof handler === "function") {
                try {
                    handler.call(listener, session);
                } catch (error) {
                    this.#world.report(`XRListener.${method}`, error);
                }
            }
        }
    }

    /** Whether `gnoblin/extensions/xr` may be imported now: the session declared XR, or the program has a session. */
    available(): boolean {
        return (
            this.#world.declared() || this.#extension !== null || this.#world.seats().some((seat) => seat.name === "xr")
        );
    }

    #adopt(viewport: XRViewport): void {
        if (this.#stock !== null && viewport.content !== this.#stock) {
            // A script's own room replaces the stock one.
            this.#dropStock();
        }
        viewport.apply();
        if (this.#extension !== null && viewport.content instanceof XRRoom) {
            this.#extension.room = viewport.content;
        }
    }

    #dropStock(): void {
        const stock = this.#stock;
        this.#stock = null;
        if (stock !== null) {
            if (this.#extension?.room === stock) this.#extension.room = null;
            if (stock.alive) stock.destroy();
        }
    }

    /** Builds the stock room unless the script said not to or a room is in the tree already (the user's). */
    #buildStock(session: XRSession): void {
        if (this.options.room === false || this.#stock !== null) {
            return;
        }
        const taken = this.#world.nodes("Node").some((node) => this.#world.scene.get(node, "name") === "room");
        if (taken) {
            return;
        }
        const room = new XRRoom({ layout: new SpatialLayout(this.options.layout), accept: this.options.accept });
        this.#stock = room;
        const viewport = new XRViewport({ content: room, camera: room.head, composition: "auto" });
        viewport.apply();
        session.viewport = viewport;
    }

    #start(seat: NodeNumber): void {
        const session = new XRSession(this.#world, seat, (viewport) => this.#adopt(viewport));
        this.#session = session;
        this.#sessions.add(session);
        this.#extension = new XRExtension(this.#sessions);
        this.#tell("onSessionStarted", session);
        this.#buildStock(session);
    }

    #end(): void {
        const session = this.#session;
        if (session === null) {
            return;
        }
        this.#cancelRuns();
        session.end();
        this.#session = null;
        this.#dropStock();
        for (const controller of [...session.controllers]) session.controllers.remove(controller);
        this.#sessions.remove(session);
        this.#extension = null;
        this.#since.clear();
        this.#declined.clear();
        this.#tell("onSessionEnded", session);
    }

    /** Runs once a frame: follows the seat, then keeps the room and the drags current. */
    frame(): void {
        const seat = this.#world.seats().find((candidate) => candidate.name === "xr");
        if (seat === undefined) {
            this.#end();
            return;
        }
        if (this.#session !== null && this.#session.id !== String(seat.node)) {
            this.#end();
        }
        if (this.#session === null) {
            this.#start(seat.node);
        }
        const session = this.#session;
        if (session === null) {
            return;
        }
        session.syncControllers();
        const room = this.#extension?.room ?? null;
        if (room !== null && room.alive) {
            try {
                room.reconcile(
                    this.#world.clients(),
                    this.#world.focusedClient(),
                    (client) => this.#world.parentKind(client) === "window",
                );
            } catch (error) {
                this.#world.report("XRRoom.reconcile", error);
            }
            this.#drags(session, room);
        } else {
            this.#cancelRuns();
        }
    }

    // -- Drags ---------------------------------------------------------------------------------------------------

    #event(
        session: XRSession,
        room: XRRoom,
        run: Pick<Run, "controller" | "binding" | "view" | "hit">,
        now: number,
        last: number,
    ): XRDragEvent {
        return {
            timestamp: now,
            deltaTime: Math.min(Math.max(now - last, 0) / 1000, 0.1),
            context: session.input,
            device: null,
            modifiers: new Set<string>(),
            controller: run.controller,
            view: run.view,
            ray: run.controller.ray,
            hit: run.hit,
            trigger: run.binding.trigger,
            hand: run.controller.hand,
            room,
            unit: room.unit,
        };
    }

    #hook(site: string, call: () => void): boolean {
        try {
            call();
            return true;
        } catch (error) {
            this.#world.report(site, error);
            return false;
        }
    }

    #finish(run: Run, hook: "onEnd" | "onCancel", session: XRSession, room: XRRoom, now: number): void {
        this.#runs.delete(run.controller);
        this.#unclaim(run);
        this.#hook(`${run.binding.id}.${hook}`, () =>
            run.binding[hook](this.#event(session, room, run, now, run.last), run.state),
        );
    }

    /** Drops what a drag claimed. A controller that is gone holds no claim, so only a refused write is reported. */
    #unclaim(run: Pick<Run, "controller" | "binding">): void {
        this.#hook(`${run.binding.id}.release`, () => run.controller.release(run.binding));
    }

    #cancelRuns(): void {
        const session = this.#session;
        const room = this.#extension?.room ?? null;
        for (const run of [...this.#runs.values()]) {
            this.#runs.delete(run.controller);
            this.#unclaim(run);
            if (session !== null && room !== null) {
                this.#hook(`${run.binding.id}.onCancel`, () =>
                    run.binding.onCancel(this.#event(session, room, run, this.#world.now(), run.last), run.state),
                );
            }
        }
    }

    /** Cancels every drag in flight (once each) and ends the session's hold. The shell calls it when a generation ends. */
    release(): void {
        this.#cancelRuns();
    }

    #drags(session: XRSession, room: XRRoom): void {
        const layout = room.layout;
        const now = this.#world.now();
        const bindings =
            layout === null
                ? []
                : [...layout.bindings].filter(
                      (b): b is XRControllerDragBinding => b instanceof XRControllerDragBinding,
                  );
        for (const controller of session.controllers) {
            const key = [...controller.buttons].sort().join("+");
            const seen = this.#since.get(controller);
            if (seen === undefined || seen.key !== key) {
                this.#since.set(controller, { key, at: now });
            }
            const run = this.#runs.get(controller);
            if (run !== undefined) {
                const chord = run.binding.parsed.buttons;
                const held =
                    chord.length === controller.buttons.size && chord.every((button) => controller.buttons.has(button));
                const still = bindings.includes(run.binding) && run.binding.enabled;
                if (!controller.alive || !controller.tracked || !still || !run.view.alive || run.view.orphaned) {
                    this.#finish(run, "onCancel", session, room, now);
                } else if (!held) {
                    this.#finish(run, "onEnd", session, room, now);
                } else {
                    const event = this.#event(session, room, run, now, run.last);
                    run.last = now;
                    if (!this.#hook(`${run.binding.id}.onUpdate`, () => run.binding.onUpdate(event, run.state))) {
                        this.#finish(run, "onCancel", session, room, now);
                    }
                }
                continue;
            }
            if (controller.buttons.size === 0) {
                this.#declined.delete(controller);
                continue;
            }
            const settled = now - (this.#since.get(controller)?.at ?? now) >= session.chordSettleMs;
            if (this.#declined.has(controller) || !controller.tracked || !settled) {
                continue;
            }
            const candidates = bindings
                .filter((binding) => binding.enabled && matches(binding.parsed, controller))
                .sort((a, b) => b.priority - a.priority);
            if (candidates.length === 0) {
                continue;
            }
            const ray = controller.ray;
            const hit = pickView(room, ray);
            for (const binding of candidates) {
                const event: XRDragEvent = {
                    ...this.#event(
                        session,
                        room,
                        { controller, binding, view: hit?.view as View, hit: hit?.point as Vector3 },
                        now,
                        now,
                    ),
                    view: hit?.view,
                    hit: hit?.point,
                };
                let state: object | false = false;
                if (!this.#hook(`${binding.id}.onStart`, () => (state = binding.onStart(event)))) {
                    continue;
                }
                if (state !== false && typeof state === "object" && state !== null && hit !== null) {
                    const run: Run = { controller, binding, state, view: hit.view, hit: hit.point, last: now };
                    this.#runs.set(controller, run);
                    // The drag owns its buttons from here: the program stops passing them to the window under the ray.
                    // A claim that cannot be made ends the drag at once, so a binding never runs half-claimed.
                    if (this.#hook(`${binding.id}.claim`, () => controller.claim(binding, binding.claims()))) {
                        break;
                    }
                    this.#finish(run, "onCancel", session, room, now);
                }
            }
            if (!this.#runs.has(controller)) {
                this.#declined.add(controller);
            }
        }
        // A controller that left the session while it held a drag.
        for (const run of [...this.#runs.values()]) {
            if (!session.controllers.get(run.controller.id)) {
                this.#finish(run, "onCancel", session, room, now);
            }
        }
    }
}

/** Whether `controller` holds exactly the buttons of `trigger`, on its hand. */
function matches(trigger: ControllerTriggerParts, controller: XRController): boolean {
    if (trigger.hand !== null && trigger.hand.toLowerCase() !== controller.hand.toLowerCase()) {
        return false;
    }
    return (
        trigger.buttons.length === controller.buttons.size &&
        trigger.buttons.every((button) => controller.buttons.has(button))
    );
}

// ---- The extensions registry --------------------------------------------------------------------------------------

/** The message of an import of an extension that is not there. */
export function missingExtension(name: string): string {
    return (
        `gnoblin/extensions/${name}: the ${name.toUpperCase()} extension is not available. ` +
        "The session did not declare it (set GNOBLIN_XR=1 and install gnoblin-xr, see docs/xr.md) and no XR program has a session. " +
        `Guard the use with \`gnoblin.extensions.${name}\`, which is null without one, or import the module after the session starts.`
    );
}

/** What `for (const ext of gnoblin.extensions)` yields: the extension's id and where it is in its lifecycle. */
export interface ExtensionInfo {
    readonly id: string;
    /** `ready` while the extension serves a session, `unloaded` while it is absent. */
    readonly status: "ready" | "unloaded";
}

/** `gnoblin.extensions`: the optional native extensions, each null while absent. */
export class Extensions {
    readonly #runtime: XRRuntime;

    constructor(runtime: XRRuntime) {
        this.#runtime = runtime;
    }

    /** The XR extension while the program has a session; null otherwise. */
    get xr(): XRExtension | null {
        return this.#runtime.extension;
    }

    /**
     * The extensions the shell knows, one `ExtensionInfo` each (owner-spec.md section 17: `for (const ext of
     * gnoblin.extensions)`). The XR program is out of process, so its presence is its `xr` seat: `ready` while it has
     * a session (`gnoblin.extensions.xr` is the object), `unloaded` otherwise (`xr` is null).
     */
    [Symbol.iterator](): Iterator<ExtensionInfo> {
        const xr: ExtensionInfo = { id: "xr", status: this.#runtime.extension === null ? "unloaded" : "ready" };
        return [Object.freeze(xr)][Symbol.iterator]();
    }

    /** Hears `onSessionStarted(session)` and `onSessionEnded(session)` of the XR extension. */
    addListener(listener: object): void {
        this.#runtime.addListener(listener);
    }

    removeListener(listener: object): void {
        this.#runtime.removeListener(listener);
    }

    /** What the stock shell does for an extension: `configure("xr", { room: false })` builds no room of its own. */
    configure(name: "xr", options: XROptions): void {
        if (name !== "xr") {
            throw new TypeError(`gnoblin.extensions.configure: unknown extension "${String(name)}"; expected "xr"`);
        }
        if (typeof options !== "object" || options === null) {
            throw new TypeError("gnoblin.extensions.configure: options must be an object");
        }
        for (const key of Object.keys(options)) {
            if (key !== "room" && key !== "layout" && key !== "accept") {
                throw new TypeError(
                    `gnoblin.extensions.configure("xr"): unknown key "${key}"; expected room, layout or accept`,
                );
            }
        }
        if (options.layout !== undefined) {
            new SpatialLayout(options.layout);
        }
        this.#runtime.options = { ...options };
    }

    /** Throws `missingExtension` unless the extension can be used now. `gnoblin/extensions/xr` calls it as it loads. */
    require(name: "xr"): void {
        if (name !== "xr" || !this.#runtime.available()) {
            throw new Error(missingExtension(String(name)));
        }
    }
}
