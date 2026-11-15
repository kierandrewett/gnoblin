// What `Client.animate` takes: the properties of a client it can move, the values and the duration and easing.

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
    x?: number;
    y?: number;
    width?: number;
    height?: number;
    opacity?: number;
    z?: number;
    rotation?: Triple;
    scale?: Triple;
}

/** Duration and easing options for a client animation. */
export interface AnimateOptions {
    /** Milliseconds. Zero is an instant write. */
    duration: number;
    /** One of the CSS names of `Easing`. Default `"ease-out"`. */
    easing?: Easing;
}
