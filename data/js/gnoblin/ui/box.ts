// Canvas containers that paint a background and arrange children in rows or columns.

import { Canvas } from "gnoblin/ui/canvas";
import type { CanvasOptions } from "gnoblin/ui/canvas";
import type { Color, PaintOp } from "gnoblin/ui/paint";

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
export class Box extends Canvas {
    /** How this box arranges its child boxes. Defaults to `none`. */
    direction: Direction = "none";
    /** Logical-pixel space between arranged children. */
    gap = 0;
    /** Logical-pixel space around arranged children. */
    padding = 0;
    /** Relative share of remaining main-axis space; zero keeps the child's assigned size. */
    grow = 0;
    /** Background colour. */
    fill: Color = "#00000000";
    /** Corner radius in logical pixels. */
    radius = 0;

    constructor(options: BoxOptions = {}) {
        super(options);
        if (options.direction !== undefined) this.direction = options.direction;
        if (options.gap !== undefined) this.gap = options.gap;
        if (options.padding !== undefined) this.padding = options.padding;
        if (options.grow !== undefined) this.grow = options.grow;
        if (options.fill !== undefined) this.fill = options.fill;
        if (options.radius !== undefined) this.radius = options.radius;
        for (const child of options.children ?? []) {
            this.add(child);
        }
    }

    /** Paints the box background as a rectangle. */
    override draw(): PaintOp[] {
        return [
            { op: "rect", x: 0, y: 0, width: this.width, height: this.height, radius: this.radius, fill: this.fill },
        ];
    }

    /** Arranges child boxes along the main axis and stretches them across the other axis. */
    override arrange(): void {
        if (this.direction === "none") {
            return;
        }
        const column = this.direction === "column";
        const kids = this.children.filter((child): child is Box => child instanceof Box);
        const size = (box: Box): number => (column ? box.height : box.width);
        const pad = this.padding;
        let growing = 0;
        let used = 0;
        for (const kid of kids) {
            if (kid.grow > 0) {
                growing += kid.grow;
            } else {
                used += size(kid);
            }
        }
        const room = column ? this.height : this.width;
        const across = (column ? this.width : this.height) - 2 * pad;
        const free = room - 2 * pad - this.gap * Math.max(kids.length - 1, 0) - used;
        let at = pad;
        for (const kid of kids) {
            if (kid.grow > 0) {
                // A growing child takes its share of the free room. A negative share is clamped to zero.
                const share = Math.max(0, Math.floor((free * kid.grow) / growing));
                if (column) kid.height = share;
                else kid.width = share;
            }
            if (column) {
                kid.y = at;
                kid.x = pad;
                kid.width = across;
            } else {
                kid.x = at;
                kid.y = pad;
                kid.height = across;
            }
            at += size(kid) + this.gap;
        }
    }
}
