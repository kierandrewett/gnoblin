// MasterStackLayout places `masterCount` nodes in a master column and the rest in a stack. The master takes `ratio`
// of the available width (or height for top and bottom); `gap` separates tiles.
// Default `swap` and `resize` bindings trade tile positions and drag the divider between the master and stack.
// `triggers` replaces a binding trigger, and `null` leaves that binding out.

import { resolveTriggers } from "gnoblin-core/bindings/binding";
import { addTilingBindings } from "gnoblin-std/bindings/tiling";
import type { LayoutSplit } from "gnoblin-std/bindings/tiling";
import type { Point } from "gnoblin-core/bindings/types";
import { checkGap, clamp, Layout, tile, tilingGeometry } from "gnoblin-std/layout";
import type { LayoutContext, LayoutNode, Placement } from "gnoblin-std/layout";

/** The edge the master column sits on. */
export type Orientation = "left" | "right" | "top" | "bottom";

const ORIENTATIONS: readonly Orientation[] = ["left", "right", "top", "bottom"];

/** Options of `MasterStackLayout`. */
export interface MasterStackOptions {
    /** Share of the free width the master column takes, clamped to 0.1..0.9 when computing. Default 0.5. */
    ratio?: number;
    /** How many nodes share the master column. Default 1. */
    masterCount?: number;
    /** Overrides the workspace's inner gap. */
    gap?: number;
    /** Which edge the master sits on. Default `left`. */
    orientation?: Orientation;
    /** `undefined` keeps the default trigger, a string replaces it and `null` leaves the binding out. */
    triggers?: { swap?: string | null; resize?: string | null };
}

const DEFAULT_TRIGGERS = { swap: "Super+MouseLeft+Drag", resize: "Super+MouseRight+Drag" } as const;

const MIN_RATIO = 0.1;
const MAX_RATIO = 0.9;

type Frame = [u: number, v: number, p: number, q: number];

/** What the last compute laid out: the one thing `findSplit` needs to know that the fields do not say. */
interface Computed {
    readonly nodes: readonly LayoutNode[];
    readonly master: number;
    /** The extent the ratio divides along the master's axis, less the gap. */
    readonly span: number;
}

/** A tiling layout with one master column on one edge. The other nodes stack in the remaining column. */
export class MasterStackLayout extends Layout {
    /** Share of the available master axis, clamped to 0.1..0.9 when computing. Default 0.5. */
    ratio: number;
    /** Number of nodes placed in the master region. Default 1. */
    masterCount: number;
    /** Gap between tiles, or `undefined` to use the workspace gap. */
    gap: number | undefined;
    /** Edge used for the master region. Default `left`. */
    orientation: Orientation;
    /** Resolved `swap` and `resize` triggers; `null` means that binding is disabled. */
    readonly triggers: Readonly<Record<"swap" | "resize", string | null>>;
    #computed: Computed | null = null;

    constructor(options: MasterStackOptions = {}) {
        super();
        this.ratio = options.ratio ?? 0.5;
        this.masterCount = options.masterCount ?? 1;
        this.gap = checkGap(options.gap, "MasterStackLayout");
        this.orientation = options.orientation ?? "left";
        this.triggers = Object.freeze(resolveTriggers(DEFAULT_TRIGGERS, options.triggers));
        addTilingBindings(this.bindings, this.triggers);
    }

    /** Returns one tiled placement per child in the master and stack regions. */
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
        this.#computed = { nodes, master, span: (sideways ? h0 : w0) - gap };
        const frames = masterStackFrames(sideways ? h0 : w0, sideways ? w0 : h0, nodes.length, master, ratio, gap);
        return nodes.map((node, i) => {
            const [rx, ry, rw, rh] = toArea(this.orientation, w0, h0, frames[i]);
            return tile(node, x + rx, y + ry, rw, rh);
        });
    }

    /**
     * The divider between the master and the stack, for any tile of the last compute: dragging the pointer along the
     * master's axis moves it by as far, whichever side the master is on. With no master or no stack there is none.
     */
    findSplit(client: LayoutNode, _point: Point): LayoutSplit | null {
        const last = this.#computed;
        if (last === null || !last.nodes.includes(client) || last.span <= 0) {
            return null;
        }
        if (last.master === 0 || last.master >= last.nodes.length) {
            return null;
        }
        const sideways = this.orientation === "top" || this.orientation === "bottom";
        // The master grows as the pointer moves away from the edge it sits on.
        const direction = this.orientation === "left" || this.orientation === "top" ? 1 : -1;
        return new MasterSplit(this, sideways ? "y" : "x", last.span, direction);
    }
}

/**
 * The master's ratio as a divider a drag moves. The layout's own `ratio` field is what changes, so the workspace
 * computes again by itself.
 */
class MasterSplit implements LayoutSplit {
    readonly axis: "x" | "y";
    readonly span: number;
    readonly direction: 1 | -1;
    readonly min = MIN_RATIO;
    readonly max = MAX_RATIO;
    readonly #layout: { ratio: number };
    readonly #original: number;

    constructor(layout: { ratio: number }, axis: "x" | "y", span: number, direction: 1 | -1) {
        this.#layout = layout;
        this.#original = layout.ratio;
        this.axis = axis;
        this.span = span;
        this.direction = direction;
    }

    get ratio(): number {
        return clamp(this.#layout.ratio, MIN_RATIO, MAX_RATIO);
    }

    set ratio(value: number) {
        this.#layout.ratio = value;
    }

    reset(): void {
        this.#layout.ratio = this.#original;
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
