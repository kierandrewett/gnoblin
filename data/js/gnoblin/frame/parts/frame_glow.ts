// `FrameGlow`: a soft halo round the window's frame, painted behind it. The paint list has no blur, so the halo is a stack
// of rounded rectangles that grow outwards and fade, drawn into the underlay's `bleed`.

import { LayerOutlet } from "gnoblin/frame/layer_outlet";
import { Canvas } from "gnoblin/ui/canvas";
import type { CanvasOptions } from "gnoblin/ui/canvas";
import type { PaintOp } from "gnoblin/ui/paint";

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
 * A glow as far out as the underlay's `bleed`: set `frame.underlay.bleed` to how far the halo reaches. It is as large as
 * the layer, so the rings grow from the frame's edge out to the layer's.
 */
export class FrameGlow extends Canvas {
    color: string;
    strength: number;
    radius: number;
    steps: number;

    constructor(options: FrameGlowOptions = {}) {
        super(options);
        this.color = options.color ?? "#89b4fa";
        this.strength = options.strength ?? 0.5;
        this.radius = options.radius ?? 0;
        this.steps = options.steps ?? 8;
    }

    override refresh(): void {
        const { parent } = this;
        if (parent !== null) {
            this.x = 0;
            this.y = 0;
            this.width = parent.width;
            this.height = parent.height;
        }
    }

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
