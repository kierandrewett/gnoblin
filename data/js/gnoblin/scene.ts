// Scene nodes a script makes: `Node`, `Camera3D` and `View` (owner-spec.md section 8, the part that exists today).
//
// A `Client` is the compositor's: a script cannot make one. These three are the script's own, and the XR program
// (design/js-xr.md) reads them: a room is a `Node`, the head is a `Camera3D`, and each panel is a `View` that shows
// a client. Making an object makes the tree node at once (queued with the frame's other writes), and `destroy()`
// removes it with its subtree. A `View` names its client in `content` and moves nothing: the client stays in its
// workspace, and one client may be shown by any number of Views.
//
// The module reaches the tree only through `SceneBackend`, which the host module installs with `attachScene`, so it
// runs (and is tested) without a host.

type NodeId = number;
type Scalar = string | number | boolean;
type HostValue = Scalar | number[] | string[];

/** What the scene classes need of the host. The shell's Rust host implements it. */
export interface SceneBackend {
    /** Makes a node of `class` (`Node`, `Camera` or `View`): its id, or the refusal code as text. */
    create(cls: string): NodeId | string;
    /** Puts `node` under `parent` at `index` (negative is last); a node already in the tree moves. */
    insert(node: NodeId, parent: NodeId, index: number): string | undefined;
    destroy(node: NodeId): string | undefined;
    children(node: NodeId): NodeId[];
    parent(node: NodeId): NodeId | undefined;
    /** The root of the tree: where a room is mounted. */
    root(): NodeId;
    get(node: NodeId, key: string): HostValue | undefined;
    set(node: NodeId, key: string, value: HostValue): string | undefined;
    /** The node a client is, or undefined for an object that is not one. */
    nodeOf(content: object): NodeId | undefined;
    /** The client a node is, or undefined. */
    contentOf(node: NodeId): object | undefined;
}

/** What a failed operation on a scene node throws. `code` says why. */
export class SceneError extends Error {
    readonly code: string;
    readonly target: string;

    constructor(code: string, target: string, message: string) {
        super(`${target}: ${message}`);
        this.name = "SceneError";
        this.code = code;
        this.target = target;
    }
}

let backend: SceneBackend | null = null;

/** Gives the module its backend. The host module calls it once, before any user code runs. */
export function attachScene(next: SceneBackend): void {
    backend = next;
}

function host(): SceneBackend {
    if (backend === null) {
        throw new Error("the scene needs the shell host and cannot run without it");
    }
    return backend;
}

/** Three numbers: rotation about each axis in radians, or the scale along each axis. */
export type Triple = readonly [number, number, number];

export interface NodeOptions {
    name?: string;
    x?: number;
    y?: number;
    z?: number;
    rotation?: Triple;
    scale?: Triple;
    visible?: boolean;
    opacity?: number;
    input?: boolean;
}

/** Puts `node` last under the tree's root, which is where the XR program looks for the room. A node in the tree stays. */
export function mountOnRoot(node: Node): void {
    if (host().parent(nodeIdOf(node)) !== undefined) {
        return;
    }
    const code = host().insert(nodeIdOf(node), host().root(), -1);
    if (code !== undefined) {
        throw new SceneError(code, `${label(node)}.mount`, `the shell refused to mount the node (${code})`);
    }
}

/** The node a script object stands for, for the objects of this module. */
const nodes = new WeakMap<object, NodeId>();
/** Every live scene object by node id, so a tree node reached by id is the same object as the one that made it. */
const live = new Map<NodeId, Node>();
const gone = new WeakSet<object>();

/** The tree node of a scene object. */
export function nodeIdOf(object: Node): NodeId {
    const id = nodes.get(object);
    if (id === undefined) {
        throw new TypeError("not a scene node");
    }
    return id;
}

/** The scene object for node `id`, or undefined when this module did not make it. */
export function sceneNode(id: NodeId): Node | undefined {
    return live.get(id);
}

/** Tells the module nodes were destroyed by someone else (the compositor, a destroyed ancestor). */
export function sceneForget(ids: readonly NodeId[]): void {
    for (const id of ids) {
        const object = live.get(id);
        if (object !== undefined) {
            live.delete(id);
            gone.add(object);
        }
    }
}

/**
 * Destroys every node this generation made that has no scene parent of its own, with their subtrees. The host calls
 * it when the generation ends, so a reload does not leave the old generation's room and Views behind.
 */
export function sceneReleaseAll(): void {
    for (const [id, object] of [...live]) {
        if (!live.has(id) || object.parent !== null) {
            continue;
        }
        try {
            object.destroy();
        } catch {
            // The node is gone already, or the shell refused: nothing is left to release.
        }
    }
}

function label(object: Node): string {
    return `${object.constructor.name}#${nodes.get(object) ?? "?"}`;
}

function finite(target: string, value: unknown): number {
    if (typeof value !== "number" || !Number.isFinite(value)) {
        throw new TypeError(`${target}: must be a finite number`);
    }
    return value;
}

function triple(target: string, value: unknown): number[] {
    if (!Array.isArray(value) || value.length !== 3) {
        throw new TypeError(`${target}: must be three numbers`);
    }
    return value.map((part) => finite(target, part));
}

// ---- Space: vectors, poses, rays and planes ----------------------------------------------------------------------
//
// A pose is a position, a rotation and one uniform scale, as the XR program places a View (it reads only the first
// component of a node's `scale`). A rotation is the node's own: Euler angles in radians, turned about X, then Y,
// then Z, so the rotation matrix is `Rz * Ry * Rx` (`xr_scene::math::euler_degrees`). Positions are in the units of
// the node's parent; the XR room's unit is the millimetre.

/** A point or a direction. */
export class Vector3 {
    x: number;
    y: number;
    z: number;

    constructor(x = 0, y = 0, z = 0) {
        this.x = x;
        this.y = y;
        this.z = z;
    }

    static zero(): Vector3 {
        return new Vector3();
    }

    static from(parts: readonly number[]): Vector3 {
        return new Vector3(parts[0] ?? 0, parts[1] ?? 0, parts[2] ?? 0);
    }

    clone(): Vector3 {
        return new Vector3(this.x, this.y, this.z);
    }

    add(other: Vector3): Vector3 {
        return new Vector3(this.x + other.x, this.y + other.y, this.z + other.z);
    }

    subtract(other: Vector3): Vector3 {
        return new Vector3(this.x - other.x, this.y - other.y, this.z - other.z);
    }

    scale(factor: number): Vector3 {
        return new Vector3(this.x * factor, this.y * factor, this.z * factor);
    }

    dot(other: Vector3): number {
        return this.x * other.x + this.y * other.y + this.z * other.z;
    }

    cross(other: Vector3): Vector3 {
        return new Vector3(
            this.y * other.z - this.z * other.y,
            this.z * other.x - this.x * other.z,
            this.x * other.y - this.y * other.x,
        );
    }

    length(): number {
        return Math.hypot(this.x, this.y, this.z);
    }

    /** The unit vector the same way, or zero for a zero vector. */
    normalize(): Vector3 {
        const length = this.length();
        return length === 0 ? new Vector3() : this.scale(1 / length);
    }

    distanceTo(other: Vector3): number {
        return this.subtract(other).length();
    }

    toArray(): [number, number, number] {
        return [this.x, this.y, this.z];
    }
}

/** A half line: from `origin` along `direction`, which need not be a unit vector. */
export class Ray {
    readonly origin: Vector3;
    readonly direction: Vector3;

    constructor(origin: Vector3, direction: Vector3) {
        this.origin = origin;
        this.direction = direction;
    }

    /** The point `distance` along the ray, in units of `direction`'s length. */
    at(distance: number): Vector3 {
        return this.origin.add(this.direction.scale(distance));
    }
}

/** A plane through `point` with the given `normal`. */
export class Plane {
    readonly point: Vector3;
    readonly normal: Vector3;

    constructor(point: Vector3, normal: Vector3) {
        this.point = point;
        this.normal = normal;
    }

    /** Where `ray` crosses the plane, or null when it is parallel to it or the plane is behind its origin. */
    intersect(ray: Ray): Vector3 | null {
        const denominator = this.normal.dot(ray.direction);
        if (Math.abs(denominator) < 1e-12) {
            return null;
        }
        const t = this.normal.dot(this.point.subtract(ray.origin)) / denominator;
        return t > 0 ? ray.at(t) : null;
    }
}

function product(a: readonly number[], b: readonly number[]): number[] {
    const out = new Array<number>(9).fill(0);
    for (let row = 0; row < 3; row++) {
        for (let col = 0; col < 3; col++) {
            out[row * 3 + col] = a[row * 3] * b[col] + a[row * 3 + 1] * b[3 + col] + a[row * 3 + 2] * b[6 + col];
        }
    }
    return out;
}

function matrixOf(euler: readonly number[]): number[] {
    const [a, b, c] = euler;
    const [sa, ca, sb, cb, sc, cc] = [Math.sin(a), Math.cos(a), Math.sin(b), Math.cos(b), Math.sin(c), Math.cos(c)];
    // Rz * Ry * Rx, written out.
    return [
        cc * cb,
        cc * sb * sa - sc * ca,
        cc * sb * ca + sc * sa,
        sc * cb,
        sc * sb * sa + cc * ca,
        sc * sb * ca - cc * sa,
        -sb,
        cb * sa,
        cb * ca,
    ];
}

/** The angles about X, Y and Z that `matrixOf` turns into `m`. At a quarter turn about Y the turn about Z is zero. */
function eulerOf(m: readonly number[]): [number, number, number] {
    const b = Math.asin(Math.max(-1, Math.min(1, -m[6])));
    if (Math.abs(m[6]) < 1 - 1e-9) {
        return [Math.atan2(m[7], m[8]), b, Math.atan2(m[3], m[0])];
    }
    return m[6] < 0 ? [Math.atan2(m[1], m[2]), b, 0] : [Math.atan2(-m[1], -m[2]), b, 0];
}

/** A pose: a position, a rotation and one uniform scale. `a.multiply(b)` is `b` placed inside `a`. */
export class Transform3D {
    position: Vector3;
    scale: number;
    /** Row-major 3x3 rotation matrix. */
    #matrix: number[];

    constructor(position = new Vector3(), rotation: readonly number[] = [0, 0, 0], scale = 1) {
        this.position = position;
        this.scale = scale;
        this.#matrix = matrixOf(rotation);
    }

    static identity(): Transform3D {
        return new Transform3D();
    }

    static translation(x: number, y: number, z: number): Transform3D {
        return new Transform3D(new Vector3(x, y, z));
    }

    /** The rotation as Euler angles in radians about X, Y and Z. */
    get rotation(): Vector3 {
        return Vector3.from(eulerOf(this.#matrix));
    }

    set rotation(value: Vector3 | readonly number[]) {
        this.#matrix = matrixOf(value instanceof Vector3 ? value.toArray() : value);
    }

    clone(): Transform3D {
        const copy = new Transform3D(this.position.clone(), [0, 0, 0], this.scale);
        copy.#matrix = [...this.#matrix];
        return copy;
    }

    /** `other` placed inside `this`: the point `p` of `other` is `this.transformPoint(other.transformPoint(p))`. */
    multiply(other: Transform3D): Transform3D {
        const result = new Transform3D(this.transformPoint(other.position), [0, 0, 0], this.scale * other.scale);
        result.#matrix = product(this.#matrix, other.#matrix);
        return result;
    }

    inverse(): Transform3D {
        const m = this.#matrix;
        const transposed = [m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8]];
        const scale = 1 / this.scale;
        const back = new Transform3D(new Vector3(), [0, 0, 0], scale);
        back.#matrix = transposed;
        back.position = back.transformDirection(this.position).scale(-scale);
        return back;
    }

    /** A direction turned by the rotation alone. */
    transformDirection(direction: Vector3): Vector3 {
        const m = this.#matrix;
        const { x, y, z } = direction;
        return new Vector3(
            m[0] * x + m[1] * y + m[2] * z,
            m[3] * x + m[4] * y + m[5] * z,
            m[6] * x + m[7] * y + m[8] * z,
        );
    }

    transformPoint(point: Vector3): Vector3 {
        return this.position.add(this.transformDirection(point.scale(this.scale)));
    }

    /** The plane of a quad that sits at this pose: through the position, facing along the local +Z. */
    plane(): Plane {
        return new Plane(this.position.clone(), this.transformDirection(new Vector3(0, 0, 1)));
    }
}

/** A plain node of the scene: the XR room is one. Children are added with `add` and go with it when it is destroyed. */
export class Node {
    constructor(options: NodeOptions = {}, cls = "Node") {
        const made = host().create(cls);
        if (typeof made !== "number") {
            throw new SceneError(made, cls, `the shell could not make a ${cls} (${made})`);
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

    /** Writes the options a constructor was given; a subclass adds its own. */
    protected assign(options: NodeOptions): void {
        if (options.name !== undefined) this.name = options.name;
        if (options.x !== undefined) this.x = options.x;
        if (options.y !== undefined) this.y = options.y;
        if (options.z !== undefined) this.z = options.z;
        if (options.rotation !== undefined) this.rotation = options.rotation;
        if (options.scale !== undefined) this.scale = options.scale;
        if (options.visible !== undefined) this.visible = options.visible;
        if (options.opacity !== undefined) this.opacity = options.opacity;
        if (options.input !== undefined) this.input = options.input;
    }

    /** Whether the node is still in the tree. A destroyed node rejects every write. */
    get alive(): boolean {
        return !gone.has(this);
    }

    /** The id as `gnoblinctl` prints it. */
    get id(): string {
        return String(nodeIdOf(this));
    }

    protected read<T extends Scalar | number[]>(key: string): T {
        return host().get(nodeIdOf(this), key) as T;
    }

    protected write(key: string, value: HostValue): void {
        if (gone.has(this)) {
            throw new SceneError("destroyed", `${label(this)}.${key}`, "the node was destroyed");
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
            throw new SceneError(code, `${label(this)}.${key}`, `the shell refused the write (${code})`);
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

    /** Rotation about each axis, in radians. */
    get rotation(): Triple {
        return this.read<number[]>("rotation") as unknown as Triple;
    }
    set rotation(value: Triple) {
        this.write("rotation", triple(`${label(this)}.rotation`, value));
    }

    /** The scale along each axis. */
    get scale(): Triple {
        return this.read<number[]>("scale") as unknown as Triple;
    }
    set scale(value: Triple) {
        this.write("scale", triple(`${label(this)}.scale`, value));
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
        const [x, y, z] = [this.x, this.y, this.z];
        return new Transform3D(new Vector3(x, y, z), this.rotation, this.scale[0]);
    }
    set transform(value: Transform3D) {
        this.x = value.position.x;
        this.y = value.position.y;
        this.z = value.position.z;
        this.rotation = value.rotation.toArray();
        this.scale = [value.scale, value.scale, value.scale];
    }

    /** The placement in the space of the topmost scene ancestor: the parents' placements composed in. */
    get worldTransform(): Transform3D {
        const above = this.parent;
        return above === null ? this.transform : above.worldTransform.multiply(this.transform);
    }

    /** Whether a pointer ray can hit the node (the tree's `input`). */
    get input(): boolean {
        return this.read<boolean>("input");
    }
    set input(value: boolean) {
        if (typeof value !== "boolean") throw new TypeError(`${label(this)}.input: must be a boolean`);
        this.write("input", value);
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
            throw new SceneError("destroyed", `${label(this)}.add`, "the node was destroyed");
        }
        const code = host().insert(nodeIdOf(child), nodeIdOf(this), index);
        if (code !== undefined) {
            throw new SceneError(code, `${label(this)}.add`, `the shell refused to add ${label(child)} (${code})`);
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
            throw new SceneError(code, `${label(this)}.destroy`, `the shell refused (${code})`);
        }
        sceneForget(all);
    }

    toString(): string {
        return label(this);
    }
}

export type Projection = "orthographic" | "perspective" | "stereo";

export interface Camera3DOptions extends NodeOptions {
    projection?: Projection;
    /** Metres per unit of the camera's space. */
    metres?: number;
    near?: number;
    far?: number;
    /** The vertical field of view in degrees. */
    fov?: number;
    /** The distance between a stereo camera's eyes, in metres. */
    ipd?: number;
}

/** A camera: a space that projects its children. A `View` draws its content through the camera it names. */
export class Camera3D extends Node {
    constructor(options: Camera3DOptions = {}, cls = "Camera") {
        super(options, cls);
    }

    protected override assign(options: Camera3DOptions): void {
        super.assign(options);
        if (options.projection !== undefined) this.projection = options.projection;
        if (options.metres !== undefined) this.metres = options.metres;
        if (options.near !== undefined) this.near = options.near;
        if (options.far !== undefined) this.far = options.far;
        if (options.fov !== undefined) this.fov = options.fov;
        if (options.ipd !== undefined) this.ipd = options.ipd;
    }

    /** `orthographic`, `perspective` or `stereo`. */
    get projection(): Projection {
        return this.read<Projection>("projection");
    }
    set projection(value: Projection) {
        if (value !== "orthographic" && value !== "perspective" && value !== "stereo") {
            throw new TypeError(`${label(this)}.projection: must be "orthographic", "perspective" or "stereo"`);
        }
        this.write("projection", value);
    }

    get metres(): number {
        return this.read<number>("metres");
    }
    set metres(value: number) {
        this.write("metres", finite(`${label(this)}.metres`, value));
    }

    get near(): number {
        return this.read<number>("near");
    }
    set near(value: number) {
        this.write("near", finite(`${label(this)}.near`, value));
    }

    get far(): number {
        return this.read<number>("far");
    }
    set far(value: number) {
        this.write("far", finite(`${label(this)}.far`, value));
    }

    get fov(): number {
        return this.read<number>("fov");
    }
    set fov(value: number) {
        this.write("fov", finite(`${label(this)}.fov`, value));
    }

    get ipd(): number {
        return this.read<number>("ipd");
    }
    set ipd(value: number) {
        this.write("ipd", finite(`${label(this)}.ipd`, value));
    }

    /** How many layers one frame of the headset may carry, as the XR program reports it; 0 until it has. */
    get layerBudget(): number {
        return this.read<number>("layer_budget");
    }
}

export interface ViewOptions extends NodeOptions {
    /** The client the View shows. */
    content?: object | null;
    /** The camera the content is drawn through. Null is the flat default. */
    camera?: Camera3D | null;
    /** The View's own rectangle, in pixels: the size its picture is drawn at. */
    width?: number;
    height?: number;
}

/**
 * A node that draws another node as a picture. `content` names a client and the client is not reparented or moved:
 * a View is a second, separate way to see it. A workspace is not a tree node in this shell, so it cannot be a
 * View's content yet.
 */
export class View extends Node {
    constructor(options: ViewOptions = {}, cls = "View") {
        super(options, cls);
    }

    protected override assign(options: ViewOptions): void {
        super.assign(options);
        if (options.content !== undefined) this.content = options.content;
        if (options.camera !== undefined) this.camera = options.camera;
        if (options.width !== undefined) this.width = options.width;
        if (options.height !== undefined) this.height = options.height;
    }

    /** The client the View shows, or null when it shows none or its client is gone. */
    get content(): object | null {
        const id = this.read<number>("source");
        return id ? (host().contentOf(id) ?? null) : null;
    }
    set content(value: object | null) {
        if (value === null) {
            this.write("source", 0);
            return;
        }
        const id = typeof value === "object" ? host().nodeOf(value) : undefined;
        if (id === undefined) {
            throw new TypeError(
                `${label(this)}.content: must be a client (a workspace is not a tree node, so a View cannot show one yet)`,
            );
        }
        this.write("source", id);
    }

    /** Whether the View's client is gone: the tree clears `source` when its node is destroyed. */
    get orphaned(): boolean {
        return this.read<number>("source") === 0;
    }

    get camera(): Camera3D | null {
        const id = this.read<number>("camera");
        const found = id ? live.get(id) : undefined;
        return found instanceof Camera3D ? found : null;
    }
    set camera(value: Camera3D | null) {
        this.write("camera", value === null ? 0 : nodeIdOf(value));
    }

    /** The width of the View's picture in pixels. */
    get width(): number {
        return this.read<number>("width");
    }
    set width(value: number) {
        this.write("width", finite(`${label(this)}.width`, value));
    }

    get height(): number {
        return this.read<number>("height");
    }
    set height(value: number) {
        this.write("height", finite(`${label(this)}.height`, value));
    }
}
