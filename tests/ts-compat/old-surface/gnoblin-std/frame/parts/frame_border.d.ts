import type { FrameContext } from "gnoblin-std/frame/client";
import { Canvas } from "gnoblin-std/ui/canvas";
import type { CanvasOptions } from "gnoblin-std/ui/canvas";
import type { PaintOp } from "gnoblin-std/ui/paint";
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
export declare class FrameBorder extends Canvas {
    /** Window context used to resolve a dynamic colour. */
    readonly context: FrameContext;
    /** The line's width in pixels. */
    thickness: number;
    /** Ring colour, or a function that returns the current colour. */
    color: FrameColour;
    /** Corner radius of the ring in logical pixels. */
    radius: number;
    constructor(context: FrameContext, options?: FrameBorderOptions);
    /** Fits the ring to its parent, excluding any layer bleed. */
    refresh(): void;
    /** Draws the ring using the current colour and bounds. */
    draw(): PaintOp[];
}
