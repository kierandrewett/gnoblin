// The rules every pointer drag shares: how far the pointer has moved since the drag began, which clients may be
// dragged at all, and how a move or resize is held inside the area a `drag_constraint` allows.

import type { BindingClient, Point, Rect } from "gnoblin/bindings/types";

/** How far the pointer has moved since `start`. */
export function moved(start: Point, now: Point): Point {
    return { x: now.x - start.x, y: now.y - start.y };
}

/** How a dragged window is kept in the work area: inside it, or with a strip of it inside. */
export type DragConstraint = "work_area" | "minimum_strip";

/** The visible strip of a window kept inside the work area under `minimum_strip`. */
export const DRAG_STRIP = 75;

/** The constraint a seat's `drag_constraint` names. Absent is `work_area`; any other value throws. */
export function dragConstraintMode(value: unknown): DragConstraint {
    if (value === undefined || value === null || value === "work_area") {
        return "work_area";
    }
    if (value === "minimum_strip") {
        return "minimum_strip";
    }
    throw new TypeError(`drag_constraint must be work_area or minimum_strip, got ${String(value)}`);
}

/** One axis: a window of extent `size` at `pos`, against an area that starts at `start` and is `span` long. */
export function clampAxis(pos: number, size: number, start: number, span: number, mode: DragConstraint): number {
    if (mode === "work_area") {
        if (size >= span) {
            return start;
        }
        return Math.min(Math.max(pos, start), start + span - size);
    }
    const visible = Math.min(DRAG_STRIP, size);
    const low = start - (size - visible);
    const high = start + span - visible;
    return Math.min(Math.max(pos, low), high);
}

/** `rect` moved to the place `mode` allows inside `area`, keeping its size. */
export function constrainRect(rect: Rect, area: Rect, mode: DragConstraint): Rect {
    return {
        x: clampAxis(rect.x, rect.width, area.x, area.width, mode),
        y: clampAxis(rect.y, rect.height, area.y, area.height, mode),
        width: rect.width,
        height: rect.height,
    };
}

/** Whether a window may be dragged: a fullscreen or maximised one declines. */
export function draggable(client: BindingClient): boolean {
    return !client.fullscreen && client.maximised !== true;
}
