// `ClientMoveBinding`: drag a window by the pointer, with the edge snapping of `gnoblin-std/snap`. The drag is the library's move
// (`libgnoblin-std`, binding kind `move`); this class gives it the window and writes what it answers.

import { PointerDragBinding } from "gnoblin-core/bindings/binding";
import type { PointerDragBindingOptions } from "gnoblin-core/bindings/binding";
import type { BindingClient, DragEvent, Point, Rect } from "gnoblin-core/bindings/types";
import { NO_INSET } from "gnoblin-core/inset";
import { DragKind, OutcomeFlags, SnapKind } from "gnoblin-std/generated/constants";
import { MoveBinding } from "gnoblin-std/generated/layouts";
import { has, rectOf, takeStep } from "gnoblin-std/bindings/step";
import { forgetSnap, SNAP_MARGIN, snapClient, zoneNamed } from "gnoblin-std/snap";
import type { SnapClient } from "gnoblin-std/snap";

/** What `ClientMoveBinding.onStart` keeps for one drag: where the press and the window were. */
export interface MoveState {
    /** Client whose bounds are being moved. */
    client: BindingClient;
    /** Pointer position when the drag started, in logical pixels. */
    startPointer: Point;
    /** Client bounds when the drag started, in logical pixels. */
    startBounds: Rect;
}

/** Options of `ClientMoveBinding`. */
export interface ClientMoveOptions extends Partial<PointerDragBindingOptions> {
    /**
     * Edge tiling on release: how near a screen edge, in pixels, the pointer must be for the drop to tile the window
     * (left and right take half the work area, top all of it). Default `SNAP_MARGIN`. `false` never tiles. Only when
     * the seat's `edge_tiling` is on, as in the Lua shell; off, a drop is just a move.
     */
    snap?: number | false;
}

/**
 * Super+MouseLeft drag: the client follows the pointer by the distance it has moved since the press. Written
 * against the client's `bounds` and the event's `position` alone; there is no native grab behind it. A drop within
 * `snap` pixels of a screen edge tiles the window when edge tiling is on (`gnoblin-std/snap`).
 */
export class ClientMoveBinding extends PointerDragBinding {
    /** Edge distance in logical pixels for tiling on release, or false to disable tiling. */
    snap: number | false;

    constructor(options: ClientMoveOptions = {}) {
        super({ ...options, id: options.id ?? "move", trigger: options.trigger ?? "Super+MouseLeft+Drag" });
        const snap = options.snap ?? SNAP_MARGIN;
        if (snap !== false && (typeof snap !== "number" || !Number.isFinite(snap) || snap < 0)) {
            throw new TypeError(`ClientMoveBinding snap must be false or a distance of 0 or more, got ${String(snap)}`);
        }
        this.snap = snap;
    }

    /** The library's move, with the edge distance this binding has now. */
    #rule(): MoveBinding {
        return new MoveBinding(this.snap === false ? { snap: false } : { snap: true, snapMargin: this.snap });
    }

    /** Starts a move for a movable client and records its starting position. */
    onStart(event: DragEvent): MoveState | false {
        const { client, position } = event;
        if (!client || !position) {
            return false;
        }
        if (!has(takeStep(this.#rule(), DragKind.START, event, client), OutcomeFlags.TOOK)) {
            return false;
        }
        // A snapped window the user takes hold of is no longer tiled to its zone.
        forgetSnap(client);
        return { client, startPointer: { ...position }, startBounds: rectOf(client.bounds) };
    }

    /** Moves the client by the pointer distance from the start. */
    onUpdate(event: DragEvent, state: object): void {
        const { client, startPointer, startBounds } = state as MoveState;
        if (!client.alive || !event.position) {
            return;
        }
        const answer = takeStep(this.#rule(), DragKind.UPDATE, event, client, {
            origin: startPointer,
            start: startBounds,
        });
        if (has(answer, OutcomeFlags.HAS_BOUNDS)) {
            client.bounds = rectOf(answer.bounds);
        }
    }

    /** Saves the moved bounds and tiles on release when edge tiling is enabled. */
    onEnd(event: DragEvent, state: object): void {
        const { client, startPointer, startBounds } = state as MoveState;
        if (!client.alive) {
            return;
        }
        const answer = takeStep(this.#rule(), DragKind.END, event, client, {
            origin: startPointer,
            start: startBounds,
        });
        if (has(answer, OutcomeFlags.HAS_FLOATING)) {
            client.floatingBounds = rectOf(answer.floating);
        }
        if (answer.snapZone !== SnapKind.NONE && event.workArea) {
            snapClient(client as SnapClient, zoneNamed(answer.snapZone), {
                edgeTiling: true,
                area: event.workArea,
                inset: client.frameInset ?? NO_INSET,
            });
        }
    }

    /** Restores the original bounds when the drag is cancelled. */
    onCancel(event: DragEvent, state: object): void {
        const { client, startPointer, startBounds } = state as MoveState;
        if (!client.alive) {
            return;
        }
        const answer = takeStep(this.#rule(), DragKind.CANCEL, event, client, {
            origin: startPointer,
            start: startBounds,
        });
        if (has(answer, OutcomeFlags.HAS_BOUNDS)) {
            client.bounds = rectOf(answer.bounds);
        }
    }
}
