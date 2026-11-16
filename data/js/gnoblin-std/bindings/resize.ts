// `ClientResizeBinding`: drag the edge or corner of a window nearest the pointer to resize it.

import { PointerDragBinding } from "gnoblin-core/bindings/binding";
import type { PointerDragBindingOptions } from "gnoblin-core/bindings/binding";
import { draggable, moved } from "gnoblin-std/bindings/drag";
import type { BindingClient, DragEvent, Point, Rect } from "gnoblin-core/bindings/types";

/** Where a resize drag holds the client: the corner that moves with the pointer. */
export type ResizeCorner = "top-left" | "top-right" | "bottom-left" | "bottom-right";

/** The one side that moves with the pointer when `edges` is `nearest`. */
export type ResizeEdge = "left" | "right" | "top" | "bottom";

/** Options of `ClientResizeBinding`. */
export interface ClientResizeOptions extends Partial<PointerDragBindingOptions> {
    /**
     * `quadrant` (default) picks the corner nearest the pointer at the press and moves two sides; `nearest` picks the
     * side nearest the pointer and moves that one alone; or name the corner.
     */
    edges?: "quadrant" | "nearest" | ResizeCorner;
}

/**
 * What `ClientResizeBinding.onStart` keeps for one drag. `corner`, a corner or one side, is fixed for the whole drag.
 */
export interface ResizeState {
    /** Client being resized. */
    client: BindingClient;
    /** Client bounds when the drag started, in logical pixels. */
    start: Rect;
    /** Pointer position when the drag started, in logical pixels. */
    pointer: Point;
    /** Corner or side that follows the pointer for the whole drag. */
    corner: ResizeCorner | ResizeEdge;
}

const CORNERS: readonly string[] = ["top-left", "top-right", "bottom-left", "bottom-right"];

/** The side of a `width` by `height` client that a press at `where` is nearest to. A tie goes to left, then top. */
function nearestEdge(where: Point, width: number, height: number): ResizeEdge {
    const gaps: [ResizeEdge, number][] = [
        ["left", where.x],
        ["top", where.y],
        ["right", width - where.x],
        ["bottom", height - where.y],
    ];
    return gaps.reduce((best, gap) => (gap[1] < best[1] ? gap : best))[0];
}

/** The fewest pixels a client may be resized to when it states no minimum of its own. */
const MIN_SIZE = { width: 100, height: 80 };

/**
 * The rectangle `start` becomes when the corner or side `corner` of it moves by `delta`. The opposite corner is the
 * anchor: a rectangle held at a limit stops there and does not slide. An axis no side moves on keeps the size it had,
 * whatever the limits say. The smallest size is `min`, or `MIN_SIZE`; `max` is none when absent. This is the arithmetic
 * of `ClientResizeBinding` and of a frame's `ResizeHandle`.
 */
export function resizedRect(
    start: Rect,
    corner: ResizeCorner | ResizeEdge,
    delta: Point,
    min: { width: number; height: number } = MIN_SIZE,
    max?: { width: number; height: number },
): Rect {
    const r = { ...start };
    // The sides that move with the pointer: a corner moves two, an edge moves one.
    const left = corner.endsWith("left");
    const right = corner.endsWith("right");
    const top = corner.startsWith("top");
    const bottom = corner.startsWith("bottom");
    if (left) {
        r.x += delta.x;
        r.width -= delta.x;
    } else if (right) {
        r.width += delta.x;
    }
    if (top) {
        r.y += delta.y;
        r.height -= delta.y;
    } else if (bottom) {
        r.height += delta.y;
    }
    if (left || right) {
        const limitedWidth = Math.max(min.width, Math.min(r.width, max?.width ?? Infinity));
        if (left) {
            r.x += r.width - limitedWidth;
        }
        r.width = limitedWidth;
    }
    if (top || bottom) {
        const limitedHeight = Math.max(min.height, Math.min(r.height, max?.height ?? Infinity));
        if (top) {
            r.y += r.height - limitedHeight;
        }
        r.height = limitedHeight;
    }
    return r;
}

/**
 * Super+MouseRight drag: the corner nearest the pointer at the press moves with it and the opposite corner
 * stays. The corner is chosen from the initial position only, so crossing the client's middle mid-drag does
 * not switch corners.
 */
export class ClientResizeBinding extends PointerDragBinding {
    /** Chooses the moving corner or side at drag start. Defaults to `quadrant`. */
    edges: "quadrant" | "nearest" | ResizeCorner;

    constructor(options: ClientResizeOptions = {}) {
        super({ ...options, id: options.id ?? "resize", trigger: options.trigger ?? "Super+MouseRight+Drag" });
        const edges = options.edges ?? "quadrant";
        if (edges !== "quadrant" && edges !== "nearest" && !CORNERS.includes(edges)) {
            throw new TypeError(
                `ClientResizeBinding edges must be quadrant, nearest or one of ${CORNERS.join(", ")}, got ${String(edges)}`,
            );
        }
        this.edges = edges;
    }

    /** Starts a resize for a movable client and records the selected corner or side. */
    onStart(event: DragEvent): ResizeState | false {
        const { client, position } = event;
        if (!client || !position || !draggable(client)) {
            return false;
        }
        const start = { ...client.bounds };
        const where = event.localPosition ?? { x: position.x - start.x, y: position.y - start.y };
        let corner: ResizeCorner | ResizeEdge;
        if (this.edges === "quadrant") {
            corner = `${where.y < start.height / 2 ? "top" : "bottom"}-${where.x < start.width / 2 ? "left" : "right"}`;
        } else if (this.edges === "nearest") {
            corner = nearestEdge(where, start.width, start.height);
        } else {
            corner = this.edges;
        }
        return { client, start, pointer: { ...position }, corner };
    }

    /** Resizes the client using pointer travel from the start. */
    onUpdate(event: DragEvent, state: object): void {
        const { client, start, pointer, corner } = state as ResizeState;
        if (!client.alive || !event.position) {
            return;
        }
        client.bounds = resizedRect(start, corner, moved(pointer, event.position), client.minSize, client.maxSize);
    }

    /** Saves the final bounds as the client's floating bounds. */
    onEnd(_event: DragEvent, state: object): void {
        const { client } = state as ResizeState;
        if (client.alive) {
            client.floatingBounds = { ...client.bounds };
        }
    }

    /** Restores the original bounds when the drag is cancelled. */
    onCancel(_event: DragEvent, state: object): void {
        const { client, start } = state as ResizeState;
        if (client.alive) {
            client.bounds = { ...start };
        }
    }
}
