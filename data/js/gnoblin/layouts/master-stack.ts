// MasterStackLayout: a master column of `masterCount` nodes and a stack of the rest. Geometry is
// data/lua/gnoblin/shell/tiling.lua `Tiling`, rectangle for rectangle: the master takes `ratio` of the
// free width (height for top and bottom), `gap` separates tiles, and every edge is floored.

import { resolveTriggers } from "gnoblin/bindings";
import { checkGap, clamp, Layout, tile, tilingGeometry } from "gnoblin/layout";
import type { LayoutContext, Placement } from "gnoblin/layout";

export type Orientation = "left" | "right" | "top" | "bottom";

const ORIENTATIONS: readonly Orientation[] = ["left", "right", "top", "bottom"];

export interface MasterStackOptions {
    /** Share of the free width the master column takes, clamped to 0.1..0.9 when computing. Default 0.5. */
    ratio?: number;
    /** How many nodes share the master column. Default 1. */
    masterCount?: number;
    /** Overrides the workspace's inner gap. */
    gap?: number;
    /** Which edge the master sits on. Default `left`. */
    orientation?: Orientation;
    /** `null` turns a binding off. The bindings themselves are not wired yet; the resolved spellings are kept. */
    triggers?: { swap?: string | null; resize?: string | null };
}

const DEFAULT_TRIGGERS = { swap: "Super+MouseLeft+Drag", resize: "Super+MouseRight+Drag" } as const;

type Frame = [u: number, v: number, p: number, q: number];

export class MasterStackLayout extends Layout {
    ratio: number;
    masterCount: number;
    gap: number | undefined;
    orientation: Orientation;
    readonly triggers: Readonly<Record<"swap" | "resize", string | null>>;

    constructor(options: MasterStackOptions = {}) {
        super();
        this.ratio = options.ratio ?? 0.5;
        this.masterCount = options.masterCount ?? 1;
        this.gap = checkGap(options.gap, "MasterStackLayout");
        this.orientation = options.orientation ?? "left";
        this.triggers = Object.freeze(resolveTriggers(DEFAULT_TRIGGERS, options.triggers));
    }

    compute(context: LayoutContext): Placement[] {
        const nodes = context.children;
        if (nodes.length === 0) {
            return [];
        }
        if (!ORIENTATIONS.includes(this.orientation)) {
            throw new TypeError("MasterStackLayout orientation must be left, right, top or bottom");
        }
        if (!Number.isFinite(this.ratio) || !Number.isFinite(this.masterCount)) {
            throw new TypeError("MasterStackLayout ratio and masterCount must be finite numbers");
        }
        const master = Math.max(0, Math.floor(this.masterCount));
        const ratio = clamp(this.ratio, 0.1, 0.9);
        const { area, gap } = tilingGeometry(this.gap, context);
        const { x, y, width: w0, height: h0 } = area;
        // Top and bottom are the master-left layout of the transposed area, turned back by `toArea`.
        const sideways = this.orientation === "top" || this.orientation === "bottom";
        const frames = masterStackFrames(sideways ? h0 : w0, sideways ? w0 : h0, nodes.length, master, ratio, gap);
        return nodes.map((node, i) => {
            const [rx, ry, rw, rh] = toArea(this.orientation, w0, h0, frames[i]);
            return tile(node, x + rx, y + ry, rw, rh);
        });
    }
}

/** Maps a rectangle of the master-left frame to the area for `orientation`; `w0` by `h0` is the area. */
function toArea(orientation: Orientation, w0: number, h0: number, [u, v, p, q]: Frame): Frame {
    switch (orientation) {
        case "left":
            return [u, v, p, q];
        case "right":
            return [w0 - u - p, v, p, q];
        case "top":
            return [v, u, q, p];
        default:
            return [v, h0 - u - p, q, p];
    }
}

/**
 * Rectangles in a `width` by `height` frame whose master column sits at u = 0: the first `master` nodes
 * share the master region top to bottom, the rest share the stack to its right. With no stack the master
 * fills the frame.
 */
function masterStackFrames(
    width: number,
    height: number,
    n: number,
    master: number,
    ratio: number,
    gap: number,
): Frame[] {
    const frames: Frame[] = [];
    const m = Math.min(master, n);
    const s = n - m;
    let masterWidth = 0;
    if (m > 0) {
        masterWidth = s === 0 ? width : Math.max(0, (width - gap) * ratio);
        const h = Math.max(0, (height - (m - 1) * gap) / m);
        for (let i = 0; i < m; i++) {
            frames.push([0, i * (h + gap), masterWidth, h]);
        }
    }
    if (s > 0) {
        const start = m > 0 ? masterWidth + gap : 0;
        const w = Math.max(0, width - start);
        const h = Math.max(0, (height - (s - 1) * gap) / s);
        for (let i = 0; i < s; i++) {
            frames.push([start, i * (h + gap), w, h]);
        }
    }
    return frames;
}
