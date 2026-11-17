// The types of example 5. A type-only module: nothing of it exists once the types are erased, except the enum.

/** A rectangle in logical pixels. */
export interface Area {
    x: number;
    y: number;
    width: number;
    height: number;
}

/** How a set of windows is divided. */
export type Arrangement = "columns" | "rows";

/** The space left between two windows, in pixels. */
export enum Gap {
    None = 0,
    Small = 8,
    Large = 24,
}
