// `FrameGlow` paints a soft halo behind the window's frame. The paint list has no blur, so the halo uses growing,
// fading rounded rectangles drawn into the underlay's `bleed`.
// of rounded rectangles that grow outwards and fade, drawn into the underlay's `bleed`.

import { LayerOutlet } from "gnoblin-std/frame/layer_outlet";
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

/** Two hex digits of `alpha`, from 0 to 1. */
function alphaByte(alpha: number): string {
    return Math.round(Math.min(1, Math.max(0, alpha)) * 255)
        .toString(16)
        .padStart(2, "0");
}

/**
 * A glow as far out as the underlay's `bleed`: set `frame.underlay.bleed` to its reach. It fills the layer, so the
 * rings grow from the frame's edge out to the layer's.
 * the layer, so the rings grow from the frame's edge out to the layer's.
 */
export class FrameGlow extends Canvas {
    /** Hex colour used for the glow. */
    color: string;
    /** Halo strength applied across the rings, from 0 to 1. */
    strength: number;
    /** Corner radius the glow follows, in logical pixels. */
    radius: number;
    /** Number of rings used to draw the glow. */
    steps: number;

    constructor(options: FrameGlowOptions = {}) {
        super(options);
        this.color = options.color ?? "#89b4fa";
        this.strength = options.strength ?? 0.5;
        this.radius = options.radius ?? 0;
        this.steps = options.steps ?? 8;
    }

    /** Fits the glow to its parent layer. */
    override refresh(): void {
        const { parent } = this;
        if (parent !== null) {
            this.x = 0;
            this.y = 0;
            this.width = parent.width;
            this.height = parent.height;
        }
    }

    /** Draws the glow rings within the layer's bleed. */
    override draw(): PaintOp[] {
        const { parent } = this;
        const reach = parent instanceof LayerOutlet ? parent.bleed : 0;
        const ops: PaintOp[] = [];
        // Outermost first, each ring over the last, so the halo is strongest at the frame and fades outwards.
        for (let ring = this.steps; ring >= 1; ring--) {
            const grow = (reach * ring) / this.steps;
            const alpha = this.strength / this.steps;
            ops.push({
                op: "rect",
                x: reach - grow,
                y: reach - grow,
                width: this.width - 2 * (reach - grow),
                height: this.height - 2 * (reach - grow),
                radius: this.radius + grow,
                fill: `${this.color}${alphaByte(alpha)}`,
            });
        }
        return ops;
    }
}
