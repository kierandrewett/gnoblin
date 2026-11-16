// FloatingLayout places a window once when it joins a workspace; `compute` does not reposition it, so the user can
// move and resize it freely. `placeRect` implements free-space, centred and cascading placement. The tests in
// `tests/shell/placement_cases.toml` cover its placement results. `onClientAdded` separates windows with coincident
// top-left corners.
//
// The default `move` and `resize` bindings let users drag windows. `triggers` replaces a binding trigger, and `null`
// leaves that binding out; bindings become active when an input context attaches the layout (`attachLayout`).

import { resolveTriggers } from "gnoblin-core/bindings/binding";
import { ClientMoveBinding } from "gnoblin-std/bindings/move";
import { ClientResizeBinding } from "gnoblin-std/bindings/resize";
import { growRect, NO_INSET, shrinkRect } from "gnoblin-core/inset";
import type { FrameInset } from "gnoblin-core/inset";
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

const SCAN_STEP = 16;
const MAX_SCAN_STEPS = 20000;
const MAX_CASCADE_STEPS = 32;
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
        const mode = MODES[this.placement];
        if (mode === undefined) {
            throw new TypeError(
                `FloatingLayout placement must be smart, center or cascade, got ${String(this.placement)}`,
            );
        }
        if (!Number.isFinite(this.cascadeStep) || this.cascadeStep < 1) {
            throw new RangeError("FloatingLayout cascadeStep must be a finite number of at least 1");
        }
        return placeRect(mode, width, height, area, occupied, this.cascadeStep);
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
        const spot = this.place(room.width, room.height, context.area, occupied);
        const frame = apartFrom(spot, occupied, context.area, this.cascadeStep);
        client.bounds = shrinkRect(frame, inset);
        this.placed.add(client);
    }
}

function intersects(a: Rect, b: Rect): boolean {
    // An empty rectangle overlaps nothing.
    return (
        a.width > 0 &&
        a.height > 0 &&
        b.width > 0 &&
        b.height > 0 &&
        a.x < b.x + b.width &&
        b.x < a.x + a.width &&
        a.y < b.y + b.height &&
        b.y < a.y + a.height
    );
}

function hits(rect: Rect, occupied: readonly Rect[]): boolean {
    return occupied.some((other) => intersects(rect, other));
}

/** Moves `r` the least distance inside `area`. A rectangle at least as large as the area goes to its origin. */
function clampInside(r: Rect, area: Rect): Rect {
    const x = r.width >= area.width ? area.x : Math.min(Math.max(r.x, area.x), area.x + area.width - r.width);
    const y = r.height >= area.height ? area.y : Math.min(Math.max(r.y, area.y), area.y + area.height - r.height);
    return { x, y, width: r.width, height: r.height };
}

/**
 * Moves `frame` back toward the area's origin, one `step` at a time, until no window already has its top-left corner.
 * `placeRect` pushes a cascade that has run out of room back inside the area, which pins every later window to the
 * area's far corner: the second and the third of three large windows would land exactly on each other, and the third
 * would hide the second. The golden table pins `placeRect`, so this runs after it. A frame that cannot move any
 * further (the area's origin) stays.
 */
function apartFrom(frame: Rect, occupied: readonly Rect[], area: Rect, step: number): Rect {
    let c = frame;
    for (let i = 0; i < MAX_CASCADE_STEPS && occupied.some((other) => other.x === c.x && other.y === c.y); i++) {
        const moved = clampInside({ x: c.x - step, y: c.y - step, width: c.width, height: c.height }, area);
        if (moved.x === c.x && moved.y === c.y) {
            break;
        }
        c = moved;
    }
    return c;
}

/** Steps diagonally from `origin` until the window overlaps nothing, or the bound is hit. */
function cascade(origin: Rect, area: Rect, occupied: readonly Rect[], step: number): Rect {
    let c = clampInside(origin, area);
    for (let i = 1; i <= MAX_CASCADE_STEPS; i++) {
        if (!hits(c, occupied)) {
            break;
        }
        const d = step * i;
        c = clampInside({ x: origin.x + d, y: origin.y + d, width: origin.width, height: origin.height }, area);
    }
    return c;
}

/** The first grid spot in the area that overlaps nothing; cascades from the top-left when none fits. */
function freeSpace(width: number, height: number, area: Rect, occupied: readonly Rect[], step: number): Rect {
    let steps = 0;
    for (let y = area.y; y + height <= area.y + area.height && steps < MAX_SCAN_STEPS; y += SCAN_STEP) {
        for (let x = area.x; x + width <= area.x + area.width && steps < MAX_SCAN_STEPS; x += SCAN_STEP) {
            const c = { x, y, width, height };
            if (!hits(c, occupied)) {
                return c;
            }
            steps++;
        }
    }
    return cascade({ x: area.x, y: area.y, width, height }, area, occupied, step);
}

/** Centred; cascades from the centre only when that spot is exactly taken. */
function centred(width: number, height: number, area: Rect, occupied: readonly Rect[], step: number): Rect {
    const c = clampInside(
        {
            x: area.x + Math.floor(Math.max(0, area.width - width) / 2),
            y: area.y + Math.floor(Math.max(0, area.height - height) / 2),
            width,
            height,
        },
        area,
    );
    const taken = occupied.some((o) => o.x === c.x && o.y === c.y && o.width === c.width && o.height === c.height);
    return taken ? cascade(c, area, occupied, step) : c;
}

/**
 * The rectangle for a new window of `width` by `height` in `area`, avoiding `occupied` where possible. A size
 * below 1 on an axis is raised to 1, so the window covers something.
 */
export function placeRect(
    mode: PlaceMode,
    width: number,
    height: number,
    area: Rect,
    occupied: readonly Rect[],
    cascadeStep = 32,
): Rect {
    const w = Math.max(1, width);
    const h = Math.max(1, height);
    switch (mode) {
        case "free_space":
            return freeSpace(w, h, area, occupied, cascadeStep);
        case "center":
            return centred(w, h, area, occupied, cascadeStep);
        case "cascade":
            return cascade({ x: area.x, y: area.y, width: w, height: h }, area, occupied, cascadeStep);
        default:
            throw new TypeError(`unknown placement mode ${String(mode)}`);
    }
}
