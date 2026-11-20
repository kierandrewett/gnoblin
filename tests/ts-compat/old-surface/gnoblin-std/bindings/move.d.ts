import { PointerDragBinding } from "gnoblin-core/bindings/binding";
import type { PointerDragBindingOptions } from "gnoblin-core/bindings/binding";
import type { BindingClient, DragEvent, Point, Rect } from "gnoblin-core/bindings/types";
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
export declare class ClientMoveBinding extends PointerDragBinding {
    #private;
    /** Edge distance in logical pixels for tiling on release, or false to disable tiling. */
    snap: number | false;
    constructor(options?: ClientMoveOptions);
    /** Starts a move for a movable client and records its starting position. */
    onStart(event: DragEvent): MoveState | false;
    /** Moves the client by the pointer distance from the start. */
    onUpdate(event: DragEvent, state: object): void;
    /** Saves the moved bounds and tiles on release when edge tiling is enabled. */
    onEnd(event: DragEvent, state: object): void;
    /** Restores the original bounds when the drag is cancelled. */
    onCancel(event: DragEvent, state: object): void;
}
