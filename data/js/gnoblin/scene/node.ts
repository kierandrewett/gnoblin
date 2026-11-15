// `Node`: a script-created node of the compositor's scene tree, with its transform, its children, its pointer
// handlers and the property bindings that follow what they read. A node is placed with `position`, `rotation` (Euler
// radians, or a quaternion on write), `scale` and `transform`, all of which write the tree's `x`, `y`, `z`, `rotation`
// and `scale`.

import { errorCodeOfRefusal } from "gnoblin/errors";
import { propertiesOf } from "gnoblin/properties";
import type { PropertyBinding, PropertyBindings } from "gnoblin/properties";
import { host, SceneError, sceneState } from "gnoblin/scene/backend";
import type { HostValue, Scalar } from "gnoblin/scene/backend";
import { givenIds, gone, label, live, metresPerUnit, nodeIdOf, nodes, sceneForget } from "gnoblin/scene/registry";
import { Transform3D } from "gnoblin/scene/transform";
import { finite, Vector3 } from "gnoblin/scene/vector";
import { rotationFrom, scaleFrom, transformFrom, vectorFrom, withAxes } from "gnoblin/spatial";
import type { AxisTriple, RotationInit, ScaleInit, TransformInit, Triple, Vec3Like } from "gnoblin/spatial";

/** What a pointer that hits a node does: goes on into its content (`forward`), stops at it for the shell (`capture`) or misses it (`none`). */
export type InputMode = "forward" | "capture" | "none";

/** A pointer edge on a node of the scene the shell is sent: the point is in the node's own plane, in its units. */
export interface NodePointerEvent {
    /** The node the pointer is on. */
    readonly node: Node;
    /** The point in the node's own plane. */
    readonly x: number;
    readonly y: number;
    /** The button of a `press` or `release`, as the kernel numbers it (272 is the left one); 0 for the other edges. */
    readonly button: number;
}

/** Initial node values; position units depend on the parent, and rotation is in radians. */
export interface NodeOptions {
    /** The node's stable name, as `new Node({ id: "xr-room" })` in the specification. It is also `node.id` and `name`. */
    id?: string;
    name?: string;
    x?: number;
    y?: number;
    z?: number;
    position?: Triple | Vec3Like;
    /** Angles about X, Y and Z in radians as a list or `{ x, y, z }`, or a quaternion. */
    rotation?: RotationInit;
    /** One number for every axis, or a value per axis. */
    scale?: ScaleInit;
    /** Written after `position`, `rotation` and `scale`; a part it leaves out is not written. */
    transform?: TransformInit;
    visible?: boolean;
    opacity?: number;
    input?: boolean | InputMode;
}

/** A plain node of the scene: the XR room is one. Children are added with `add` and go with it when it is destroyed. */
export class Node {
    constructor(options: NodeOptions = {}, cls = "Node") {
        const made = sceneState.wrapping ?? host().create(cls);
        if (typeof made !== "number") {
            throw new SceneError(errorCodeOfRefusal(made), cls, `the shell could not make a ${cls} (${made})`);
        }
        nodes.set(this, made);
        live.set(made, this);
        try {
            this.assign(options);
        } catch (error) {
            this.destroy();
            throw error;
        }
    }

    /**
     * Writes the options a constructor was given; a subclass adds its own. Placement goes through the setters, so a
     * subclass that redefines `position` or `rotation` gets its own conversion.
     */
    protected assign(options: NodeOptions): void {
        if (options.id !== undefined) {
            if (typeof options.id !== "string" || options.id === "") {
                throw new TypeError(`${label(this)}.id: must be a non-empty string`);
            }
            givenIds.set(this, options.id);
            this.name = options.id;
        }
        if (options.name !== undefined) this.name = options.name;
        if (options.x !== undefined) this.x = options.x;
        if (options.y !== undefined) this.y = options.y;
        if (options.z !== undefined) this.z = options.z;
        if (options.position !== undefined) this.position = options.position;
        if (options.rotation !== undefined) this.rotation = options.rotation;
        if (options.scale !== undefined) this.scale = options.scale;
        if (options.transform !== undefined) this.transform = options.transform;
        if (options.visible !== undefined) this.visible = options.visible;
        if (options.opacity !== undefined) this.opacity = options.opacity;
        if (options.input !== undefined) this.input = options.input;
    }

    /** Whether the node is still in the tree. A destroyed node rejects every write. */
    get alive(): boolean {
        return !gone.has(this);
    }

    /** The id the node was made with (`new Node({ id })`), or else the tree's id as `gnoblinctl` prints it. */
    get id(): string {
        return givenIds.get(this) ?? String(nodeIdOf(this));
    }

    protected read<T extends Scalar | number[]>(key: string): T {
        return host().get(nodeIdOf(this), key) as T;
    }

    protected write(key: string, value: HostValue): void {
        if (gone.has(this)) {
            throw new SceneError("not_found", `${label(this)}.${key}`, "the node was destroyed");
        }
        // The tree sends every write it is given, so a write of what is already there is left out: a drag that
        // writes the pose every frame costs an op only for the parts that moved.
        const current = host().get(nodeIdOf(this), key);
        const same = Array.isArray(value)
            ? Array.isArray(current) &&
              current.length === value.length &&
              current.every((part, at) => part === value[at])
            : current === value;
        if (same) {
            return;
        }
        const code = host().set(nodeIdOf(this), key, value);
        if (code !== undefined) {
            throw new SceneError(
                errorCodeOfRefusal(code),
                `${label(this)}.${key}`,
                `the shell refused the write (${code})`,
            );
        }
    }

    get name(): string {
        return this.read<string>("name");
    }
    set name(value: string) {
        if (typeof value !== "string") throw new TypeError(`${label(this)}.name: must be a string`);
        this.write("name", value);
    }

    get x(): number {
        return this.read<number>("x");
    }
    set x(value: number) {
        this.write("x", finite(`${label(this)}.x`, value));
    }
    get y(): number {
        return this.read<number>("y");
    }
    set y(value: number) {
        this.write("y", finite(`${label(this)}.y`, value));
    }
    get z(): number {
        return this.read<number>("z");
    }
    set z(value: number) {
        this.write("z", finite(`${label(this)}.z`, value));
    }

    /**
     * The angles about X, Y and Z, in radians (turned about X, then Y, then Z). The list also answers to `x`, `y` and
     * `z`. A write takes a list, `{ x, y, z }` or a quaternion `{ x, y, z, w }`, which is stored as its Euler angles.
     */
    get rotation(): AxisTriple {
        return withAxes(this.read<number[]>("rotation"));
    }
    set rotation(value: RotationInit) {
        this.write("rotation", rotationFrom(`${label(this)}.rotation`, value));
    }

    /** The scale along each axis; the list also answers to `x`, `y` and `z`. A write takes one number for every axis. */
    get scale(): AxisTriple {
        return withAxes(this.read<number[]>("scale"));
    }
    set scale(value: ScaleInit) {
        this.write("scale", scaleFrom(`${label(this)}.scale`, value));
    }

    get visible(): boolean {
        return this.read<boolean>("visible");
    }
    set visible(value: boolean) {
        if (typeof value !== "boolean") throw new TypeError(`${label(this)}.visible: must be a boolean`);
        this.write("visible", value);
    }

    get opacity(): number {
        return this.read<number>("opacity");
    }
    set opacity(value: number) {
        this.write("opacity", finite(`${label(this)}.opacity`, value));
    }

    /** The node's own placement: its position, rotation and one uniform scale (the first component of `scale`). */
    get transform(): Transform3D {
        return new Transform3D(this.position, this.rotation, this.scale[0]);
    }
    /**
     * Places the node: `{ position, rotation, scale }` or a `Transform3D`. A part left out stays as it is. The parts are
     * checked together before any is written, and the writes go out in the frame's one transaction.
     */
    set transform(value: Transform3D | TransformInit) {
        const parts = transformFrom(`${label(this)}.transform`, value);
        if (parts.position !== null) this.position = parts.position;
        if (parts.rotation !== null) this.rotation = parts.rotation;
        if (parts.scale !== null) this.scale = parts.scale;
    }

    /** The node's position in its parent's units. A write takes a list or `{ x, y, z }` and sets `x`, `y` and `z`. */
    get position(): Vector3 {
        return new Vector3(this.x, this.y, this.z);
    }
    set position(value: Triple | Vec3Like) {
        const [x, y, z] = vectorFrom(`${label(this)}.position`, value);
        this.x = x;
        this.y = y;
        this.z = z;
    }

    /** The placement in the space of the topmost scene ancestor: the parents' placements composed in. */
    get worldTransform(): Transform3D {
        const above = this.parent;
        return above === null ? this.transform : above.worldTransform.multiply(this.transform);
    }

    /**
     * What a pointer that reaches the node does. `forward` (the default): the pointer hits the node and, for a `View`
     * of a client, goes on into the client, in the client's own surface coordinates; the shell is sent nothing for
     * the View. `capture`: the pointer hits the node and stops there; the shell is sent the node's pointer events
     * and the client none, so a script can make its own interaction over a client's picture. `none`: the pointer
     * misses the node, and its children are still tested. The tree holds this as two properties: `input` (set for
     * `forward` and `capture`) and `input_mode` (`forward` or `capture`). A write also takes `true` (`forward`) and
     * `false` (`none`).
     */
    get input(): InputMode {
        if (!this.read<boolean>("input")) {
            return "none";
        }
        return this.read<string>("input_mode") === "capture" ? "capture" : "forward";
    }
    set input(value: boolean | InputMode) {
        const mode = value === true ? "forward" : value === false ? "none" : value;
        if (mode !== "forward" && mode !== "capture" && mode !== "none") {
            throw new TypeError(`${label(this)}.input: must be "forward", "capture", "none" or a boolean`);
        }
        if (mode !== "none") {
            this.write("input_mode", mode);
        }
        this.write("input", mode !== "none");
    }

    /** Called when the pointer enters the node. Only a node the shell is sent edges for hears it: see `input`. */
    onPointerEnter?: (event: NodePointerEvent) => void;
    /** Called when the pointer moves over the node. */
    onPointerMotion?: (event: NodePointerEvent) => void;
    /** Called when the pointer leaves the node. */
    onPointerLeave?: (event: NodePointerEvent) => void;
    /** Called when a button goes down on the node. */
    onPointerPress?: (event: NodePointerEvent) => void;
    /** Called when a button comes up on the node. */
    onPointerRelease?: (event: NodePointerEvent) => void;

    /**
     * Metres in one unit of this node's space; 1 until set, as the specification's default is logical units. The
     * script keeps it: the tree has no such property for a plain node (a camera's own is `metres`).
     */
    get metresPerUnit(): number {
        return metresPerUnit.get(this) ?? 1;
    }
    set metresPerUnit(value: number) {
        if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) {
            throw new TypeError(`${label(this)}.metresPerUnit: must be a finite number above 0`);
        }
        metresPerUnit.set(this, value);
    }

    /** The reactive properties of the node: `node.properties.bind("opacity", () => ...)`. */
    get properties(): PropertyBindings {
        return propertiesOf(this, () => label(this));
    }

    /** Shorthand for `node.properties.bind`. */
    bind(property: string, compute: () => unknown): PropertyBinding {
        return this.properties.bind(property, compute);
    }

    /** The scene node this one is under, or null for one with none or one this module did not make. */
    get parent(): Node | null {
        const id = host().parent(nodeIdOf(this));
        return id === undefined ? null : (live.get(id) ?? null);
    }

    /** The children this module made, in tree order. A snapshot: adding to it changes nothing. */
    get children(): Node[] {
        const out: Node[] = [];
        for (const id of host().children(nodeIdOf(this))) {
            const child = live.get(id);
            if (child !== undefined) out.push(child);
        }
        return out;
    }

    /** Puts `child` last under this node, or at `index`. A child already in the tree moves. Returns it. */
    add<T extends Node>(child: T, index = -1): T {
        if (gone.has(this) || gone.has(child)) {
            throw new SceneError("not_found", `${label(this)}.add`, "the node was destroyed");
        }
        const code = host().insert(nodeIdOf(child), nodeIdOf(this), index);
        if (code !== undefined) {
            throw new SceneError(
                errorCodeOfRefusal(code),
                `${label(this)}.add`,
                `the shell refused to add ${label(child)} (${code})`,
            );
        }
        return child;
    }

    /** The first descendant this module made whose `name` is `name`, depth first. */
    find(name: string): Node | undefined {
        for (const child of this.children) {
            if (child.name === name) return child;
            const deeper = child.find(name);
            if (deeper !== undefined) return deeper;
        }
        return undefined;
    }

    /** Removes the node and everything under it from the tree. Destroying twice is not an error. */
    destroy(): void {
        if (gone.has(this)) {
            return;
        }
        const id = nodeIdOf(this);
        const all = [id];
        for (let at = 0; at < all.length; at++) {
            all.push(...host().children(all[at]));
        }
        const code = host().destroy(id);
        if (code !== undefined) {
            throw new SceneError(errorCodeOfRefusal(code), `${label(this)}.destroy`, `the shell refused (${code})`);
        }
        sceneForget(all);
    }

    toString(): string {
        return label(this);
    }
}
