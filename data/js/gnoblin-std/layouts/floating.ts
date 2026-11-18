// FloatingLayout places a window once when it joins a workspace; `compute` does not reposition it, so the user can
// move and resize it freely. The free-space, centred and cascading placement, and the separation of windows with
// coincident top-left corners, are the library's (`libgnoblin-std`, `floating_place`); this class reads what the library needs of
// the clients and writes the rectangle it answers.
//
// The default `move` and `resize` bindings let users drag windows. `triggers` replaces a binding trigger, and `null`
// leaves that binding out; bindings become active when an input context attaches the layout (`attachLayout`).

import { resolveTriggers } from "gnoblin-core/bindings/binding";
import { ClientMoveBinding } from "gnoblin-std/bindings/move";
import { ClientResizeBinding } from "gnoblin-std/bindings/resize";
import { growRect, NO_INSET, shrinkRect } from "gnoblin-core/inset";
import type { FrameInset } from "gnoblin-core/inset";
import { Floating } from "gnoblin-std/generated/classes";
import { PlaceKind, STD_FLOAT_APART } from "gnoblin-std/generated/constants";
import { Layout } from "gnoblin-std/layout";
import type { LayoutContext } from "gnoblin-std/layout";
import type { Rect } from "gnoblin-core/types";

/** `smart` is the first spot in the area that overlaps nothing; `center` and `cascade` are as named. */
export type FloatingPlacement = "smart" | "center" | "cascade";

/** The internal placement mode `placeRect` takes, one per `FloatingPlacement`. */
export type PlaceMode = "free_space" | "center" | "cascade";

const MODES: Readonly<Record<FloatingPlacement, PlaceMode>> = {
    smart: "free_space",
    center: "center",
    cascade: "cascade",
};

/** The size a window opens at before it has committed one. */
const DEFAULT_WIDTH = 800;
const DEFAULT_HEIGHT = 600;

/** Options of `FloatingLayout`. */
export interface FloatingOptions {
    /** Placement strategy for new windows. Default `smart`. */
    placement?: FloatingPlacement;
    /** Distance between cascade positions, in logical pixels. Default 32. */
    cascadeStep?: number;
    /** `undefined` keeps the default trigger, a string replaces it and `null` leaves the binding out. */
    triggers?: { move?: string | null; resize?: string | null };
}

/**
 * The part of a client the placement reads and writes. `floatingBounds` is set by the host once it has a position the
 * user chose.
 */
export interface FloatingClient {
    /** Current content bounds, in logical pixels. */
    bounds: Rect;
    /** Client-committed content size, in logical pixels, when available. */
    size?: { width: number; height: number };
    /** Host-set bounds for a window the user has already positioned. */
    floatingBounds?: Rect | null;
    /** Whether this window is included when choosing a placement. */
    visible?: boolean;
    /** Whether the window currently uses tiled placement. */
    tiled?: boolean;
    /**
     * The room the window's frame takes round the content. A window is placed as a frame: the content with this much
     * on each side.
     */
    readonly frameInset?: FrameInset;
}

const DEFAULT_TRIGGERS = { move: "Super+MouseLeft+Drag", resize: "Super+MouseRight+Drag" } as const;

/**
 * A layout that does not tile. Each window is placed once when it first appears; the user then moves and resizes it.
 */
export class FloatingLayout extends Layout {
    /** Strategy used to place new windows. Default `smart`. */
    placement: FloatingPlacement;
    /** Distance between cascade positions, in logical pixels. Default 32. */
    cascadeStep: number;
    /** Resolved `move` and `resize` triggers; `null` means that binding is disabled. */
    readonly triggers: Readonly<Record<"move" | "resize", string | null>>;
    // This layout may receive client objects it does not own, so it tracks placements separately. The weak set also
    // releases each client automatically when the client is no longer referenced.
    private readonly placed = new WeakSet<object>();

    constructor(options: FloatingOptions = {}) {
        super();
        this.placement = options.placement ?? "smart";
        this.cascadeStep = options.cascadeStep ?? 32;
        this.triggers = Object.freeze(resolveTriggers(DEFAULT_TRIGGERS, options.triggers));
        if (this.triggers.move !== null) {
            this.bindings.add(new ClientMoveBinding({ id: "move", trigger: this.triggers.move }));
        }
        if (this.triggers.resize !== null) {
            this.bindings.add(
                new ClientResizeBinding({ id: "resize", trigger: this.triggers.resize, edges: "quadrant" }),
            );
        }
    }

    /** The rectangle for a new window of `width` by `height` in `area`, avoiding `occupied` where possible. */
    place(width: number, height: number, area: Rect, occupied: readonly Rect[]): Rect {
        return this.#ask(width, height, area, occupied, false);
    }

    /** The library's placement with this layout's options, checked as they always were. */
    #ask(width: number, height: number, area: Rect, occupied: readonly Rect[], apart: boolean): Rect {
        const mode = MODES[this.placement];
        if (mode === undefined) {
            throw new TypeError(
                `FloatingLayout placement must be smart, center or cascade, got ${String(this.placement)}`,
            );
        }
        if (!Number.isFinite(this.cascadeStep) || this.cascadeStep < 1) {
            throw new RangeError("FloatingLayout cascadeStep must be a finite number of at least 1");
        }
        return ask(mode, width, height, area, occupied, this.cascadeStep, apart);
    }

    /** Places a newly added client once unless it already has floating bounds or was placed by this layout. */
    onClientAdded(client: FloatingClient, context: LayoutContext): void {
        client.tiled = false; // a window that came from a tiled layout is floating now, placed or not
        if (client.floatingBounds || this.placed.has(client)) {
            return;
        }
        // Until the shell writes `bounds`, they hold the size the client committed.
        const size = client.size ?? client.bounds;
        const width = size.width > 0 ? size.width : DEFAULT_WIDTH;
        const height = size.height > 0 ? size.height : DEFAULT_HEIGHT;
        const inset = client.frameInset ?? NO_INSET;
        const occupied: Rect[] = [];
        for (const other of context.children as readonly FloatingClient[]) {
            if (other !== client && other.visible !== false) {
                // Another window takes its frame's room as well as its content's.
                occupied.push(growRect(other.bounds, other.frameInset ?? NO_INSET));
            }
        }
        const room = growRect({ x: 0, y: 0, width, height }, inset);
        const frame = this.#ask(room.width, room.height, context.area, occupied, true);
        client.bounds = shrinkRect(frame, inset);
        this.placed.add(client);
    }
}

const KINDS: Readonly<Record<PlaceMode, number>> = {
    free_space: PlaceKind.SMART,
    center: PlaceKind.CENTER,
    cascade: PlaceKind.CASCADE,
};

function ask(
    mode: PlaceMode,
    width: number,
    height: number,
    area: Rect,
    occupied: readonly Rect[],
    cascadeStep: number,
    apart: boolean,
): Rect {
    const kind = KINDS[mode];
    if (kind === undefined) {
        throw new TypeError(`unknown placement mode ${String(mode)}`);
    }
    return Floating.place({
        flags: apart ? STD_FLOAT_APART : 0,
        mode: kind,
        step: cascadeStep,
        width,
        height,
        area,
        occupied: [...occupied],
    });
}

/**
 * The rectangle for a new window of `width` by `height` in `area`, avoiding `occupied` where possible. A size
 * below 1 on an axis is raised to 1, so the window covers something. The placement is the library's.
 */
export function placeRect(
    mode: PlaceMode,
    width: number,
    height: number,
    area: Rect,
    occupied: readonly Rect[],
    cascadeStep = 32,
): Rect {
    return ask(mode, width, height, area, occupied, cascadeStep, false);
}
