// What every pointer drag shares: how a move or resize is held inside the area a `drag_constraint` allows. The arithmetic is the
// library's (`libgnoblin-std`, `drag_clamp`); this module is the spelling a script knows it by.

import type { Rect } from "gnoblin-core/bindings/types";
import { Drag } from "gnoblin-std/generated/classes";
import { ConstraintKind, DragKind } from "gnoblin-std/generated/constants";

/** How a dragged window is kept in the work area: inside it, or with a strip of it inside. */
export type DragConstraint = "work_area" | "minimum_strip";

/** The visible strip of a window kept inside the work area under `minimum_strip`. */
export const DRAG_STRIP = DragKind.STRIP;

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
    const constraint = mode === "minimum_strip" ? ConstraintKind.MINIMUM_STRIP : ConstraintKind.WORK_AREA;
    return Drag.clamp(pos, size, start, span, constraint);
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
