// DwindleLayout: a binary split tree in the style of Hyprland. Geometry is data/lua/gnoblin/shell/tiling.lua
// `Dwindle`. Each new node splits the cell of the node that had focus when it arrived (or the last node's cell) in
// two. A split keeps the direction it was created with: "horizontal" and "vertical" fix it, "aspect" takes the longer
// side of the cell at creation. `ratio` is the share of the first child of every split and `gap` separates the two
// halves of each split.
//
// The tree is state the layout keeps between computes, as the Lua class kept one per workspace: a node that
// goes collapses its split, a node that arrives is inserted. That makes `compute` depend on its history, so one
// instance must serve one workspace. Every walk is a loop over an explicit stack: a run of windows that each split
// the last one makes a tree as deep as it is long, and the engine's call stack is short.
//
// It registers two default bindings in `layout.bindings`: `swap` (Super+MouseLeft+Drag, `ClientSwapBinding`) trades the
// places of two leaves in the tree, and `resize` (Super+MouseRight+Drag, `SplitResizeBinding`) drags one split. A dragged
// split keeps a ratio of its own, which `layout.ratio` no longer sets for it; a cancelled drag gives the split back to
// `layout.ratio`. `triggers` replaces a trigger, and `null` leaves that binding out.

import { ClientSwapBinding, resolveTriggers, SplitResizeBinding } from "gnoblin/bindings";
import type { LayoutSplit, Point } from "gnoblin/bindings";
import { checkGap, clamp, Layout, tile, tilingGeometry } from "gnoblin/layout";
import type { LayoutContext, LayoutNode, Placement, Rect } from "gnoblin/layout";

export type DwindleSplit = "aspect" | "horizontal" | "vertical";

export interface DwindleOptions {
    split?: DwindleSplit;
    /** The first child's share of every split, held to 0.1 to 0.9. Default 0.5. */
    ratio?: number;
    /** Overrides the workspace's inner gap. */
    gap?: number;
    /** `undefined` keeps the default trigger, a string replaces it and `null` leaves the binding out. */
    triggers?: { swap?: string | null; resize?: string | null };
}

const DEFAULT_TRIGGERS = { swap: "Super+MouseLeft+Drag", resize: "Super+MouseRight+Drag" } as const;

const SPLITS: readonly string[] = ["aspect", "horizontal", "vertical"];
const MIN_RATIO = 0.1;
const MAX_RATIO = 0.9;

interface Leaf {
    /** The node in this place of the tree. A swap puts the other one here. */
    leaf: LayoutNode;
    parent: Split | null;
}

/** `"h"` puts `a` left of `b`; `"v"` puts `a` above `b`. */
interface Split {
    readonly dir: "h" | "v";
    /** The first child's share a drag set for this split alone, or null while the split follows the layout's `ratio`. */
    ratio: number | null;
    a: Tree;
    b: Tree;
    parent: Split | null;
}

type Tree = Leaf | Split;

function isLeaf(tree: Tree): tree is Leaf {
    return "leaf" in tree;
}

/** The rectangle of every leaf under `root` inside `area`; with `splits`, the rectangle each split divides as well. */
function cells(root: Tree, area: Rect, gap: number, ratio: number, splits?: Map<Split, Rect>): Map<LayoutNode, Rect> {
    const out = new Map<LayoutNode, Rect>();
    const pending: { node: Tree; rect: Rect }[] = [{ node: root, rect: area }];
    for (let next = pending.pop(); next !== undefined; next = pending.pop()) {
        const { node, rect } = next;
        if (isLeaf(node)) {
            out.set(node.leaf, rect);
        } else if (node.dir === "h") {
            splits?.set(node, rect);
            const first = Math.max(0, (rect.width - gap) * (node.ratio ?? ratio));
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
            splits?.set(node, rect);
            const first = Math.max(0, (rect.height - gap) * (node.ratio ?? ratio));
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

export class DwindleLayout extends Layout {
    split: DwindleSplit;
    ratio: number;
    gap: number | undefined;
    readonly triggers: Readonly<Record<"swap" | "resize", string | null>>;
    /**
     * How many times a drag changed the ratio of one split. The tree is not a field the workspace can compare, so this
     * counter is: a change to it is what makes the workspace compute again.
     */
    revision = 0;
    #geometry: { area: Rect; gap: number; ratio: number } | null = null;
    #root: Tree | null = null;
    #leaves = new Map<LayoutNode, Leaf>();
    /** The focus the previous compute saw: the node a new one splits, because the new one takes focus on arrival. */
    #focus: LayoutNode | null = null;

    constructor(options: DwindleOptions = {}) {
        super();
        this.split = options.split ?? "aspect";
        this.ratio = options.ratio ?? 0.5;
        this.gap = checkGap(options.gap, "DwindleLayout");
        this.triggers = Object.freeze(resolveTriggers(DEFAULT_TRIGGERS, options.triggers));
        if (this.triggers.swap !== null) {
            this.bindings.add(new ClientSwapBinding({ id: "swap", trigger: this.triggers.swap }));
        }
        if (this.triggers.resize !== null) {
            this.bindings.add(new SplitResizeBinding({ id: "resize", trigger: this.triggers.resize }));
        }
    }

    compute(context: LayoutContext): Placement[] {
        const nodes = context.children;
        if (!SPLITS.includes(this.split)) {
            throw new TypeError("DwindleLayout split must be aspect, horizontal or vertical");
        }
        if (typeof this.ratio !== "number" || !Number.isFinite(this.ratio)) {
            throw new RangeError("DwindleLayout ratio must be a finite number");
        }
        const ratio = clamp(this.ratio, MIN_RATIO, MAX_RATIO);
        const { area, gap } = tilingGeometry(this.gap, context);
        this.#geometry = { area, gap, ratio };
        const live = new Set<LayoutNode>(nodes);
        for (const node of [...this.#leaves.keys()]) {
            if (!live.has(node)) {
                this.#remove(node);
            }
        }
        for (const node of nodes) {
            if (this.#leaves.has(node)) {
                continue;
            }
            if (this.#root === null) {
                this.#root = this.#leafFor(node, null);
                continue;
            }
            const target = this.#focus !== null && this.#leaves.has(this.#focus) ? this.#focus : this.#lastLeaf();
            const cell = cells(this.#root, area, gap, ratio).get(target) as Rect;
            const dir = this.split === "aspect" ? (cell.width >= cell.height ? "horizontal" : "vertical") : this.split;
            this.#insert(target, node, dir === "horizontal" ? "h" : "v");
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
        const rects = cells(this.#root, area, gap, ratio);
        return nodes.map((node) => {
            const r = rects.get(node) as Rect;
            return tile(node, r.x, r.y, r.width, r.height);
        });
    }

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
     * The split of the last compute whose divider is nearest `point`, among the splits above `client`'s tile: the ones
     * that bound the tile or a bigger region around it. A tie goes to the split nearer the tile.
     */
    findSplit(client: LayoutNode, point: Point): LayoutSplit | null {
        const geometry = this.#geometry;
        const leaf = this.#leaves.get(client);
        if (geometry === null || this.#root === null || leaf === undefined) {
            return null;
        }
        const regions = new Map<Split, Rect>();
        cells(this.#root, geometry.area, geometry.gap, geometry.ratio, regions);
        let best: { split: Split; distance: number; span: number } | null = null;
        for (let split = leaf.parent; split !== null; split = split.parent) {
            const region = regions.get(split);
            if (region === undefined) {
                continue;
            }
            const along = split.dir === "h" ? region.width : region.height;
            const first = Math.max(0, (along - geometry.gap) * (split.ratio ?? geometry.ratio));
            const divider = (split.dir === "h" ? region.x : region.y) + first + geometry.gap / 2;
            const distance = Math.abs((split.dir === "h" ? point.x : point.y) - divider);
            if (along - geometry.gap > 0 && (best === null || distance < best.distance)) {
                best = { split, distance, span: along - geometry.gap };
            }
        }
        if (best === null) {
            return null;
        }
        const bump = (): void => {
            this.revision += 1;
        };
        return new DwindleDivider(best.split, best.span, geometry.ratio, bump);
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

    /** Replaces the leaf `target` with a split of `target` and the new leaf `node`. */
    #insert(target: LayoutNode, node: LayoutNode, dir: "h" | "v"): void {
        const old = this.#leaves.get(target) as Leaf;
        const split: Split = { dir, ratio: null, a: old, b: old, parent: old.parent };
        split.b = this.#leafFor(node, split);
        this.#replace(old, split);
        old.parent = split;
    }

    /** Removes the leaf `node`; the sibling subtree takes the split's place. */
    #remove(node: LayoutNode): void {
        const leaf = this.#leaves.get(node) as Leaf;
        this.#leaves.delete(node);
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
    readonly #changed: () => void;
    readonly #split: Split;
    readonly #inherited: number;
    readonly #original: number | null;

    constructor(split: Split, span: number, inherited: number, changed: () => void) {
        this.#changed = changed;
        this.#split = split;
        this.#inherited = inherited;
        this.#original = split.ratio;
        this.axis = split.dir === "h" ? "x" : "y";
        this.span = span;
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
