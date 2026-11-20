import type { TransformInit } from "gnoblin-core/math/spatial";
import { BindingCollection } from "gnoblin-core/bindings/binding";
import type { LayoutSplit } from "gnoblin-std/bindings/tiling";
import type { Point } from "gnoblin-core/bindings/types";
import type { Rect } from "gnoblin-core/types";
/** A layout places whatever the host calls a node; the library only needs its identity. */
export type LayoutNode = object;
/**
 * Where one node goes. A node the layout leaves out of its result is not touched, and a placement with no
 * `bounds` leaves its node where it is.
 */
export interface Placement<N extends LayoutNode = LayoutNode> {
    /** The node this placement describes. */
    node: N;
    /** The node's rectangle in logical pixels. */
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
     * The node's placement in 3D: position, rotation in radians (a list, `{ x, y, z }` or a quaternion) and scale
     * (one number or one per axis). On a flat workspace, position `x` and `y` offset the node from the centre of its
     * `bounds` or, without bounds, the area. Coordinates are layout pixels with y down; `z` is depth. Rotation and
     * scale apply to the node. The flat `z`, `rotation` and `scale` fields take precedence when both forms are given.
     * See `docs/typescript-shell.md`, "Spatial layouts".
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
export declare class Layout {
    /** Pointer and split-resize bindings associated with this layout. */
    readonly bindings: BindingCollection;
    /**
     * Where `context.children` go. The base class places nothing, which leaves every node where it is.
     * Placing is pure: the same context and fields give the same result.
     */
    compute(_context: LayoutContext): Placement[];
}
/**
 * The optional hooks of a `Layout`. They sit on the interface, merged with the class, so a subclass that does not
 * define one has no own property to hide the inherited method.
 */
export interface Layout {
    /** Called after a node joins the workspace. */
    onClientAdded?(client: LayoutNode, context: LayoutContext): void;
    /** Called after a node leaves the workspace. */
    onClientRemoved?(client: LayoutNode, context: LayoutContext): void;
    /** Called when the workspace rectangle changes. */
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
    /** Gap between neighbouring tiles, in logical pixels. */
    inner: number;
    /** Gap between tiles and the workspace edge, in logical pixels. */
    outer: number;
}
/** `area` shrunk by `outer` on every side, never below zero. */
export declare function insetRect(area: Rect, outer: number): Rect;
/** A gap option: a finite number of at least zero, or `undefined` for "use the workspace's". */
export declare function checkGap(value: unknown, what: string): number | undefined;
/**
 * The rectangle a tiling layout fills and the gap between its tiles: `area` inset by the workspace's outer
 * gap, and the layout's own gap when it has one, else the workspace's inner gap, else none.
 */
export declare function tilingGeometry(own: number | undefined, context: LayoutContext): {
    area: Rect;
    gap: number;
};
/** `value` limited to the range `low` to `high`. `low` must not be above `high`. */
export declare function clamp(value: number, low: number, high: number): number;
/**
 * A tiled placement at whole pixels. Each of the four edges is floored on its own, and the size is the distance between
 * two floored edges. Flooring the width itself would leave a tile short of where the area ends: a master of 607.2 and a
 * stack of 404.8 that starts at 619.2 are 607 and 404 wide when each is floored apart, and the stack ends at 1023 in an
 * area that ends at 1024.
 *
 * The goldens in the tests come from the Lua shell, which floored every tiled rectangle this way.
 */
export declare function tile<N extends LayoutNode>(node: N, x: number, y: number, width: number, height: number): Placement<N>;
