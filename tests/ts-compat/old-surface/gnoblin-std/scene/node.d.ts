import type { PropertyBinding, PropertyBindings } from "gnoblin-core/properties";
import type { HostValue, Scalar } from "gnoblin-std/scene/backend";
import { Transform3D } from "gnoblin-core/math/transform";
import { Vector3 } from "gnoblin-core/math/vector";
import type { AxisTriple, RotationInit, ScaleInit, TransformInit, Triple, Vec3Like } from "gnoblin-core/math/spatial";
/** How a pointer that hits a node is handled: forwarded to its content, captured by the node, or ignored. */
export type InputMode = "forward" | "capture" | "none";
/** A pointer edge sent to a node handler. The point uses the node's own plane and units. */
export interface NodePointerEvent {
    /** The node the pointer is on. */
    readonly node: Node;
    /** X coordinate in the node's plane. */
    readonly x: number;
    /** Y coordinate in the node's plane. */
    readonly y: number;
    /** The button number from the kernel on press and release; 272 is the left button. Other edges use 0. */
    readonly button: number;
}
/** Initial node values. Position uses the parent's units; rotation uses radians. */
export interface NodeOptions {
    /** A stable name for the node, also returned by `node.id` and `node.name`. */
    id?: string;
    /** Optional display name, defaulting to the id when one is given. */
    name?: string;
    /** X position in the parent's units. */
    x?: number;
    /** Y position in the parent's units. */
    y?: number;
    /** Z position in the parent's units. */
    z?: number;
    /** Position in the parent's units. */
    position?: Triple | Vec3Like;
    /** Angles about X, Y and Z in radians as a list or `{ x, y, z }`, or a quaternion. */
    rotation?: RotationInit;
    /** One number for every axis, or a value per axis. */
    scale?: ScaleInit;
    /** Written after `position`, `rotation` and `scale`; a part it leaves out is not written. */
    transform?: TransformInit;
    /** Whether the node is visible. */
    visible?: boolean;
    /** Opacity from 0 (transparent) to 1 (opaque). */
    opacity?: number;
    /** Pointer handling mode, or a boolean shorthand. */
    input?: boolean | InputMode;
}
/** A node in the scene tree. Add children with `add`; destroying a node also destroys its children. */
export declare class Node {
    constructor(options?: NodeOptions, cls?: string);
    /**
     * Writes the options a constructor was given; a subclass adds its own. Placement goes through the setters, so a
     * subclass that redefines `position` or `rotation` gets its own conversion.
     */
    protected assign(options: NodeOptions): void;
    /** Whether the node is still in the tree. A destroyed node rejects every write. */
    get alive(): boolean;
    /** The id the node was made with (`new Node({ id })`), or else the tree's id as `gnoblinctl` prints it. */
    get id(): string;
    protected read<T extends Scalar | number[]>(key: string): T;
    protected write(key: string, value: HostValue): void;
    /** Returns the node's display name. */
    get name(): string;
    /** Sets the display name. */
    set name(value: string);
    /** X position in the parent's units. */
    get x(): number;
    /** Sets the X position in the parent's units. */
    set x(value: number);
    /** Y position in the parent's units. */
    get y(): number;
    /** Sets the Y position in the parent's units. */
    set y(value: number);
    /** Z position in the parent's units. */
    get z(): number;
    /** Sets the Z position in the parent's units. */
    set z(value: number);
    /**
     * The angles about X, Y and Z, in radians (turned about X, then Y, then Z). The list also answers to `x`, `y` and
     * `z`. A write takes a list, `{ x, y, z }` or a quaternion `{ x, y, z, w }`, which is stored as its Euler angles.
     */
    get rotation(): AxisTriple;
    /** Sets rotation from angles in radians or a quaternion. */
    set rotation(value: RotationInit);
    /** The scale along each axis; the list also answers to `x`, `y` and `z`. A write takes one number for all axes. */
    get scale(): AxisTriple;
    /** Sets the scale along each axis. */
    set scale(value: ScaleInit);
    /** Whether the node is visible. */
    get visible(): boolean;
    /** Sets whether the node is visible. */
    set visible(value: boolean);
    /** Opacity from 0 (transparent) to 1 (opaque). */
    get opacity(): number;
    /** Sets opacity from 0 (transparent) to 1 (opaque). */
    set opacity(value: number);
    /** The node's own placement: its position, rotation and one uniform scale (the first component of `scale`). */
    get transform(): Transform3D;
    /**
     * Places the node: `{ position, rotation, scale }` or a `Transform3D`. A part left out stays as it is. The parts
     * are checked together before any is written, and the writes go out in the frame's one transaction.
     */
    set transform(value: Transform3D | TransformInit);
    /** The node's position in its parent's units. A write takes a list or `{ x, y, z }` and sets `x`, `y` and `z`. */
    get position(): Vector3;
    /** Sets position from a vector or three coordinates in the parent's units. */
    set position(value: Triple | Vec3Like);
    /** The placement in the space of the topmost scene ancestor: the parents' placements composed in. */
    get worldTransform(): Transform3D;
    /**
     * What a pointer that reaches the node does. `forward` (the default): the pointer hits the node and, for a `View`
     * of a client, goes on into the client, in the client's own surface coordinates; the shell is sent nothing for
     * the View. `capture`: the pointer hits the node and stops there; the shell is sent the node's pointer events
     * and the client none, so a script can make its own interaction over a client's picture. `none`: the pointer
     * misses the node, and its children are still tested. The tree holds this as two properties: `input` (set for
     * `forward` and `capture`) and `input_mode` (`forward` or `capture`). A write also takes `true` (`forward`) and
     * `false` (`none`).
     */
    get input(): InputMode;
    /** Sets pointer handling to `forward`, `capture` or `none`. */
    set input(value: boolean | InputMode);
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
     * Metres in one unit of this node's space. Defaults to 1 and stays in the script because a plain node has no such
     * tree property; a camera has its own `metres` value.
     */
    get metresPerUnit(): number;
    /** Sets metres per unit; the value must be greater than zero. */
    set metresPerUnit(value: number);
    /** The reactive properties of the node: `node.properties.bind("opacity", () => ...)`. */
    get properties(): PropertyBindings;
    /** Shorthand for `node.properties.bind`. */
    bind(property: string, compute: () => unknown): PropertyBinding;
    /** The scene node this one is under, or null for one with none or one this module did not make. */
    get parent(): Node | null;
    /** The children this module made, in tree order. A snapshot: adding to it changes nothing. */
    get children(): Node[];
    /** Puts `child` last under this node, or at `index`. A child already in the tree moves. Returns it. */
    add<T extends Node>(child: T, index?: number): T;
    /** The first descendant this module made whose `name` is `name`, depth first. */
    find(name: string): Node | undefined;
    /** Removes the node and everything under it from the tree. Destroying twice is not an error. */
    destroy(): void;
    /** Returns the node's class and host id. */
    toString(): string;
}
