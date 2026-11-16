// `ResizeHandles`: the eight handles round a window's frame that resize it. A server-decorated window has no edge to
// take hold of unless a frame puts handles there. The handles reach `thickness` pixels outside the frame, past the
// window's geometry, where the compositor still hits them.

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
export class ResizeHandles extends Box {
    /** Distance each handle reaches outside the frame, in logical pixels. */
    thickness: number;
    /** Distance each handle reaches inside the frame, in logical pixels. */
    inside: number;
    /** Length of each corner handle and the edge-handle gap, in logical pixels. */
    corner: number;
    /** Each resize handle, keyed by its edge or corner. */
    readonly handles = new Map<ResizeSide, ResizeHandle>();

    constructor(context: FrameContext, options: ResizeHandlesOptions = {}) {
        super({ input: false });
        this.thickness = options.thickness ?? 6;
        this.inside = options.inside ?? 2;
        this.corner = options.corner ?? 14;
        const sides: ResizeSide[] = [
            "top-left",
            "top-right",
            "bottom-left",
            "bottom-right",
            "top",
            "bottom",
            "left",
            "right",
        ];
        for (const side of sides) {
            const handle = new ResizeHandle(context, side);
            handle.fitParent = false;
            this.handles.set(side, handle);
            this.add(handle);
        }
    }

    /** Sizes this node to its parent layer. */
    override refresh(): void {
        // The layer this is in is as large as the frame; this node is as large as the layer.
        const { parent } = this;
        if (parent instanceof Box) {
            this.x = 0;
            this.y = 0;
            this.width = parent.width;
            this.height = parent.height;
        }
    }

    /** Places each edge and corner handle using this node's size. */
    override arrange(): void {
        const { thickness: t, inside, corner } = this;
        const reach = t + inside;
        const run = corner - t;
        const [width, height] = [this.width, this.height];
        const place = (side: ResizeSide, x: number, y: number, w: number, h: number): void => {
            const handle = this.handles.get(side);
            if (handle !== undefined) {
                [handle.x, handle.y, handle.width, handle.height] = [x, y, Math.max(0, w), Math.max(0, h)];
            }
        };
        place("top-left", -t, -t, corner, corner);
        place("top-right", width - run, -t, corner, corner);
        place("bottom-left", -t, height - run, corner, corner);
        place("bottom-right", width - run, height - run, corner, corner);
        place("top", run, -t, width - 2 * run, reach);
        place("bottom", run, height - inside, width - 2 * run, reach);
        place("left", -t, run, reach, height - 2 * run);
        place("right", width - inside, run, reach, height - 2 * run);
    }

    /** Returns no paint operations; the handles have no visible drawing. */
    override draw(): [] {
        return [];
    }
}
