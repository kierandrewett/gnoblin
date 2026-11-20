import type { Rect } from "gnoblin-core/bindings/types";
/** How a dragged window is kept in the work area: inside it, or with a strip of it inside. */
export type DragConstraint = "work_area" | "minimum_strip";
/** The visible strip of a window kept inside the work area under `minimum_strip`. */
export declare const DRAG_STRIP: 75;
/** The constraint a seat's `drag_constraint` names. Absent is `work_area`; any other value throws. */
export declare function dragConstraintMode(value: unknown): DragConstraint;
/** One axis: a window of extent `size` at `pos`, against an area that starts at `start` and is `span` long. */
export declare function clampAxis(pos: number, size: number, start: number, span: number, mode: DragConstraint): number;
/** `rect` moved to the place `mode` allows inside `area`, keeping its size. */
export declare function constrainRect(rect: Rect, area: Rect, mode: DragConstraint): Rect;
