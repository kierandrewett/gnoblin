import type { Point } from "gnoblin-core/bindings/types";
import type { LayoutNode } from "gnoblin-std/layout";
import { NativeLayout } from "gnoblin-std/layouts/native";
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
    triggers?: {
        swap?: string | null;
        resize?: string | null;
    };
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
/** A dwindle tiling layout. Each new node splits the cell it arrives in, as the header describes. */
export declare class DwindleLayout extends NativeLayout {
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
    revision: number;
    constructor(options?: DwindleOptions);
    protected get kind(): number;
    protected get options(): OptionSpecs;
    protected pointerOf(): Point | null;
    protected ownSizeOf(node: LayoutNode): OwnSize | null;
    protected changed(): void;
    /** The direction `preselect` set for the next window, or `none`. */
    get preselected(): DwindleDirection;
    /**
     * Sets where the next window opens: `left` or `up` puts it first (left or above) with a split that goes that way,
     * `right` or `down` second. The split keeps its direction whatever its cell's shape. The choice is used once, unless
     * `permanentDirectionOverride` is set; `none` clears it. Nothing is changed in the tree, and the first window of a
     * workspace does not use it.
     */
    preselect(direction: DwindleDirection): void;
    /**
     * Turns the split above the focused window from side by side to top and bottom or back. A split that follows the
     * shape of its cell takes its direction from the cell again at once, so this needs `preserveSplit` (or a split that
     * is fixed), as in Hyprland. Returns false when no window with a split above it has focus.
     */
    toggleSplit(workspace: NodeSource): boolean;
    /** Swaps the two halves of the split above the focused window. Returns false when there is none. */
    swapSplit(workspace: NodeSource): boolean;
    /**
     * Turns the split above the focused window by `angle` degrees clockwise, a multiple of 90 (default 90; negative
     * turns the other way): the first half goes from the left to the top to the right to the bottom. 180 swaps the
     * halves. Like `toggleSplit`, the direction part needs `preserveSplit`. Returns false when there is no split above
     * the focused window.
     */
    rotateSplit(workspace: NodeSource, angle?: number): boolean;
    /**
     * Makes the focused window a direct child of the root split: it takes the place of the root's other half, which
     * goes where the window was. Stable (the default) keeps the window on the same side of the screen; unstable puts it
     * on the other. Returns false when the window is alone or already a child of the root.
     */
    moveToRoot(workspace: NodeSource, options?: {
        stable?: boolean;
    }): boolean;
    /**
     * Changes the share of the split above the focused window: by `amount` (a fraction, so 0.1 moves the divider a tenth
     * of the split), or to it with `exact`, held to 0.1 to 0.9. Hyprland's `splitratio` takes twice these numbers. The
     * split keeps the ratio, whatever the layout's `ratio` becomes. Returns false when the window has no split above it.
     */
    splitRatio(workspace: NodeSource, amount: number, options?: {
        exact?: boolean;
    }): boolean;
    /**
     * Toggles pseudotiling of the focused window: it keeps its own size, centred in its cell and held to the cell's size,
     * instead of filling the cell. The size is `size`, else the window's `width` and `height` (with its `frameInset`),
     * else the size its cell has at the next compute. `size` is read only when the window becomes pseudotiled. Returns
     * whether the window is pseudotiled after the call; false also when no window with a tile has focus.
     */
    pseudotile(workspace: NodeSource, size?: DwindleSize): boolean;
    /** Whether `node` is pseudotiled. */
    pseudotiled(node: LayoutNode): boolean;
}
export {};
