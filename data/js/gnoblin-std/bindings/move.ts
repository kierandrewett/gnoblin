// `ClientMoveBinding`: drag a window by the pointer, with the edge snapping of `gnoblin-std/snap`.

import { PointerDragBinding } from "gnoblin-core/bindings/binding";
import type { PointerDragBindingOptions } from "gnoblin-core/bindings/binding";
import { constrainRect, dragConstraintMode, draggable, moved } from "gnoblin-std/bindings/drag";
import type { BindingClient, DragEvent, Point, Rect } from "gnoblin-core/bindings/types";
import { growRect, NO_INSET, shrinkRect } from "gnoblin-core/inset";
import { edgeZone, forgetSnap, SNAP_MARGIN, snapClient } from "gnoblin-std/snap";
import type { SnapClient } from "gnoblin-std/snap";

/** What `ClientMoveBinding.onStart` keeps for one drag. */
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

    /** Starts a move for a movable client and records its starting position. */
    onStart(event: DragEvent): MoveState | false {
        const { client, position } = event;
        if (!client || !position || !draggable(client)) {
            return false;
        }
        // A snapped window the user takes hold of is no longer tiled to its zone.
        forgetSnap(client);
        return { client, startPointer: { ...position }, startBounds: { ...client.bounds } };
    }

    /** Moves the client by the pointer distance from the start. */
    onUpdate(event: DragEvent, state: object): void {
        const { client, startPointer, startBounds } = state as MoveState;
        if (!client.alive || !event.position) {
            return;
        }
        const delta = moved(startPointer, event.position);
        const target = { ...startBounds, x: startBounds.x + delta.x, y: startBounds.y + delta.y };
        // The seat's constraint keeps the window reachable. Without a work area there is nothing to keep it in. The
        // frame (the content with its frame around it) is what stays in: a titlebar dragged off the top could not be
        // grabbed again.
        if (!event.workArea) {
            client.bounds = target;
            return;
        }
        const inset = client.frameInset ?? NO_INSET;
        const frame = constrainRect(growRect(target, inset), event.workArea, dragConstraintMode(event.dragConstraint));
        client.bounds = { ...shrinkRect(frame, inset), width: target.width, height: target.height };
    }

    /** Saves the moved bounds and tiles on release when edge tiling is enabled. */
    onEnd(event: DragEvent, state: object): void {
        const { client } = state as MoveState;
        if (!client.alive) {
            return;
        }
        client.floatingBounds = { ...client.bounds };
        if (this.snap === false || event.edgeTiling !== true || !event.workArea || !event.position) {
            return;
        }
        const zone = edgeZone(event.position.x, event.position.y, event.workArea, this.snap);
        if (zone !== null) {
            snapClient(client as SnapClient, zone, {
                edgeTiling: true,
                area: event.workArea,
                inset: client.frameInset ?? NO_INSET,
            });
        }
    }

    /** Restores the original bounds when the drag is cancelled. */
    onCancel(_event: DragEvent, state: object): void {
        const { client, startBounds } = state as MoveState;
        if (client.alive) {
            client.bounds = { ...startBounds };
        }
    }
}
