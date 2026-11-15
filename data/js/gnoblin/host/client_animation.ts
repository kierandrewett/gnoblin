// Types and property names accepted by `Client.animate`.

import type { Triple } from "gnoblin/spatial";
import type { Easing } from "gnoblin/transition/easing";

/** The properties of a client `animate` can move, and the kind of value each takes. */
export const ANIMATED_KEYS: Readonly<Record<string, "number" | "vector">> = {
    x: "number",
    y: "number",
    width: "number",
    height: "number",
    opacity: "number",
    z: "number",
    rotation: "vector",
    scale: "vector",
};

/** What `Client.animate` takes: the values to move to, each of the kind its property holds. */
export interface ClientAnimation {
    /** Horizontal position in logical pixels. */
    x?: number;
    /** Vertical position in logical pixels. */
    y?: number;
    /** Width in logical pixels. */
    width?: number;
    /** Height in logical pixels. */
    height?: number;
    /** Opacity from 0 (transparent) to 1 (opaque). */
    opacity?: number;
    /** Depth; negative values place the window in front of the camera. */
    z?: number;
    /** Rotation about the x, y and z axes, in radians. */
    rotation?: Triple;
    /** Scale along the x, y and z axes. */
    scale?: Triple;
}

/** Duration and easing options for a client animation. */
export interface AnimateOptions {
    /** Milliseconds. Zero is an instant write. */
    duration: number;
    /** One of the CSS names of `Easing`. Default `"ease-out"`. */
    easing?: Easing;
}
