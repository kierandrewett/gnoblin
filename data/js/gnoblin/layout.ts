// The Layout base class and what every stock layout shares: the context a layout computes from, the
// placement it returns, the workspace gaps, and whole-pixel tiling.
//
// This module owns the types and the small geometry helpers. It does not hold any layout: the stock ones are in
// `gnoblin/layouts/*`, and `gnoblin/workspace` is what calls `compute` and applies the placements.
// Read `gnoblin/workspace` first to see how a layout is driven; read `gnoblin/layouts/master-stack` for the
// smallest real layout.

import type { TransformInit } from "gnoblin/spatial";
import { BindingCollection } from "gnoblin/bindings";
import type { LayoutSplit, Point } from "gnoblin/bindings";

/** A rectangle in layout pixels: `x` and `y` are the top-left corner, `width` and `height` the size. */
export interface Rect {
    x: number;
    y: number;
    width: number;
    height: number;
}

/** A layout places whatever the host calls a node; the library only needs its identity. */
export type LayoutNode = object;

/**
 * Where one node goes. A node the layout leaves out of its result is not touched, and a placement with no
 * `bounds` leaves its node where it is.
 */
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
    /**
     * The node's placement in 3D, as the specification's spatial layouts return it: a position, a rotation (radians, a list,
     * `{ x, y, z }` or a quaternion) and a scale (one number or one per axis). On a flat workspace the position's `x` and
     * `y` move the node from the centre of its `bounds` (or, with no `bounds`, of the area, at the node's own size), in
     * layout pixels with y down; its `z` is the node's depth; the rotation and scale are the node's. The flat fields
     * `z`, `rotation` and `scale` win over the transform's when both are given. See `docs/typescript-shell.md`,
     * "Spatial layouts".
     */
    transform?: TransformInit;
}

/**
 * What the host hands to `Layout.compute` and to the hooks. It is a copy: a layout can keep it, and changing it
 * does not reach the workspace.
 */
export interface LayoutContext<N extends LayoutNode = LayoutNode> {
    /** The workspace that owns the layout. Untyped, as this module does not import the host. */
    readonly workspace?: unknown;
    /** The nodes to place, in order. */
    readonly children: readonly N[];
    /** The workspace's rectangle. A tiling layout insets it by `gap.outer` itself, so the host does not. */
    readonly area: Rect;
    /** The workspace's gaps. A layout whose own `gap` is set uses that instead of `gap.inner`. */
    readonly gap?: Gaps;
    /** The node that has keyboard focus in the workspace, or null. */
    readonly focused?: N | null;
}

/**
 * The base class of every layout. A subclass keeps its options as plain own fields and overrides `compute`.
 * The workspace compares the own fields between frames and computes again when one changed, so a layout needs
 * no setters. Private (`#x`) fields are not compared.
 */
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
/**
 * The optional hooks of a `Layout`. They sit on the interface, merged with the class, so a subclass that does not
 * define one has no own property to hide the inherited method.
 */
export interface Layout {
    onClientAdded?(client: LayoutNode, context: LayoutContext): void;
    onClientRemoved?(client: LayoutNode, context: LayoutContext): void;
    onAreaChanged?(oldArea: Rect, newArea: Rect): void;
    /**
     * The workspace traded the places of `a` and `b` in its order (`workspace.clients.swap`). A layout that places by
     * order has nothing to do, as the next compute reads the new order; one that keeps its own record of who sits
     * where (a tree) trades them there.
     */
    onClientsSwapped?(a: LayoutNode, b: LayoutNode): void;
    /**
     * The divider a drag that pressed `client` at `point` would move, or null when the layout has none there. A layout
     * with dividers answers from the geometry of its last compute; `SplitResizeBinding` drives what it returns.
     */
    findSplit?(client: LayoutNode, point: Point): LayoutSplit | null;
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

/** `value` limited to the range `low` to `high`. `low` must not be above `high`. */
export function clamp(value: number, low: number, high: number): number {
    return Math.max(low, Math.min(high, value));
}

/**
 * The float error of the arithmetic before a floor: the right edge of 619.2 + 404.8 is 1023.9999999999999 in IEEE
 * doubles, and flooring it as it is would lose a pixel that the layout meant to give.
 */
const EDGE_EPSILON = 1e-6;

/**
 * A tiled placement at whole pixels. Each of the four edges is floored on its own, and the size is the distance between
 * two floored edges. Flooring the width itself would leave a tile short of where the area ends: a master of 607.2 and a
 * stack of 404.8 that starts at 619.2 are 607 and 404 wide when each is floored apart, and the stack ends at 1023 in an
 * area that ends at 1024.
 *
 * The goldens in the tests come from the Lua shell, which floored every tiled rectangle this way.
 */
export function tile<N extends LayoutNode>(node: N, x: number, y: number, width: number, height: number): Placement<N> {
    const left = Math.floor(x + EDGE_EPSILON);
    const top = Math.floor(y + EDGE_EPSILON);
    return {
        node,
        tiled: true,
        bounds: {
            x: left,
            y: top,
            width: Math.floor(x + width + EDGE_EPSILON) - left,
            height: Math.floor(y + height + EDGE_EPSILON) - top,
        },
    };
}
