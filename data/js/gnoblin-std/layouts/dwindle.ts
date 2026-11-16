// DwindleLayout keeps a binary split tree for one workspace and places windows as Hyprland's dwindle layout does. A new
// window splits the cell of the focused window (the last window's cell when none has focus) in two.
//
// A split's direction is not fixed. Every compute reads the cell the split divides: wider than tall, the two halves sit
// side by side; taller than wide, one above the other. `preserveSplit` (or `smartSplit`, which implies it) keeps the
// direction a split had when it was made, as does the `split` option when it is `horizontal` or `vertical` and a
// split made by `preselect`. `splitWidthMultiplier` scales the height in that comparison.
//
// The tree persists between computes: removing a node collapses its split, and adding one inserts a leaf. A deep tree
// is walked with an explicit stack so it does not use the engine's call stack.
//
// Ratios are fractions of the first child, 0.1 to 0.9. Hyprland's `splitRatio` and `default_split_ratio` are twice that:
// 1.0 there is 0.5 here, and Hyprland's 0.1 to 1.9 range is 0.05 to 0.95 there and 0.1 to 0.9 here.
//
// The actions (`toggleSplit`, `swapSplit`, `rotateSplit`, `preselect`, `moveToRoot`, `splitRatio`, `pseudotile`) are
// methods a key binding calls with the workspace; they change the tree and the workspace computes again. A split a
// drag or `splitRatio` set keeps its own ratio until reset.
//
// Default `swap` and `resize` bindings exchange leaves and drag split dividers. `triggers` replaces a binding trigger,
// and `null` leaves that binding out.

import { resolveTriggers } from "gnoblin-core/bindings/binding";
import { addTilingBindings } from "gnoblin-std/bindings/tiling";
import type { LayoutSplit } from "gnoblin-std/bindings/tiling";
import type { Point } from "gnoblin-core/bindings/types";
import { growRect, NO_INSET } from "gnoblin-core/inset";
import type { FrameInset } from "gnoblin-core/inset";
import { checkGap, clamp, Layout, tile, tilingGeometry } from "gnoblin-std/layout";
import type { LayoutContext, LayoutNode, Placement } from "gnoblin-std/layout";
import type { Rect } from "gnoblin-core/types";
import type { FocusableNode, NodeSource } from "gnoblin-std/layouts/strip";

/** How a split picks its direction. `aspect` follows the shape of the cell, as Hyprland's dwindle does. */
export type DwindleSplit = "aspect" | "horizontal" | "vertical";

/** Where `preselect` puts the next window, relative to the window it splits. `none` clears the choice. */
export type DwindleDirection = "left" | "right" | "up" | "down" | "none";

/** A window size in pixels, frame included: what a pseudotiled window keeps. */
export interface DwindleSize {
    /** Width in logical pixels. */
    width: number;
    /** Height in logical pixels. */
    height: number;
}

/** Options of `DwindleLayout`. The names are Hyprland's `dwindle` options in camelCase, with their defaults. */
export interface DwindleOptions {
    /**
     * Split direction for each new node. `aspect` follows the shape of the cell, every compute (Hyprland's default);
     * `horizontal` and `vertical` fix it for the splits this layout makes. Default `aspect`.
     */
    split?: DwindleSplit;
    /**
     * The first child's share of every split that was not dragged or set, held to 0.1 to 0.9. Hyprland's
     * `default_split_ratio` is twice this: its default 1.0 is 0.5 here. Default 0.5.
     */
    ratio?: number;
    /** Overrides the workspace's inner gap. */
    gap?: number;
    /** `undefined` keeps the default trigger, a string replaces it and `null` leaves the binding out. */
    triggers?: { swap?: string | null; resize?: string | null };
    /**
     * Where a new window goes in the cell it splits. 0: on the side of the cell the `pointer` is on, and second (right
     * or below) when there is no pointer; 1: first (left or above); 2: second (right or below). Default 0.
     */
    forceSplit?: 0 | 1 | 2;
    /** Keeps each split's direction as it was made instead of following the shape of its cell. Default false. */
    preserveSplit?: boolean;
    /**
     * Chooses the direction and side of a new window by the `pointer`'s place in the cell it splits: the cell is divided
     * into four triangles by its diagonals. Implies `preserveSplit`. With no pointer it does nothing. Default false.
     */
    smartSplit?: boolean;
    /**
     * Which divider a `SplitResizeBinding` drag moves. True: the divider nearest the press. False: both dividers that
     * bound the window by its place in the tree, the one above it and the nearest one of the other direction above
     * that, each by the pointer's travel along its own axis. Default true.
     */
    smartResizing?: boolean;
    /** Keeps a `preselect` choice for every new window until it is cleared or changed, instead of once. Default false. */
    permanentDirectionOverride?: boolean;
    /**
     * A split goes side by side when the cell's width is at least its height times this, held to 0.1 to 3.0. Above 1
     * more splits go top to bottom, which suits a wide monitor. Default 1.
     */
    splitWidthMultiplier?: number;
    /**
     * True: a new window splits the focused window. False: it splits the window under the `pointer`, or the first
     * window when there is no pointer. Default true.
     */
    useActiveForSplits?: boolean;
    /**
     * Which window of a new split gets `ratio`. 0: the first one (top or left). 1: the window the new one was opened on,
     * wherever it sits. Default 0.
     */
    splitBias?: 0 | 1;
    /**
     * Where the pointer is, in the layout's coordinates, or null. `forceSplit` 0, `smartSplit` and
     * `useActiveForSplits` read it when a window arrives. A layout has no pointer of its own: give it one with
     * `() => gnoblin.input.primary.pointer`. Default none.
     */
    pointer?: (() => Point | null) | null;
}

const DEFAULT_TRIGGERS = { swap: "Super+MouseLeft+Drag", resize: "Super+MouseRight+Drag" } as const;

const SPLITS: readonly string[] = ["aspect", "horizontal", "vertical"];
const DIRECTIONS: readonly string[] = ["left", "right", "up", "down", "none"];
const MIN_RATIO = 0.1;
const MAX_RATIO = 0.9;
const MIN_MULTIPLIER = 0.1;
const MAX_MULTIPLIER = 3;
/** The options that are true or false. */
const FLAGS = [
    "preserveSplit",
    "smartSplit",
    "useActiveForSplits",
    "smartResizing",
    "permanentDirectionOverride",
] as const;

interface Leaf {
    /** The node in this place of the tree. A swap puts the other one here. */
    leaf: LayoutNode;
    parent: Split | null;
}

/** `"h"` puts `a` left of `b`; `"v"` puts `a` above `b`. */
interface Split {
    /** The direction. A split that follows its cell has it set again by every compute. */
    dir: "h" | "v";
    /** The direction does not follow the shape of the cell: a `split` option, a preselection. */
    fixed: boolean;
    /** The layout's ratio is turned over for this split: `splitBias` gave it to the window the new one was opened on. */
    biased: boolean;
    /**
     * The first child's share a drag or `splitRatio` set for this split alone, or null while the split follows the
     * layout's `ratio`.
     */
    ratio: number | null;
    a: Tree;
    b: Tree;
    parent: Split | null;
}

type Tree = Leaf | Split;

/** What the cells of a compute depend on besides the tree. */
interface Shape {
    gap: number;
    /** The layout's ratio, held to its range. */
    ratio: number;
    /** No split follows its cell: `preserveSplit` or `smartSplit`. */
    preserve: boolean;
    /** `splitWidthMultiplier`, held to its range. */
    multiplier: number;
}

function isLeaf(tree: Tree): tree is Leaf {
    return "leaf" in tree;
}

/** The share of the first child of `split` when it has no ratio of its own. */
function inheritedShare(split: Split, ratio: number): number {
    return split.biased ? 1 - ratio : ratio;
}

/**
 * The rectangle of every leaf under `root` inside `area`; with `splits`, the rectangle each split divides as well. A
 * split that follows its cell takes its direction from the cell here: top and bottom when the height times the
 * multiplier is above the width, side by side otherwise (a square cell is side by side).
 */
function cells(root: Tree, area: Rect, shape: Shape, splits?: Map<Split, Rect>): Map<LayoutNode, Rect> {
    const { gap } = shape;
    const out = new Map<LayoutNode, Rect>();
    const pending: { node: Tree; rect: Rect }[] = [{ node: root, rect: area }];
    for (let next = pending.pop(); next !== undefined; next = pending.pop()) {
        const { node, rect } = next;
        if (isLeaf(node)) {
            out.set(node.leaf, rect);
            continue;
        }
        if (!shape.preserve && !node.fixed) {
            node.dir = rect.height * shape.multiplier > rect.width ? "v" : "h";
        }
        splits?.set(node, rect);
        const share = clamp(node.ratio ?? inheritedShare(node, shape.ratio), MIN_RATIO, MAX_RATIO);
        if (node.dir === "h") {
            const first = Math.max(0, (rect.width - gap) * share);
            pending.push({
                node: node.b,
                rect: {
                    x: rect.x + first + gap,
                    y: rect.y,
                    width: Math.max(0, rect.width - gap - first),
                    height: rect.height,
                },
            });
            pending.push({ node: node.a, rect: { x: rect.x, y: rect.y, width: first, height: rect.height } });
        } else {
            const first = Math.max(0, (rect.height - gap) * share);
            pending.push({
                node: node.b,
                rect: {
                    x: rect.x,
                    y: rect.y + first + gap,
                    width: rect.width,
                    height: Math.max(0, rect.height - gap - first),
                },
            });
            pending.push({ node: node.a, rect: { x: rect.x, y: rect.y, width: rect.width, height: first } });
        }
    }
    return out;
}

/** The squared distance from `point` to `rect`; 0 inside it. */
function distance(rect: Rect, point: Point): number {
    const dx = Math.max(rect.x - point.x, 0, point.x - (rect.x + rect.width));
    const dy = Math.max(rect.y - point.y, 0, point.y - (rect.y + rect.height));
    return dx * dx + dy * dy;
}

/** The node whose cell holds `point`, else the one whose cell is nearest it; the first of equals. */
function nearest(rects: ReadonlyMap<LayoutNode, Rect>, point: Point): LayoutNode {
    let best: LayoutNode | null = null;
    let least = Infinity;
    for (const [node, rect] of rects) {
        const d = distance(rect, point);
        if (best === null || d < least) {
            best = node;
            least = d;
        }
    }
    return best as LayoutNode;
}

/** The own size of `node` with its frame, when it has a width and a height; else null. */
function ownSize(node: LayoutNode): DwindleSize | null {
    const { width, height, frameInset } = node as { width?: unknown; height?: unknown; frameInset?: FrameInset };
    if (typeof width !== "number" || typeof height !== "number" || !(width > 0) || !(height > 0)) {
        return null;
    }
    const outer = growRect({ x: 0, y: 0, width, height }, frameInset ?? NO_INSET);
    return { width: outer.width, height: outer.height };
}

function checkFlag(layout: DwindleLayout, name: (typeof FLAGS)[number]): void {
    const value = (layout as unknown as Record<string, unknown>)[name];
    if (typeof value !== "boolean") {
        throw new TypeError(`DwindleLayout ${name} must be true or false`);
    }
}

/** A dwindle tiling layout. Each new node splits the cell it arrives in, as the header describes. */
export class DwindleLayout extends Layout {
    /** Direction used when a new node arrives. */
    split: DwindleSplit;
    /** First child's share of each split, clamped to 0.1..0.9 when computing. Default 0.5. */
    ratio: number;
    /** Overrides the workspace's inner gap; `undefined` uses the workspace gap. */
    gap: number | undefined;
    /** Resolved `swap` and `resize` triggers; `null` means that binding is disabled. */
    readonly triggers: Readonly<Record<"swap" | "resize", string | null>>;
    /** Where a new window goes in its cell: 0 by the pointer, 1 first, 2 second. Default 0. */
    forceSplit: 0 | 1 | 2;
    /** Whether a split keeps the direction it was made with. Default false. */
    preserveSplit: boolean;
    /** Whether the pointer's triangle in the cell picks the split. Default false. */
    smartSplit: boolean;
    /** Whether a resize drag moves the divider nearest the press (true) or both dividers of the tile (false). */
    smartResizing: boolean;
    /** Whether `preselect` stays until cleared. Default false. */
    permanentDirectionOverride: boolean;
    /** Scales the height in the side-by-side test, 0.1 to 3.0. Default 1. */
    splitWidthMultiplier: number;
    /** Whether a new window splits the focused window or the one under the pointer. Default true. */
    useActiveForSplits: boolean;
    /** Which window of a new split gets `ratio`: 0 the first, 1 the one the new one was opened on. Default 0. */
    splitBias: 0 | 1;
    /** Where the pointer is, or null. */
    pointer: (() => Point | null) | null;
    /**
     * How many times a drag or an action changed the tree or the ratio of one split. The tree is not a field the
     * workspace can compare, so this counter is: a change to it is what makes the workspace compute again.
     */
    revision = 0;
    #geometry: { area: Rect; shape: Shape } | null = null;
    #root: Tree | null = null;
    #leaves = new Map<LayoutNode, Leaf>();
    /** Pseudotiled nodes and the size each keeps; null until the next compute gives it its cell's size. */
    #pseudo = new Map<LayoutNode, DwindleSize | null>();
    #preselect: DwindleDirection = "none";
    /** The focus the previous compute saw: the node a new one splits, because the new one takes focus on arrival. */
    #focus: LayoutNode | null = null;

    constructor(options: DwindleOptions = {}) {
        super();
        this.split = options.split ?? "aspect";
        this.ratio = options.ratio ?? 0.5;
        this.gap = checkGap(options.gap, "DwindleLayout");
        this.forceSplit = options.forceSplit ?? 0;
        this.preserveSplit = options.preserveSplit ?? false;
        this.smartSplit = options.smartSplit ?? false;
        this.smartResizing = options.smartResizing ?? true;
        this.permanentDirectionOverride = options.permanentDirectionOverride ?? false;
        this.splitWidthMultiplier = options.splitWidthMultiplier ?? 1;
        this.useActiveForSplits = options.useActiveForSplits ?? true;
        this.splitBias = options.splitBias ?? 0;
        this.pointer = options.pointer ?? null;
        this.triggers = Object.freeze(resolveTriggers(DEFAULT_TRIGGERS, options.triggers));
        addTilingBindings(this.bindings, this.triggers);
    }

    /** The direction `preselect` set for the next window, or `none`. */
    get preselected(): DwindleDirection {
        return this.#preselect;
    }

    /** Computes placements for the current children, updating the stored split tree as needed. */
    compute(context: LayoutContext): Placement[] {
        const nodes = context.children;
        const { area, shape } = this.#check(context);
        const live = new Set<LayoutNode>(nodes);
        for (const node of [...this.#leaves.keys()]) {
            if (!live.has(node)) {
                this.#remove(node);
            }
        }
        for (const node of nodes) {
            if (!this.#leaves.has(node)) {
                this.#open(node, area, shape);
            }
        }
        const focused = context.focused;
        if (focused !== null && focused !== undefined && live.has(focused)) {
            this.#focus = focused;
        } else if (this.#focus !== null && !live.has(this.#focus)) {
            this.#focus = null;
        }
        if (this.#root === null) {
            return [];
        }
        const rects = cells(this.#root, area, shape);
        return nodes.map((node) => {
            const cell = rects.get(node) as Rect;
            if (!this.#pseudo.has(node)) {
                return tile(node, cell.x, cell.y, cell.width, cell.height);
            }
            const own = this.#pseudo.get(node) ?? { width: cell.width, height: cell.height };
            this.#pseudo.set(node, own);
            const width = Math.min(own.width, cell.width);
            const height = Math.min(own.height, cell.height);
            return tile(node, cell.x + (cell.width - width) / 2, cell.y + (cell.height - height) / 2, width, height);
        });
    }

    /** Swaps two nodes in the stored tree without changing their positions. */
    onClientsSwapped(a: LayoutNode, b: LayoutNode): void {
        const first = this.#leaves.get(a);
        const second = this.#leaves.get(b);
        if (first === undefined || second === undefined) {
            return; // one of them has not been placed yet: the next compute puts it where the order says
        }
        first.leaf = b;
        second.leaf = a;
        this.#leaves.set(a, second);
        this.#leaves.set(b, first);
    }

    /**
     * The divider a resize drag that pressed `client` at `point` moves. With `smartResizing` it is the split of the last
     * compute whose divider is nearest `point`, among the splits above `client`'s tile: the ones that bound the tile or a
     * bigger region around it; a tie goes to the split nearer the tile. Without it, the split directly above the tile,
     * and the nearest split above that which goes the other way as its `cross`.
     */
    findSplit(client: LayoutNode, point: Point): LayoutSplit | null {
        const geometry = this.#geometry;
        const leaf = this.#leaves.get(client);
        if (geometry === null || this.#root === null || leaf === undefined) {
            return null;
        }
        const { shape } = geometry;
        const regions = new Map<Split, Rect>();
        cells(this.#root, geometry.area, shape, regions);
        const bump = (): void => {
            this.revision += 1;
        };
        const span = (split: Split): number => {
            const region = regions.get(split) as Rect;
            return (split.dir === "h" ? region.width : region.height) - shape.gap;
        };
        const divider = (split: Split, cross: DwindleDivider | null): DwindleDivider =>
            new DwindleDivider(split, span(split), inheritedShare(split, shape.ratio), bump, cross);
        if (!this.smartResizing) {
            const above = leaf.parent;
            if (above === null || !(span(above) > 0)) {
                return null;
            }
            let other = above.parent;
            while (other !== null && other.dir === above.dir) {
                other = other.parent;
            }
            return divider(above, other !== null && span(other) > 0 ? divider(other, null) : null);
        }
        let best: { split: Split; distance: number } | null = null;
        for (let split = leaf.parent; split !== null; split = split.parent) {
            const region = regions.get(split);
            if (region === undefined) {
                continue;
            }
            const along = split.dir === "h" ? region.width : region.height;
            const share = clamp(split.ratio ?? inheritedShare(split, shape.ratio), MIN_RATIO, MAX_RATIO);
            const first = Math.max(0, (along - shape.gap) * share);
            const edge = (split.dir === "h" ? region.x : region.y) + first + shape.gap / 2;
            const away = Math.abs((split.dir === "h" ? point.x : point.y) - edge);
            if (along - shape.gap > 0 && (best === null || away < best.distance)) {
                best = { split, distance: away };
            }
        }
        return best === null ? null : divider(best.split, null);
    }

    /**
     * Sets where the next window opens: `left` or `up` puts it first (left or above) with a split that goes that way,
     * `right` or `down` second. The split keeps its direction whatever its cell's shape. The choice is used once, unless
     * `permanentDirectionOverride` is set; `none` clears it. Nothing is changed in the tree, and the first window of a
     * workspace does not use it.
     */
    preselect(direction: DwindleDirection): void {
        if (!DIRECTIONS.includes(direction)) {
            throw new TypeError(
                `DwindleLayout preselect direction must be left, right, up, down or none, got ${String(direction)}`,
            );
        }
        this.#preselect = direction;
    }

    /**
     * Turns the split above the focused window from side by side to top and bottom or back. A split that follows the
     * shape of its cell takes its direction from the cell again at once, so this needs `preserveSplit` (or a split that
     * is fixed), as in Hyprland. Returns false when no window with a split above it has focus.
     */
    toggleSplit(workspace: NodeSource): boolean {
        const split = this.#focusedLeaf(workspace)?.parent;
        if (split === null || split === undefined) {
            return false;
        }
        split.dir = split.dir === "h" ? "v" : "h";
        this.revision += 1;
        return true;
    }

    /** Swaps the two halves of the split above the focused window. Returns false when there is none. */
    swapSplit(workspace: NodeSource): boolean {
        const split = this.#focusedLeaf(workspace)?.parent;
        if (split === null || split === undefined) {
            return false;
        }
        [split.a, split.b] = [split.b, split.a];
        this.revision += 1;
        return true;
    }

    /**
     * Turns the split above the focused window by `angle` degrees clockwise, a multiple of 90 (default 90; negative
     * turns the other way): the first half goes from the left to the top to the right to the bottom. 180 swaps the
     * halves. Like `toggleSplit`, the direction part needs `preserveSplit`. Returns false when there is no split above
     * the focused window.
     */
    rotateSplit(workspace: NodeSource, angle = 90): boolean {
        if (typeof angle !== "number" || !Number.isFinite(angle) || angle % 90 !== 0) {
            throw new RangeError(`DwindleLayout rotateSplit angle must be a multiple of 90, got ${String(angle)}`);
        }
        const split = this.#focusedLeaf(workspace)?.parent;
        if (split === null || split === undefined) {
            return false;
        }
        const turns = (((Math.trunc(angle / 90) % 4) + 4) % 4) as 0 | 1 | 2 | 3;
        const flip = turns === 1 || turns === 3;
        if ((turns === 1 && split.dir === "v") || turns === 2 || (turns === 3 && split.dir === "h")) {
            [split.a, split.b] = [split.b, split.a];
        }
        if (flip) {
            split.dir = split.dir === "h" ? "v" : "h";
        }
        this.revision += 1;
        return true;
    }

    /**
     * Makes the focused window a direct child of the root split: it takes the place of the root's other half, which
     * goes where the window was. Stable (the default) keeps the window on the same side of the screen; unstable puts it
     * on the other. Returns false when the window is alone or already a child of the root.
     */
    moveToRoot(workspace: NodeSource, options: { stable?: boolean } = {}): boolean {
        const stable = options.stable ?? true;
        const leaf = this.#focusedLeaf(workspace);
        const above = leaf?.parent;
        if (leaf === null || above === null || above === undefined || above.parent === null) {
            return false;
        }
        let ancestor: Tree = above;
        let root: Split = above.parent;
        while (root.parent !== null) {
            ancestor = root;
            root = root.parent;
        }
        const other = root.a === ancestor ? root.b : root.a;
        if (above.a === leaf) {
            above.a = other;
        } else {
            above.b = other;
        }
        if (root.a === other) {
            root.a = leaf;
        } else {
            root.b = leaf;
        }
        other.parent = above;
        leaf.parent = root;
        if (stable) {
            [root.a, root.b] = [root.b, root.a];
        }
        this.revision += 1;
        return true;
    }

    /**
     * Changes the share of the split above the focused window: by `amount` (a fraction, so 0.1 moves the divider a tenth
     * of the split), or to it with `exact`, held to 0.1 to 0.9. Hyprland's `splitratio` takes twice these numbers. The
     * split keeps the ratio, whatever the layout's `ratio` becomes. Returns false when the window has no split above it.
     */
    splitRatio(workspace: NodeSource, amount: number, options: { exact?: boolean } = {}): boolean {
        if (typeof amount !== "number" || !Number.isFinite(amount)) {
            throw new TypeError(`DwindleLayout splitRatio needs a finite number, got ${String(amount)}`);
        }
        const split = this.#focusedLeaf(workspace)?.parent;
        const geometry = this.#geometry;
        if (split === null || split === undefined || geometry === null) {
            return false;
        }
        const current = split.ratio ?? inheritedShare(split, geometry.shape.ratio);
        split.ratio = clamp(options.exact === true ? amount : current + amount, MIN_RATIO, MAX_RATIO);
        this.revision += 1;
        return true;
    }

    /**
     * Toggles pseudotiling of the focused window: it keeps its own size, centred in its cell and held to the cell's size,
     * instead of filling the cell. The size is `size`, else the window's `width` and `height` (with its `frameInset`),
     * else the size its cell has at the next compute. `size` is read only when the window becomes pseudotiled. Returns
     * whether the window is pseudotiled after the call; false also when no window with a tile has focus.
     */
    pseudotile(workspace: NodeSource, size?: DwindleSize): boolean {
        if (size !== undefined) {
            const { width, height } = size;
            if (!Number.isFinite(width) || !Number.isFinite(height) || !(width > 0) || !(height > 0)) {
                throw new RangeError(
                    `DwindleLayout pseudotile size must be finite numbers above 0, got ${String(width)} by ${String(height)}`,
                );
            }
        }
        const leaf = this.#focusedLeaf(workspace);
        if (leaf === null) {
            return false;
        }
        const on = !this.#pseudo.has(leaf.leaf);
        if (on) {
            const own = size === undefined ? ownSize(leaf.leaf) : { width: size.width, height: size.height };
            this.#pseudo.set(leaf.leaf, own);
        } else {
            this.#pseudo.delete(leaf.leaf);
        }
        this.revision += 1;
        return on;
    }

    /** Whether `node` is pseudotiled. */
    pseudotiled(node: LayoutNode): boolean {
        return this.#pseudo.has(node);
    }

    /** Checks the options, stores the geometry of this compute and returns it. */
    #check(context: LayoutContext): { area: Rect; shape: Shape } {
        if (!SPLITS.includes(this.split)) {
            throw new TypeError("DwindleLayout split must be aspect, horizontal or vertical");
        }
        if (typeof this.ratio !== "number" || !Number.isFinite(this.ratio)) {
            throw new RangeError("DwindleLayout ratio must be a finite number");
        }
        if (this.forceSplit !== 0 && this.forceSplit !== 1 && this.forceSplit !== 2) {
            throw new RangeError(`DwindleLayout forceSplit must be 0, 1 or 2, got ${String(this.forceSplit)}`);
        }
        if (this.splitBias !== 0 && this.splitBias !== 1) {
            throw new RangeError(`DwindleLayout splitBias must be 0 or 1, got ${String(this.splitBias)}`);
        }
        if (typeof this.splitWidthMultiplier !== "number" || !Number.isFinite(this.splitWidthMultiplier)) {
            throw new RangeError("DwindleLayout splitWidthMultiplier must be a finite number");
        }
        for (const name of FLAGS) {
            checkFlag(this, name);
        }
        if (this.pointer !== null && typeof this.pointer !== "function") {
            throw new TypeError("DwindleLayout pointer must be a function or null");
        }
        const { area, gap } = tilingGeometry(this.gap, context);
        const shape: Shape = {
            gap,
            ratio: clamp(this.ratio, MIN_RATIO, MAX_RATIO),
            preserve: this.preserveSplit || this.smartSplit,
            multiplier: clamp(this.splitWidthMultiplier, MIN_MULTIPLIER, MAX_MULTIPLIER),
        };
        const geometry = { area, shape };
        this.#geometry = geometry;
        return geometry;
    }

    /** The pointer, or null when there is no source for it or it gave something that is not a point. */
    #pointer(): Point | null {
        const at = this.pointer === null ? null : this.pointer();
        return at !== null && Number.isFinite(at.x) && Number.isFinite(at.y) ? at : null;
    }

    /** Puts the new `node` in the tree, splitting the cell of the window the options choose. */
    #open(node: LayoutNode, area: Rect, shape: Shape): void {
        if (this.#root === null) {
            this.#root = this.#leafFor(node, null);
            return;
        }
        const rects = cells(this.#root, area, shape);
        const at = this.#pointer();
        let target: LayoutNode;
        if (!this.useActiveForSplits) {
            target = at === null ? (this.#leaves.keys().next().value as LayoutNode) : nearest(rects, at);
        } else if (this.#focus !== null && this.#leaves.has(this.#focus)) {
            target = this.#focus;
        } else {
            target = at === null ? this.#lastLeaf() : nearest(rects, at);
        }
        const cell = rects.get(target) as Rect;
        const mult = shape.multiplier;
        let dir: "h" | "v";
        let first = false;
        let fixed = false;
        if (this.#preselect !== "none") {
            const choice = this.#preselect;
            dir = choice === "left" || choice === "right" ? "h" : "v";
            first = choice === "left" || choice === "up";
            fixed = true;
            if (!this.permanentDirectionOverride) {
                this.#preselect = "none";
            }
        } else {
            if (this.split !== "aspect") {
                dir = this.split === "horizontal" ? "h" : "v";
                fixed = true;
            } else if (shape.preserve) {
                dir = cell.width > cell.height * mult ? "h" : "v";
            } else {
                dir = cell.height * mult > cell.width ? "v" : "h";
            }
            if (this.smartSplit && at !== null) {
                // The cell's diagonals make four triangles; the pointer's triangle gives the direction and the side.
                const dx = at.x - (cell.x + cell.width / 2);
                const dy = at.y - (cell.y + cell.height / 2);
                if (Math.abs(dy / dx) < cell.height / cell.width) {
                    dir = "h";
                    first = dx < 0;
                } else {
                    dir = "v";
                    first = !(dy > 0);
                }
            } else if (this.forceSplit === 1) {
                first = true;
            } else if (this.forceSplit === 0 && at !== null) {
                first = dir === "h" ? at.x < cell.x + cell.width / 2 : at.y < cell.y + cell.height / 2;
            }
        }
        this.#insert(target, node, dir, first, fixed, this.splitBias === 1 && first);
    }

    #leafFor(node: LayoutNode, parent: Split | null): Leaf {
        const leaf: Leaf = { leaf: node, parent };
        this.#leaves.set(node, leaf);
        return leaf;
    }

    /** The last leaf in reading order: the one a new node splits when none had focus. */
    #lastLeaf(): LayoutNode {
        let node = this.#root as Tree;
        while (!isLeaf(node)) {
            node = node.b;
        }
        return node.leaf;
    }

    /** The leaf of the focused node of `workspace` that the tree holds, or null. */
    #focusedLeaf(workspace: NodeSource): Leaf | null {
        for (const node of workspace.clients) {
            if ((node as FocusableNode).focused) {
                return this.#leaves.get(node) ?? null;
            }
        }
        return null;
    }

    /** Replaces the leaf `target` with a split of `target` and the new leaf `node`, which is first when `first`. */
    #insert(
        target: LayoutNode,
        node: LayoutNode,
        dir: "h" | "v",
        first: boolean,
        fixed: boolean,
        biased: boolean,
    ): void {
        const old = this.#leaves.get(target) as Leaf;
        const split: Split = { dir, fixed, biased, ratio: null, a: old, b: old, parent: old.parent };
        const fresh = this.#leafFor(node, split);
        split.a = first ? fresh : old;
        split.b = first ? old : fresh;
        this.#replace(old, split);
        old.parent = split;
    }

    /** Removes the leaf `node`; the sibling subtree takes the split's place. */
    #remove(node: LayoutNode): void {
        const leaf = this.#leaves.get(node) as Leaf;
        this.#leaves.delete(node);
        this.#pseudo.delete(node);
        const parent = leaf.parent;
        if (parent === null) {
            this.#root = null;
            return;
        }
        const sibling = parent.a === leaf ? parent.b : parent.a;
        sibling.parent = parent.parent;
        this.#replace(parent, sibling);
    }

    /** Puts `next` where `old` hangs: in its parent's `a` or `b`, or at the root. */
    #replace(old: Tree, next: Tree): void {
        const parent = old.parent;
        if (parent === null) {
            this.#root = next;
        } else if (parent.a === old) {
            parent.a = next;
        } else {
            parent.b = next;
        }
    }
}

/** One split of a dwindle tree as a divider a drag moves. A split the drag set has a ratio of its own until `reset`. */
class DwindleDivider implements LayoutSplit {
    readonly axis: "x" | "y";
    readonly span: number;
    readonly direction = 1;
    readonly min = MIN_RATIO;
    readonly max = MAX_RATIO;
    readonly cross: DwindleDivider | null;
    readonly #changed: () => void;
    readonly #split: Split;
    readonly #inherited: number;
    readonly #original: number | null;

    constructor(split: Split, span: number, inherited: number, changed: () => void, cross: DwindleDivider | null) {
        this.#changed = changed;
        this.#split = split;
        this.#inherited = inherited;
        this.#original = split.ratio;
        this.axis = split.dir === "h" ? "x" : "y";
        this.span = span;
        this.cross = cross;
    }

    get ratio(): number {
        return clamp(this.#split.ratio ?? this.#inherited, MIN_RATIO, MAX_RATIO);
    }

    set ratio(value: number) {
        this.#split.ratio = clamp(value, MIN_RATIO, MAX_RATIO);
        this.#changed();
    }

    reset(): void {
        this.#split.ratio = this.#original;
        this.#changed();
    }
}
