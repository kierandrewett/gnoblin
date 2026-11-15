// `ClientResizeBinding`: drag the edge or corner of a window nearest the pointer to resize it.

import { PointerDragBinding } from "gnoblin/bindings/binding";
import type { PointerDragBindingOptions } from "gnoblin/bindings/binding";
import { draggable, moved } from "gnoblin/bindings/drag";
import type { BindingClient, DragEvent, Point, Rect } from "gnoblin/bindings/types";

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
    client: BindingClient;
    start: Rect;
    pointer: Point;
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
 * Super+MouseRight drag: the corner nearest the pointer at the press moves with it and the opposite corner
 * stays. The corner is chosen from the initial position only, so crossing the client's middle mid-drag does
 * not switch corners.
 */
export class ClientResizeBinding extends PointerDragBinding {
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

    onUpdate(event: DragEvent, state: object): void {
        const { client, start, pointer, corner } = state as ResizeState;
        if (!client.alive || !event.position) {
            return;
        }
        const delta = moved(pointer, event.position);
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
        const min = client.minSize ?? MIN_SIZE;
        const max = client.maxSize;
        // The opposite corner is the anchor: a client held at its limit stops there and does not slide.
        // An axis no side moves on keeps the size it had, whatever the limits say.
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
        client.bounds = r;
    }

    onEnd(_event: DragEvent, state: object): void {
        const { client } = state as ResizeState;
        if (client.alive) {
            client.floatingBounds = { ...client.bounds };
        }
    }

    onCancel(_event: DragEvent, state: object): void {
        const { client, start } = state as ResizeState;
        if (client.alive) {
            client.bounds = { ...start };
        }
    }
}
