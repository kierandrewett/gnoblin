// A camera is a scene node that defines projection settings and the real-world scale for its view.

import { Node } from "gnoblin-std/scene/node";
import type { NodeOptions } from "gnoblin-std/scene/node";
import { label } from "gnoblin-std/scene/registry";
import { finite } from "gnoblin-core/math/vector";

/** How a camera maps its scene into an image. */
export type Projection = "orthographic" | "perspective" | "stereo";

/** Initial projection and clipping values for a camera; near and far use camera-space units. */
export interface Camera3DOptions extends NodeOptions {
    /** Projection mode; defaults come from the host. */
    projection?: Projection;
    /** Metres per unit of the camera's space. */
    metres?: number;
    /** Near clipping distance in camera-space units. */
    near?: number;
    /** Far clipping distance in camera-space units. */
    far?: number;
    /** The vertical field of view in degrees. */
    fov?: number;
    /** The distance between a stereo camera's eyes, in metres. */
    ipd?: number;
    /** Camera position, using the same coordinates as `x`, `y` and `z`. */
    position?: { x: number; y: number; z: number };
    /** The visible height in scene units of an orthographic camera. */
    size?: number;
}

/** Keeps the orthographic height because the tree has no property for it. */
const cameraSizes = new WeakMap<object, number>();

/** A scene node that projects its children. A `View` draws content through its camera. */
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

    /** Visible orthographic height in scene units; defaults to 1000 and must be greater than zero. */
    get size(): number {
        return cameraSizes.get(this) ?? 1000;
    }
    /** Sets the orthographic height; it must be greater than zero. */
    set size(value: number) {
        if (finite(`${label(this)}.size`, value) <= 0) {
            throw new RangeError(`${label(this)}.size: must be above 0`);
        }
        cameraSizes.set(this, value);
    }

    /** Returns the active projection mode: `orthographic`, `perspective` or `stereo`. */
    get projection(): Projection {
        return this.read<Projection>("projection");
    }
    /** Sets the projection mode. */
    set projection(value: Projection) {
        if (value !== "orthographic" && value !== "perspective" && value !== "stereo") {
            throw new TypeError(`${label(this)}.projection: must be "orthographic", "perspective" or "stereo"`);
        }
        this.write("projection", value);
    }

    /** Returns the camera's metres per scene unit. */
    get metres(): number {
        return this.read<number>("metres");
    }
    /** Sets the camera's metres per scene unit. */
    set metres(value: number) {
        this.write("metres", finite(`${label(this)}.metres`, value));
    }

    /** Returns the near clipping distance in camera-space units. */
    get near(): number {
        return this.read<number>("near");
    }
    /** Sets the near clipping distance in camera-space units. */
    set near(value: number) {
        this.write("near", finite(`${label(this)}.near`, value));
    }

    /** Returns the far clipping distance in camera-space units. */
    get far(): number {
        return this.read<number>("far");
    }
    /** Sets the far clipping distance in camera-space units. */
    set far(value: number) {
        this.write("far", finite(`${label(this)}.far`, value));
    }

    /** Returns the vertical field of view in degrees. */
    get fov(): number {
        return this.read<number>("fov");
    }
    /** Sets the vertical field of view in degrees. */
    set fov(value: number) {
        this.write("fov", finite(`${label(this)}.fov`, value));
    }

    /** Returns the stereo eye spacing in metres. */
    get ipd(): number {
        return this.read<number>("ipd");
    }
    /** Sets the stereo eye spacing in metres. */
    set ipd(value: number) {
        this.write("ipd", finite(`${label(this)}.ipd`, value));
    }

    /** Number of layers the headset can carry in one frame, or 0 before the XR program reports a value. */
    get layerBudget(): number {
        return this.read<number>("layer_budget");
    }
}
