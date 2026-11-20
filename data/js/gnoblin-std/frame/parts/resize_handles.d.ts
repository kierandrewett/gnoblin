import type { FrameContext } from "gnoblin-std/frame/client";
import { ResizeHandle } from "gnoblin-std/frame/parts/resize_handle";
import type { ResizeSide } from "gnoblin-std/frame/parts/resize_handle";
import { Box } from "gnoblin-std/ui/box";
/** Options of `ResizeHandles`, in pixels. */
export interface ResizeHandlesOptions {
    /** How far a handle reaches outside the frame. */
    thickness?: number;
    /** How far a handle reaches inside the frame. */
    inside?: number;
    /** The side of a corner handle, which is also how far an edge handle stops short of the corner. */
    corner?: number;
}
/**
 * Four edge handles and four corner handles, laid round the frame from the size of the node that holds them. Put one in
 * the `overlay`, which is as large as the frame: `frame.resizeHandles(6)` does. The node lets the pointer through
 * everywhere but on a handle.
 */
export declare class ResizeHandles extends Box {
    /** Distance each handle reaches outside the frame, in logical pixels. */
    thickness: number;
    /** Distance each handle reaches inside the frame, in logical pixels. */
    inside: number;
    /** Length of each corner handle and the edge-handle gap, in logical pixels. */
    corner: number;
    /** Each resize handle, keyed by its edge or corner. */
    readonly handles: Map<ResizeSide, ResizeHandle>;
    constructor(context: FrameContext, options?: ResizeHandlesOptions);
    /** Sizes this node to its parent layer. */
    refresh(): void;
    /** Places each edge and corner handle using this node's size. */
    arrange(): void;
    /** Returns no paint operations; the handles have no visible drawing. */
    draw(): [];
}
