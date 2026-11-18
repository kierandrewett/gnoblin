// DwindleLayout keeps a binary split tree for one workspace and places windows as Hyprland's dwindle layout does. A new
// window splits the cell of the focused window (the last window's cell when none has focus) in two.
//
// A split's direction is not fixed. Every compute reads the cell the split divides: wider than tall, the two halves sit
// side by side; taller than wide, one above the other. `preserveSplit` (or `smartSplit`, which implies it) keeps the
// direction a split had when it was made, as does the `split` option when it is `horizontal` or `vertical` and a
// split made by `preselect`. `splitWidthMultiplier` scales the height in that comparison.
//
// The tree persists between computes: removing a node collapses its split, and adding one inserts a leaf. The tree, the
// splitting rules and the actions are the library's (`libgnoblin-std`, kind `dwindle`); this class holds the options and the
// bindings, and tells the library which window has the keyboard and where the pointer is.
//
// Ratios are fractions of the first child, 0.1 to 0.9. Hyprland's `splitRatio` and `default_split_ratio` are twice that:
// 1.0 there is 0.5 here, and Hyprland's 0.1 to 1.9 range is 0.05 to 0.95 there and 0.1 to 0.9 here.
//
// The actions (`toggleSplit`, `swapSplit`, `rotateSplit`, `preselect`, `moveToRoot`, `splitRatio`, `pseudotile`) are
// methods a key binding calls with the workspace; they change the tree and the workspace computes again (`revision`
// counts the changes, as the tree is not a field the workspace can compare). A split a drag or `splitRatio` set keeps
// its own ratio until reset.
//
// Default `swap` and `resize` bindings exchange leaves and drag split dividers. `triggers` replaces a binding trigger,
// and `null` leaves that binding out.

import { resolveTriggers } from "gnoblin-core/bindings/binding";
import { addTilingBindings } from "gnoblin-std/bindings/tiling";
import type { Point } from "gnoblin-core/bindings/types";
import { growRect, NO_INSET } from "gnoblin-core/inset";
import type { FrameInset } from "gnoblin-core/inset";
import {
    DwindleDirection as DirectionMembers,
    DwindleForceSplit,
    DwindlePointer,
    DwindleSplit as SplitMembers,
    DwindleSplitBias,
} from "gnoblin-std/generated/layouts";
import { BuiltinKind } from "gnoblin-std/generated/constants";
import { checkGap } from "gnoblin-std/layout";
import type { LayoutNode } from "gnoblin-std/layout";
import { argOf, NativeLayout, nodeArgOf } from "gnoblin-std/layouts/native";
import type { ActionSource, OptionSpecs, OwnSize } from "gnoblin-std/layouts/native";

/** What an action takes of a workspace: the nodes it holds. */
type NodeSource = ActionSource;

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

/** The words of `forceSplit`, by its number. */
const FORCE_SPLIT = ["by_pointer", "first", "second"] as const;
/** The words of `splitBias`, by its number. */
const SPLIT_BIAS = ["first", "opened"] as const;

const OPTIONS: OptionSpecs = {
    split: { native: "split", members: SplitMembers },
    ratio: { native: "ratio" },
    gap: { native: "gap" },
    forceSplit: {
        native: "force_split",
        members: DwindleForceSplit,
        toNative: (value) => (typeof value === "number" ? (FORCE_SPLIT[value] ?? value) : value),
        fromNative: (value) => FORCE_SPLIT.indexOf(value as (typeof FORCE_SPLIT)[number]),
    },
    preserveSplit: { native: "preserve_split" },
    smartSplit: { native: "smart_split" },
    smartResizing: { native: "smart_resizing" },
    permanentDirectionOverride: { native: "permanent_direction_override" },
    splitWidthMultiplier: { native: "split_width_multiplier" },
    useActiveForSplits: { native: "use_active_for_splits" },
    splitBias: {
        native: "split_bias",
        members: DwindleSplitBias,
        toNative: (value) => (typeof value === "number" ? (SPLIT_BIAS[value] ?? value) : value),
        fromNative: (value) => SPLIT_BIAS.indexOf(value as (typeof SPLIT_BIAS)[number]),
    },
    // The library reads the pointer from the query when it is told the seat's is the one to read; the function is the script's.
    pointer: {
        native: "pointer",
        members: DwindlePointer,
        readOnly: true,
        toNative: (value) => {
            if (value === null || value === undefined) {
                return "none";
            }
            if (typeof value !== "function") {
                throw new TypeError("DwindleLayout pointer must be a function or null");
            }
            return "seat";
        },
    },
};

/** The own size of `node` with its frame, when it has a width and a height; else null. */
function ownSize(node: LayoutNode): OwnSize | null {
    const { width, height, frameInset } = node as { width?: unknown; height?: unknown; frameInset?: FrameInset };
    if (typeof width !== "number" || typeof height !== "number" || !(width > 0) || !(height > 0)) {
        return null;
    }
    const outer = growRect({ x: 0, y: 0, width, height }, frameInset ?? NO_INSET);
    return { width: outer.width, height: outer.height };
}

/** The node of `source` that has the keyboard, if any. */
function focusedOf(source: NodeSource): LayoutNode | undefined {
    for (const node of source.clients) {
        if ((node as { focused?: unknown }).focused === true) {
            return node;
        }
    }
    return undefined;
}

/** A dwindle tiling layout. Each new node splits the cell it arrives in, as the header describes. */
export class DwindleLayout extends NativeLayout {
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

    protected get kind(): number {
        return BuiltinKind.DWINDLE;
    }

    protected get options(): OptionSpecs {
        return OPTIONS;
    }

    protected override pointerOf(): Point | null {
        const at = this.pointer === null ? null : this.pointer();
        return at !== null && Number.isFinite(at.x) && Number.isFinite(at.y) ? at : null;
    }

    protected override ownSizeOf(node: LayoutNode): OwnSize | null {
        return ownSize(node);
    }

    protected override changed(): void {
        this.revision += 1;
    }

    /** The direction `preselect` set for the next window, or `none`. */
    get preselected(): DwindleDirection {
        const answer = this.act("preselected", [], { clients: [] }, false);
        const word = Object.entries(DirectionMembers).find(([, member]) => member === answer);
        return (word === undefined ? "none" : word[0].toLowerCase()) as DwindleDirection;
    }

    /**
     * Sets where the next window opens: `left` or `up` puts it first (left or above) with a split that goes that way,
     * `right` or `down` second. The split keeps its direction whatever its cell's shape. The choice is used once, unless
     * `permanentDirectionOverride` is set; `none` clears it. Nothing is changed in the tree, and the first window of a
     * workspace does not use it.
     */
    preselect(direction: DwindleDirection): void {
        this.act("preselect", [argOf("direction", direction, DirectionMembers)], { clients: [] }, false);
    }

    /**
     * Turns the split above the focused window from side by side to top and bottom or back. A split that follows the
     * shape of its cell takes its direction from the cell again at once, so this needs `preserveSplit` (or a split that
     * is fixed), as in Hyprland. Returns false when no window with a split above it has focus.
     */
    toggleSplit(workspace: NodeSource): boolean {
        return this.act("toggle_split", [], workspace) as boolean;
    }

    /** Swaps the two halves of the split above the focused window. Returns false when there is none. */
    swapSplit(workspace: NodeSource): boolean {
        return this.act("swap_split", [], workspace) as boolean;
    }

    /**
     * Turns the split above the focused window by `angle` degrees clockwise, a multiple of 90 (default 90; negative
     * turns the other way): the first half goes from the left to the top to the right to the bottom. 180 swaps the
     * halves. Like `toggleSplit`, the direction part needs `preserveSplit`. Returns false when there is no split above
     * the focused window.
     */
    rotateSplit(workspace: NodeSource, angle = 90): boolean {
        return this.act("rotate_split", [argOf("angle", angle)], workspace) as boolean;
    }

    /**
     * Makes the focused window a direct child of the root split: it takes the place of the root's other half, which
     * goes where the window was. Stable (the default) keeps the window on the same side of the screen; unstable puts it
     * on the other. Returns false when the window is alone or already a child of the root.
     */
    moveToRoot(workspace: NodeSource, options: { stable?: boolean } = {}): boolean {
        return this.act("move_to_root", [argOf("stable", options.stable ?? true)], workspace) as boolean;
    }

    /**
     * Changes the share of the split above the focused window: by `amount` (a fraction, so 0.1 moves the divider a tenth
     * of the split), or to it with `exact`, held to 0.1 to 0.9. Hyprland's `splitratio` takes twice these numbers. The
     * split keeps the ratio, whatever the layout's `ratio` becomes. Returns false when the window has no split above it.
     */
    splitRatio(workspace: NodeSource, amount: number, options: { exact?: boolean } = {}): boolean {
        const action = options.exact === true ? "set_split_ratio" : "split_ratio";
        const name = options.exact === true ? "ratio" : "amount";
        return this.act(action, [argOf(name, amount)], workspace) as boolean;
    }

    /**
     * Toggles pseudotiling of the focused window: it keeps its own size, centred in its cell and held to the cell's size,
     * instead of filling the cell. The size is `size`, else the window's `width` and `height` (with its `frameInset`),
     * else the size its cell has at the next compute. `size` is read only when the window becomes pseudotiled. Returns
     * whether the window is pseudotiled after the call; false also when no window with a tile has focus.
     */
    pseudotile(workspace: NodeSource, size?: DwindleSize): boolean {
        const focused = focusedOf(workspace);
        const was = focused !== undefined && this.pseudotiled(focused);
        const values = [argOf("width", size?.width), argOf("height", size?.height)];
        const on = this.act("pseudotile", values, workspace, false) as boolean;
        if (on || was) {
            this.changed();
        }
        return on;
    }

    /** Whether `node` is pseudotiled. */
    pseudotiled(node: LayoutNode): boolean {
        return this.act("is_pseudotiled", [nodeArgOf("client", node)], { clients: [node] }, false) as boolean;
    }
}
