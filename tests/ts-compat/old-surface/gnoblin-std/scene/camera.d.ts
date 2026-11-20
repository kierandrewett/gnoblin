import { Node } from "gnoblin-std/scene/node";
import type { NodeOptions } from "gnoblin-std/scene/node";
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
    position?: {
        x: number;
        y: number;
        z: number;
    };
    /** The visible height in scene units of an orthographic camera. */
    size?: number;
}
/** A scene node that projects its children. A `View` draws content through its camera. */
export declare class Camera3D extends Node {
    constructor(options?: Camera3DOptions, cls?: string);
    protected assign(options: Camera3DOptions): void;
    /** Visible orthographic height in scene units; defaults to 1000 and must be greater than zero. The library keeps it: the tree has no property for it. */
    get size(): number;
    /** Sets the orthographic height; it must be greater than zero. */
    set size(value: number);
    /** Returns the active projection mode: `orthographic`, `perspective` or `stereo`. */
    get projection(): Projection;
    /** Sets the projection mode. */
    set projection(value: Projection);
    /** Returns the camera's metres per scene unit. */
    get metres(): number;
    /** Sets the camera's metres per scene unit. */
    set metres(value: number);
    /** Returns the near clipping distance in camera-space units. */
    get near(): number;
    /** Sets the near clipping distance in camera-space units. */
    set near(value: number);
    /** Returns the far clipping distance in camera-space units. */
    get far(): number;
    /** Sets the far clipping distance in camera-space units. */
    set far(value: number);
    /** Returns the vertical field of view in degrees. */
    get fov(): number;
    /** Sets the vertical field of view in degrees. */
    set fov(value: number);
    /** Returns the stereo eye spacing in metres. */
    get ipd(): number;
    /** Sets the stereo eye spacing in metres. */
    set ipd(value: number);
    /** Number of layers the headset can carry in one frame, or 0 before the XR program reports a value. */
    get layerBudget(): number;
}
