import { PointerDragBinding } from "gnoblin-core/bindings/binding";
import type { BindingCollection, PointerDragBindingOptions } from "gnoblin-core/bindings/binding";
import type { BindingClient, DragEvent, Point } from "gnoblin-core/bindings/types";
/**
 * What the tiling bindings read of a client. `tiled` is the flag a layout sets when it places the client, and it stays
 * set when a script floats the client afterwards, so `floating` and `parentWindow` (a dialog is never tiled) are
 * read as well.
 */
export interface TiledClient extends BindingClient {
    /** True when the layout places the client in a tile. */
    readonly tiled?: boolean;
    /** True when the client is floating instead of occupying a tile. */
    readonly floating?: boolean;
    /** Parent window when the client is a dialog. */
    readonly parentWindow?: unknown;
    /** Whether the client is visible to the user. */
    readonly visible?: boolean;
    /** Workspace that holds the client's tile. */
    readonly workspace?: OrderedWorkspace | null;
}
/** The part of a workspace the swap binding needs: its clients in layout order, and a way to trade two places. */
export interface OrderedWorkspace {
    /** Clients in layout order; `swap` trades their positions. */
    readonly clients: Iterable<TiledClient> & {
        swap(a: TiledClient, b: TiledClient): void;
    };
}
/** What `ClientSwapBinding.onStart` keeps for one drag. `over` is the tile the pointer last was over, or null. */
export interface SwapState {
    /** Client being dragged for a swap. */
    dragged: TiledClient;
    /** Workspace whose client order may change on release. */
    workspace: OrderedWorkspace;
    /** Tile last under the pointer, or null when none was found. */
    over: TiledClient | null;
}
/**
 * Super+MouseLeft drag in a tiling layout: drop the tile you hold on another tile and the two trade places in the
 * workspace's order, so the layout puts each where the other was. Nothing moves while the pointer does: the order
 * changes once, on release, and the layout computes once. A release outside every tile, over the tile itself or
 * after a cancel leaves the order as it was. Only a tiled window takes part; a floating, fullscreen or maximised
 * one declines.
 */
export declare class ClientSwapBinding extends PointerDragBinding {
    constructor(options?: Partial<PointerDragBindingOptions>);
    /** Starts a swap when the pressed client belongs to a tiled workspace. */
    onStart(event: DragEvent): SwapState | false;
    /** Tracks the tile currently under the pointer. */
    onUpdate(event: DragEvent, state: object): void;
    /** Swaps the dragged client with the tile under the release point. */
    onEnd(event: DragEvent, state: object): void;
}
/** What `SplitResizeBinding.onStart` keeps for one drag. */
export interface SplitState {
    /** Divider being resized. */
    split: LayoutSplit;
    /** Pointer position when the drag started, in logical pixels. */
    start: Point;
    /** Divider ratio when the drag started. */
    startRatio: number;
    /** The second divider the same drag moves along the other axis (`LayoutSplit.cross`), or null. */
    cross: {
        split: LayoutSplit;
        startRatio: number;
    } | null;
}
/**
 * A divider a layout lets a drag move: what a layout's `findSplit` returns. Dragging the pointer `span` pixels along
 * `axis` moves `ratio` by one, in the direction `direction` says, and the divider follows the pointer. The layout holds
 * `ratio` to `min` and `max` itself; the binding clamps to them as well so that `reset` is the only way back.
 */
export interface LayoutSplit {
    /** `x` for a divider the pointer's horizontal movement carries, `y` for one its vertical movement does. */
    readonly axis: "x" | "y";
    /** Pixels of pointer travel that change `ratio` by 1. Always above zero. */
    readonly span: number;
    /** 1 when moving the pointer right or down grows `ratio`, -1 when it shrinks it. */
    readonly direction: 1 | -1;
    /** Lowest ratio the layout allows. */
    readonly min: number;
    /** Highest ratio the layout allows. */
    readonly max: number;
    /**
     * A second divider, on the other axis, that the same drag moves: the pointer's travel along `cross.axis` moves it as
     * the travel along `axis` moves this one. A layout offers one when a drag is to resize a tile by both of the
     * dividers that bound it (Hyprland's dwindle with `smart_resizing` off). Absent or null: one divider.
     */
    readonly cross?: LayoutSplit | null;
    /** Current divider position as a ratio. */
    ratio: number;
    /** Puts the split back as it was when `findSplit` returned it: a drag cancelled. */
    reset(): void;
}
/**
 * Super+MouseRight drag in a tiling layout: moves the divider nearest the press by as far as the pointer travels, so
 * the tiles on both sides follow it while the pointer is down. The layout decides which divider that is and how its
 * ratio is held (`Layout.findSplit`); a layout with no divider (a grid, a monocle) has none, and this declines. The
 * ratio is live, so a release keeps it and a cancel puts it back. A window that is not tiled, or is fullscreen or
 * maximised, declines. A layout may hand back a second divider on the other axis (`LayoutSplit.cross`); the same
 * drag then moves both, each by the pointer's travel along its own axis.
 */
export declare class SplitResizeBinding extends PointerDragBinding {
    #private;
    constructor(options?: Partial<PointerDragBindingOptions>);
    /** Starts resizing the nearest divider when the layout provides one. */
    onStart(event: DragEvent): SplitState | false;
    /** Updates the divider ratios from pointer travel. */
    onUpdate(event: DragEvent, state: object): void;
    /** Restores the divider position from before the drag. */
    onCancel(event: DragEvent, state: object): void;
}
/**
 * Adds the drag bindings a tiling layout offers to `bindings`: the swap binding for a `swap` trigger, and the resize
 * binding for a `resize` trigger. A `null` trigger leaves its binding out, and so does an absent `resize` (a layout
 * with no divider has none).
 */
export declare function addTilingBindings(bindings: BindingCollection, triggers: {
    readonly swap: string | null;
    readonly resize?: string | null;
}): void;
