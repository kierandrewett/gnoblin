import { Canvas } from "gnoblin-std/ui/canvas";
import type { CanvasOptions } from "gnoblin-std/ui/canvas";
import type { PaintOp } from "gnoblin-std/ui/paint";
/** Options of `FrameGlow`. */
export interface FrameGlowOptions extends CanvasOptions {
    /** The halo's colour as `#rrggbb`. */
    color?: string;
    /** The strongest the halo is, next to the frame, from 0 to 1. */
    strength?: number;
    /** The rounded corners of the frame the halo follows. */
    radius?: number;
    /** The rings the halo is made of: more are smoother. */
    steps?: number;
}
/**
 * A glow as far out as the underlay's `bleed`: set `frame.underlay.bleed` to its reach. It fills the layer, so the
 * rings grow from the frame's edge out to the layer's.
 * the layer, so the rings grow from the frame's edge out to the layer's.
 */
export declare class FrameGlow extends Canvas {
    /** Hex colour used for the glow. */
    color: string;
    /** Halo strength applied across the rings, from 0 to 1. */
    strength: number;
    /** Corner radius the glow follows, in logical pixels. */
    radius: number;
    /** Number of rings used to draw the glow. */
    steps: number;
    constructor(options?: FrameGlowOptions);
    /** Fits the glow to its parent layer. */
    refresh(): void;
    /** Draws the glow rings within the layer's bleed. */
    draw(): PaintOp[];
}
