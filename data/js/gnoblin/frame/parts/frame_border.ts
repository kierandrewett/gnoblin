// `FrameBorder` draws a ring over the window's frame. In the overlay it acts as a focus ring that follows focus.

import type { FrameContext } from "gnoblin/frame/client";
import { LayerOutlet } from "gnoblin/frame/layer_outlet";
import { Canvas } from "gnoblin/ui/canvas";
import type { CanvasOptions } from "gnoblin/ui/canvas";
import type { PaintOp } from "gnoblin/ui/paint";

/** A colour, or a function of the window that makes one: `(client) => client.focused ? "#89b4fa" : "#45475a"`. */
export type FrameColour = string | ((client: FrameContext["client"]) => string);

/** Options of `FrameBorder`. */
export interface FrameBorderOptions extends CanvasOptions {
    /** The line's width in pixels. */
    thickness?: number;
    /** Ring colour, or a function of the framed window. */
    color?: FrameColour;
    /** The corner radius of the ring: the window's `corner_radius` when it has one. */
    radius?: number;
}

/**
 * A ring along the frame edge, `thickness` pixels wide and inside it. It fills its layer or outlet less the layer's
 * `bleed`. A `color` function is called each frame, so a focus ring follows the window's colour.
 */
export class FrameBorder extends Canvas {
    /** Window context used to resolve a dynamic colour. */
    readonly context: FrameContext;
    /** The line's width in pixels. */
    thickness: number;
    /** Ring colour, or a function that returns the current colour. */
    color: FrameColour;
    /** Corner radius of the ring in logical pixels. */
    radius: number;

    constructor(context: FrameContext, options: FrameBorderOptions = {}) {
        super(options);
        this.context = context;
        this.thickness = options.thickness ?? 1;
        this.color = options.color ?? "#89b4fa";
        this.radius = options.radius ?? 0;
    }

    /** Fits the ring to its parent, excluding any layer bleed. */
    override refresh(): void {
        const { parent } = this;
        if (parent !== null) {
            // A layer reaches `bleed` pixels past the frame; the ring is on the frame's own edge.
            const bleed = parent instanceof LayerOutlet ? parent.bleed : 0;
            this.x = bleed;
            this.y = bleed;
            this.width = parent.width - 2 * bleed;
            this.height = parent.height - 2 * bleed;
        }
    }

    /** Draws the ring using the current colour and bounds. */
    override draw(): PaintOp[] {
        const line = this.thickness;
        const colour = typeof this.color === "function" ? this.color(this.context.client) : this.color;
        return [
            {
                op: "rect",
                x: line / 2,
                y: line / 2,
                width: this.width - line,
                height: this.height - line,
                radius: this.radius,
                fill: "#00000000",
                stroke: colour,
                strokeWidth: line,
            },
        ];
    }
}
