import { PointerDragBinding } from "gnoblin-core/bindings/binding";
import type { PointerDragBindingOptions } from "gnoblin-core/bindings/binding";
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
/**
 * The rectangle `start` becomes when the corner or side `corner` of it moves by `delta`. The opposite corner is the
 * anchor: a rectangle held at a limit stops there and does not slide. An axis no side moves on keeps the size it had,
 * whatever the limits say. The smallest size is `min`, or 100 by 80; `max` is none when absent. This is the arithmetic
 * of `ClientResizeBinding` and of a frame's `ResizeHandle`, and the library's.
 */
export declare function resizedRect(start: Rect, corner: ResizeCorner | ResizeEdge, delta: Point, min?: {
    width: number;
    height: number;
}, max?: {
    width: number;
    height: number;
}): Rect;
/**
 * Super+MouseRight drag: the corner nearest the pointer at the press moves with it and the opposite corner
 * stays. The corner is chosen from the initial position only, so crossing the client's middle mid-drag does
 * not switch corners.
 */
export declare class ClientResizeBinding extends PointerDragBinding {
    #private;
    /** Chooses the moving corner or side at drag start. Defaults to `quadrant`. */
    edges: "quadrant" | "nearest" | ResizeCorner;
    constructor(options?: ClientResizeOptions);
    /** Starts a resize for a movable client and records the selected corner or side. */
    onStart(event: DragEvent): ResizeState | false;
    /** Resizes the client using pointer travel from the start. */
    onUpdate(event: DragEvent, state: object): void;
    /** Saves the final bounds as the client's floating bounds. */
    onEnd(event: DragEvent, state: object): void;
    /** Restores the original bounds when the drag is cancelled. */
    onCancel(event: DragEvent, state: object): void;
}
