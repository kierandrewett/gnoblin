// `ClientResizeBinding`: drag the edge or corner of a window nearest the pointer to resize it. The drag is the library's resize
// (`libgnoblin-std`, binding kind `resize`); this class gives it the window and writes what it answers.

import { PointerDragBinding } from "gnoblin-core/bindings/binding";
import type { PointerDragBindingOptions } from "gnoblin-core/bindings/binding";
import type { BindingClient, DragEvent, Point, Rect } from "gnoblin-core/bindings/types";
import { Drag } from "gnoblin-std/generated/classes";
import { ClientFlags, DragKind, OutcomeFlags, SideFlags } from "gnoblin-std/generated/constants";
import { ResizeBinding, ResizeGrip } from "gnoblin-std/generated/layouts";
import { has, rectOf, takeStep } from "gnoblin-std/bindings/step";

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

/** What `edges` says, in the library's words. */
const GRIPS: Readonly<Record<string, ResizeGrip>> = {
    quadrant: ResizeGrip.QUADRANT,
    nearest: ResizeGrip.NEAREST,
    "top-left": ResizeGrip.TOP_LEFT,
    "top-right": ResizeGrip.TOP_RIGHT,
    "bottom-left": ResizeGrip.BOTTOM_LEFT,
    "bottom-right": ResizeGrip.BOTTOM_RIGHT,
};

/** The sides of the library's `SideFlags` that the name of a corner or an edge moves. */
function sidesOf(corner: ResizeCorner | ResizeEdge): number {
    let sides = 0;
    if (corner.endsWith("left")) {
        sides |= SideFlags.LEFT;
    } else if (corner.endsWith("right")) {
        sides |= SideFlags.RIGHT;
    }
    if (corner.startsWith("top")) {
        sides |= SideFlags.TOP;
    } else if (corner.startsWith("bottom")) {
        sides |= SideFlags.BOTTOM;
    }
    return sides;
}

/** The name of the corner or the edge that `SideFlags` flags are. */
const NAMES: Readonly<Record<number, ResizeCorner | ResizeEdge>> = {
    [SideFlags.LEFT | SideFlags.TOP]: "top-left",
    [SideFlags.RIGHT | SideFlags.TOP]: "top-right",
    [SideFlags.LEFT | SideFlags.BOTTOM]: "bottom-left",
    [SideFlags.RIGHT | SideFlags.BOTTOM]: "bottom-right",
    [SideFlags.LEFT]: "left",
    [SideFlags.RIGHT]: "right",
    [SideFlags.TOP]: "top",
    [SideFlags.BOTTOM]: "bottom",
};

/**
 * The rectangle `start` becomes when the corner or side `corner` of it moves by `delta`. The opposite corner is the
 * anchor: a rectangle held at a limit stops there and does not slide. An axis no side moves on keeps the size it had,
 * whatever the limits say. The smallest size is `min`, or 100 by 80; `max` is none when absent. This is the arithmetic
 * of `ClientResizeBinding` and of a frame's `ResizeHandle`, and the library's.
 */
export function resizedRect(
    start: Rect,
    corner: ResizeCorner | ResizeEdge,
    delta: Point,
    min?: { width: number; height: number },
    max?: { width: number; height: number },
): Rect {
    return Drag.resize(start, sidesOf(corner), delta.x, delta.y, {
        flags: (min === undefined ? 0 : ClientFlags.HAS_MIN) | (max === undefined ? 0 : ClientFlags.HAS_MAX),
        minWidth: min?.width ?? 0,
        minHeight: min?.height ?? 0,
        maxWidth: max?.width ?? 0,
        maxHeight: max?.height ?? 0,
    });
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
        if (!Object.hasOwn(GRIPS, edges)) {
            throw new TypeError(
                `ClientResizeBinding edges must be quadrant, nearest or one of ${Object.keys(GRIPS).slice(2).join(", ")}, got ${String(edges)}`,
            );
        }
        this.edges = edges;
    }

    /** The library's resize, with the grip this binding has now. */
    #rule(): ResizeBinding {
        return new ResizeBinding({ grip: GRIPS[this.edges] });
    }

    /** Starts a resize for a movable client and records the selected corner or side. */
    onStart(event: DragEvent): ResizeState | false {
        const { client, position } = event;
        if (!client || !position) {
            return false;
        }
        const answer = takeStep(this.#rule(), DragKind.START, event, client);
        if (!has(answer, OutcomeFlags.TOOK)) {
            return false;
        }
        return { client, start: rectOf(client.bounds), pointer: { ...position }, corner: NAMES[answer.sides] };
    }

    /** Resizes the client using pointer travel from the start. */
    onUpdate(event: DragEvent, state: object): void {
        const { client, start, pointer, corner } = state as ResizeState;
        if (!client.alive || !event.position) {
            return;
        }
        const answer = takeStep(this.#rule(), DragKind.UPDATE, event, client, {
            origin: pointer,
            start,
            sides: sidesOf(corner),
        });
        if (has(answer, OutcomeFlags.HAS_BOUNDS)) {
            client.bounds = rectOf(answer.bounds);
        }
    }

    /** Saves the final bounds as the client's floating bounds. */
    onEnd(event: DragEvent, state: object): void {
        const { client, start, pointer, corner } = state as ResizeState;
        if (!client.alive) {
            return;
        }
        const answer = takeStep(this.#rule(), DragKind.END, event, client, {
            origin: pointer,
            start,
            sides: sidesOf(corner),
        });
        if (has(answer, OutcomeFlags.HAS_FLOATING)) {
            client.floatingBounds = rectOf(answer.floating);
        }
    }

    /** Restores the original bounds when the drag is cancelled. */
    onCancel(event: DragEvent, state: object): void {
        const { client, start, pointer, corner } = state as ResizeState;
        if (!client.alive) {
            return;
        }
        const answer = takeStep(this.#rule(), DragKind.CANCEL, event, client, {
            origin: pointer,
            start,
            sides: sidesOf(corner),
        });
        if (has(answer, OutcomeFlags.HAS_BOUNDS)) {
            client.bounds = rectOf(answer.bounds);
        }
    }
}
