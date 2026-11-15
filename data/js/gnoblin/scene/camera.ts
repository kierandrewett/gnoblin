// `Camera3D`: the node the XR program renders the room from, with its projection and the metres in one unit of the
// space it looks into.

import { Node } from "gnoblin/scene/node";
import type { NodeOptions } from "gnoblin/scene/node";
import { label } from "gnoblin/scene/registry";
import { finite } from "gnoblin/scene/vector";

/** How a camera maps its scene into the image. */
export type Projection = "orthographic" | "perspective" | "stereo";

/** Initial projection and clipping values for a camera; near and far use camera-space units. */
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
    /** Where the camera sits, as the specification writes it (the same as `x`, `y` and `z`). */
    position?: { x: number; y: number; z: number };
    /** The visible height in scene units of an orthographic camera. */
    size?: number;
}

/** The `size` of each camera: the tree has no property for it, so a script's camera holds it. */
const cameraSizes = new WeakMap<object, number>();

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
        if (options.size !== undefined) this.size = options.size;
        if (options.position !== undefined) {
            this.x = finite(`${label(this)}.position.x`, options.position.x);
            this.y = finite(`${label(this)}.position.y`, options.position.y);
            this.z = finite(`${label(this)}.position.z`, options.position.z);
        }
    }

    /** The visible height of an orthographic camera, in scene units. The shell keeps it; nothing draws from it yet. */
    get size(): number {
        return cameraSizes.get(this) ?? 1000;
    }
    set size(value: number) {
        if (finite(`${label(this)}.size`, value) <= 0) {
            throw new RangeError(`${label(this)}.size: must be above 0`);
        }
        cameraSizes.set(this, value);
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
