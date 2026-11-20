import { Canvas } from "gnoblin-std/ui/canvas";
import type { CanvasOptions } from "gnoblin-std/ui/canvas";
import type { Color, PaintOp } from "gnoblin-std/ui/paint";
/** How a box places its children: in a `row`, in a `column`, or `none` (they stay where they are). */
export type Direction = "row" | "column" | "none";
/** Options for `Box`. Sizes, gaps and padding are pixels. */
export interface BoxOptions extends CanvasOptions {
    /** Layout direction; defaults to `none`. */
    direction?: Direction;
    /** Logical-pixel space between children; defaults to `0`. */
    gap?: number;
    /** Logical-pixel space around the children; defaults to `0`. */
    padding?: number;
    /** Relative share of remaining main-axis space; `0` keeps the child's own size. */
    grow?: number;
    /** Background colour; defaults to transparent. */
    fill?: Color;
    /** Corner radius in logical pixels; defaults to `0`. */
    radius?: number;
    /** Initial children, added in order. */
    children?: readonly Canvas[];
}
/**
 * A canvas that fills itself and lays its child boxes out: `direction` `"row"` or `"column"` is a small flexbox, the
 * main axis in order with `gap` between and `padding` around; a child with `grow` takes its share of what is left;
 * every child is stretched across the other axis. Children that are not boxes stay where they are.
 */
export declare class Box extends Canvas {
    /** How this box arranges its child boxes. Defaults to `none`. */
    direction: Direction;
    /** Logical-pixel space between arranged children. */
    gap: number;
    /** Logical-pixel space around arranged children. */
    padding: number;
    /** Relative share of remaining main-axis space; zero keeps the child's assigned size. */
    grow: number;
    /** Background colour. */
    fill: Color;
    /** Corner radius in logical pixels. */
    radius: number;
    constructor(options?: BoxOptions);
    /** Paints the box background as a rectangle. */
    draw(): PaintOp[];
    /** Arranges child boxes along the main axis and stretches them across the other axis. */
    arrange(): void;
}
