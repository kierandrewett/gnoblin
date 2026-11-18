// The two drag bindings of the tiling layouts: `ClientSwapBinding` swaps a tiled window with the one under the
// pointer, and `SplitResizeBinding` moves the split between two tiles. The drags are the library's (`libgnoblin-std`, binding
// kinds `swap` and `split_resize`); these classes give them the windows and the dividers and write what they answer.

import { PointerDragBinding } from "gnoblin-core/bindings/binding";
import type { BindingCollection, PointerDragBindingOptions } from "gnoblin-core/bindings/binding";
import type { BindingClient, DragEvent, Point } from "gnoblin-core/bindings/types";
import { AxisKind, DragKind, OutcomeFlags, STD_SPLIT_CROSS } from "gnoblin-std/generated/constants";
import { SplitResizeBinding as NativeSplitResize, SwapBinding } from "gnoblin-std/generated/layouts";
import type { Split } from "gnoblin-std/generated/records";
import { has, idOf, takeStep, tileOf } from "gnoblin-std/bindings/step";

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
    readonly clients: Iterable<TiledClient> & { swap(a: TiledClient, b: TiledClient): void };
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
 * Asks the library which tile of `workspace`, other than `dragged`, the pointer in `event` holds, at the `phase` of a swap drag;
 * null when none does, or when the event has no position.
 */
function tileAt(
    phase: number,
    event: DragEvent,
    dragged: TiledClient,
    workspace: OrderedWorkspace,
): TiledClient | null {
    if (!event.position) {
        return null;
    }
    const byId = new Map<number, TiledClient>();
    for (const client of workspace.clients) {
        if (client.alive) {
            byId.set(idOf(client), client);
        }
    }
    const answer = takeStep(new SwapBinding(), phase, event, dragged, { tiles: [...byId.values()].map(tileOf) });
    return has(answer, OutcomeFlags.HAS_SWAP) ? (byId.get(answer.swapWith) ?? null) : null;
}

/**
 * Super+MouseLeft drag in a tiling layout: drop the tile you hold on another tile and the two trade places in the
 * workspace's order, so the layout puts each where the other was. Nothing moves while the pointer does: the order
 * changes once, on release, and the layout computes once. A release outside every tile, over the tile itself or
 * after a cancel leaves the order as it was. Only a tiled window takes part; a floating, fullscreen or maximised
 * one declines.
 */
export class ClientSwapBinding extends PointerDragBinding {
    constructor(options: Partial<PointerDragBindingOptions> = {}) {
        super({ ...options, id: options.id ?? "swap", trigger: options.trigger ?? "Super+MouseLeft+Drag" });
    }

    /** Starts a swap when the pressed client belongs to a tiled workspace. */
    onStart(event: DragEvent): SwapState | false {
        const client = event.client as TiledClient | undefined;
        if (!client?.workspace || !has(takeStep(new SwapBinding(), DragKind.START, event, client), OutcomeFlags.TOOK)) {
            return false;
        }
        return { dragged: client, workspace: client.workspace, over: null };
    }

    /** Tracks the tile currently under the pointer. */
    onUpdate(event: DragEvent, state: object): void {
        const swap = state as SwapState;
        swap.over = tileAt(DragKind.UPDATE, event, swap.dragged, swap.workspace);
    }

    /** Swaps the dragged client with the tile under the release point. */
    onEnd(event: DragEvent, state: object): void {
        const { dragged, workspace } = state as SwapState;
        // The release decides, not the last sample: the pointer may have moved since it.
        const over = tileAt(DragKind.END, event, dragged, workspace);
        if (over !== null && dragged.alive && dragged.workspace === workspace) {
            workspace.clients.swap(dragged, over);
        }
    }
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
    cross: { split: LayoutSplit; startRatio: number } | null;
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

/** What `SplitResizeBinding` reads of a layout: `findSplit`, which a layout with a divider has. */
interface SplitSource {
    findSplit?(client: TiledClient, point: Point): LayoutSplit | null;
}

/**
 * Super+MouseRight drag in a tiling layout: moves the divider nearest the press by as far as the pointer travels, so
 * the tiles on both sides follow it while the pointer is down. The layout decides which divider that is and how its
 * ratio is held (`Layout.findSplit`); a layout with no divider (a grid, a monocle) has none, and this declines. The
 * ratio is live, so a release keeps it and a cancel puts it back. A window that is not tiled, or is fullscreen or
 * maximised, declines. A layout may hand back a second divider on the other axis (`LayoutSplit.cross`); the same
 * drag then moves both, each by the pointer's travel along its own axis.
 */
export class SplitResizeBinding extends PointerDragBinding {
    constructor(options: Partial<PointerDragBindingOptions> = {}) {
        super({ ...options, id: options.id ?? "resize", trigger: options.trigger ?? "Super+MouseRight+Drag" });
    }

    /** Starts resizing the nearest divider when the layout provides one. */
    onStart(event: DragEvent): SplitState | false {
        const client = event.client as TiledClient | undefined;
        const { position } = event;
        const layout = event.layout as SplitSource | undefined;
        if (!client || !position) {
            return false;
        }
        const split = typeof layout?.findSplit === "function" ? layout.findSplit(client, position) : null;
        if (split === null || split === undefined) {
            return false;
        }
        const state = this.#hold(split, position);
        if (
            !has(
                takeStep(new NativeSplitResize(), DragKind.START, event, client, { split: splitOf(state) }),
                OutcomeFlags.TOOK,
            )
        ) {
            return false;
        }
        return state;
    }

    /** What the drag keeps from its start: the dividers and their ratios as the layout found them, and the press. */
    #hold(split: LayoutSplit, start: Point): SplitState {
        const { cross } = split;
        const second = cross !== null && cross !== undefined && cross.span > 0;
        return {
            split,
            start: { ...start },
            startRatio: split.ratio,
            cross: second ? { split: cross, startRatio: cross.ratio } : null,
        };
    }

    /** Updates the divider ratios from pointer travel. */
    onUpdate(event: DragEvent, state: object): void {
        const held = state as SplitState;
        if (!event.position) {
            return;
        }
        const answer = takeStep(new NativeSplitResize(), DragKind.UPDATE, event, undefined, {
            origin: held.start,
            split: splitOf(held),
        });
        if (has(answer, OutcomeFlags.HAS_RATIO)) {
            held.split.ratio = answer.ratio;
        }
        if (held.cross !== null && has(answer, OutcomeFlags.HAS_CROSS_RATIO)) {
            held.cross.split.ratio = answer.crossRatio;
        }
    }

    /** Restores the divider position from before the drag. */
    onCancel(event: DragEvent, state: object): void {
        const held = state as SplitState;
        const answer = takeStep(new NativeSplitResize(), DragKind.CANCEL, event, undefined, {
            origin: held.start,
            split: splitOf(held),
        });
        if (has(answer, OutcomeFlags.RESET_SPLIT)) {
            held.split.reset();
            held.cross?.split.reset();
        }
    }
}

/**
 * Adds the drag bindings a tiling layout offers to `bindings`: the swap binding for a `swap` trigger, and the resize
 * binding for a `resize` trigger. A `null` trigger leaves its binding out, and so does an absent `resize` (a layout
 * with no divider has none).
 */
export function addTilingBindings(
    bindings: BindingCollection,
    triggers: { readonly swap: string | null; readonly resize?: string | null },
): void {
    if (triggers.swap !== null) {
        bindings.add(new ClientSwapBinding({ id: "swap", trigger: triggers.swap }));
    }
    if (triggers.resize !== undefined && triggers.resize !== null) {
        bindings.add(new SplitResizeBinding({ id: "resize", trigger: triggers.resize }));
    }
}

/**
 * What the library is told of the divider a drag holds: its axis, its span, how far it may go, and the second one, with the ratios
 * they had when the drag started.
 */
function splitOf({ split, startRatio, cross }: SplitState): Split {
    return {
        flags: cross !== null ? STD_SPLIT_CROSS : 0,
        token: 0,
        axis: split.axis === "x" ? AxisKind.X : AxisKind.Y,
        direction: split.direction,
        span: split.span,
        min: split.min,
        max: split.max,
        ratio: startRatio,
        crossAxis: cross?.split.axis === "y" ? AxisKind.Y : AxisKind.X,
        crossDirection: cross?.split.direction ?? 1,
        crossSpan: cross?.split.span ?? 0,
        crossMin: cross?.split.min ?? 0,
        crossMax: cross?.split.max ?? 0,
        crossRatio: cross?.startRatio ?? 0,
    };
}
