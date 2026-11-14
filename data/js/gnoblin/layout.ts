// The Layout base class and what every stock layout shares: the context a layout computes from, the
// placement it returns, and whole-pixel tiling (the Lua shell floors every tiled rectangle, and the
// goldens it produced are the reference).

import { BindingCollection } from "gnoblin/bindings";

export interface Rect {
    x: number;
    y: number;
    width: number;
    height: number;
}

/** A layout places whatever the host calls a node; the library only needs its identity. */
export type LayoutNode = object;

export interface Placement<N extends LayoutNode = LayoutNode> {
    node: N;
    bounds?: Rect;
    /** `false` hides the node (a monocle's other windows). Default: shown. */
    visible?: boolean;
    /** Marks the node as in a tiled layout, so its client drops the border and rounding it draws itself. */
    tiled?: boolean;
    /** Depth: along the viewing axis, negative in front of the camera. Default 0. */
    z?: number;
    /** Rotation about each axis in radians. Default no rotation. */
    rotation?: readonly [number, number, number];
    /** Scale along each axis. Default 1. */
    scale?: readonly [number, number, number];
}

export interface LayoutContext<N extends LayoutNode = LayoutNode> {
    readonly workspace?: unknown;
    /** The nodes to place, in order. */
    readonly children: readonly N[];
    /** The workspace's rectangle. A tiling layout insets it by `gap.outer` itself, so the host does not. */
    readonly area: Rect;
    /** The workspace's gaps. A layout whose own `gap` is set uses that instead of `gap.inner`. */
    readonly gap?: Gaps;
    readonly focused?: N | null;
}

export class Layout {
    readonly bindings = new BindingCollection();

    /**
     * Where `context.children` go. The base class places nothing, which leaves every node where it is.
     * Placing is pure: the same context and fields give the same result.
     */
    compute(_context: LayoutContext): Placement[] {
        return [];
    }
}

// Optional hooks live on the interface, not the class: a class field would define an own `undefined`
// property in the base constructor and hide the method a subclass declares.
export interface Layout {
    onClientAdded?(client: LayoutNode, context: LayoutContext): void;
    onClientRemoved?(client: LayoutNode, context: LayoutContext): void;
    onAreaChanged?(oldArea: Rect, newArea: Rect): void;
}

/** The workspace's gaps: `inner` between tiles, `outer` between the tiles and the area's edge. */
export interface Gaps {
    inner: number;
    outer: number;
}

/** `area` shrunk by `outer` on every side, never below zero. */
export function insetRect(area: Rect, outer: number): Rect {
    return {
        x: area.x + outer,
        y: area.y + outer,
        width: Math.max(0, area.width - 2 * outer),
        height: Math.max(0, area.height - 2 * outer),
    };
}

/** A gap option: a finite number of at least zero, or `undefined` for "use the workspace's". */
export function checkGap(value: unknown, what: string): number | undefined {
    if (value === undefined) {
        return undefined;
    }
    if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
        throw new RangeError(`${what} gap must be a finite number of at least 0, got ${String(value)}`);
    }
    return value;
}

/**
 * The rectangle a tiling layout fills and the gap between its tiles: `area` inset by the workspace's outer
 * gap, and the layout's own gap when it has one, else the workspace's inner gap, else none.
 */
export function tilingGeometry(own: number | undefined, context: LayoutContext): { area: Rect; gap: number } {
    return { area: insetRect(context.area, context.gap?.outer ?? 0), gap: own ?? context.gap?.inner ?? 0 };
}

export function clamp(value: number, low: number, high: number): number {
    return Math.max(low, Math.min(high, value));
}

/** A tiled placement at whole pixels: every edge is floored on its own, as the Lua shell does. */
export function tile<N extends LayoutNode>(node: N, x: number, y: number, width: number, height: number): Placement<N> {
    return {
        node,
        tiled: true,
        bounds: { x: Math.floor(x), y: Math.floor(y), width: Math.floor(width), height: Math.floor(height) },
    };
}
