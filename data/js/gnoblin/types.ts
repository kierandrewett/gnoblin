// Types the host modules share: a node handle, what a property can hold, and a rectangle in the compositor's
// logical coordinates. A leaf module, so a type is imported from here and not from the module that happened to
// declare it first (it replaces the copies in `scene`, `properties` and `scene_mirror`).

/** The host tree node handle, represented as a JavaScript number by the shell bridge. */
export type NodeId = number;

/** A property scalar accepted by the host bridge. */
export type Scalar = string | number | boolean;

/** What a property holds: a scalar, the three numbers of a rotation or scale, or a list of names (a claim list). */
export type HostValue = Scalar | number[] | string[];

/** A rectangle in the compositor's logical coordinates. */
export interface Rect {
    x: number;
    y: number;
    width: number;
    height: number;
}

/** The data-only selection `where` takes: each key is a property, each value what it must equal. */
export type Query = Record<string, Scalar | Scalar[]>;
