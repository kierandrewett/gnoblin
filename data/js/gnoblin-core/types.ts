// Shared types for host modules: node handles, property values and compositor-coordinate rectangles.

/** The host tree node handle, represented as a JavaScript number by the shell bridge. */
export type NodeId = number;

/** A property scalar accepted by the host bridge. */
export type Scalar = string | number | boolean;

/** What a property holds: a scalar, the three numbers of a rotation or scale, or a list of names (a claim list). */
export type HostValue = Scalar | number[] | string[];

/** A rectangle in the compositor's logical coordinates. */
export interface Rect {
    /** Horizontal position in logical pixels. */
    x: number;
    /** Vertical position in logical pixels. */
    y: number;
    /** Width in logical pixels. */
    width: number;
    /** Height in logical pixels. */
    height: number;
}

/** The data-only selection `where` takes: each key is a property, each value what it must equal. */
export type Query = Record<string, Scalar | Scalar[]>;
