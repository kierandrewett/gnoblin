// `CornerOutlet`: a square at a window frame's corner, over the edges that meet there. It takes no room and can hold a
// resize handle, rounded cap or badge.

import type { FrameGeometry, OutletOwner } from "gnoblin-std/frame/outlet";
import { Outlet } from "gnoblin-std/frame/outlet";
import type { Rect } from "gnoblin-core/types";

/** The four corners of a frame. */
export type Corner = "top-left" | "top-right" | "bottom-left" | "bottom-right";

/**
 * A corner of the frame: a `size` by `size` square whose outer corner is the frame's. Left `null`, `size` is the
 * thicker of the two edges that meet there. It lies over the edges and the content, in front of them, and takes
 * no room: an inset comes from the edges alone.
 */
export class CornerOutlet extends Outlet {
    /** The corner this outlet occupies. */
    readonly corner: Corner;
    /** The side of the square in pixels, or `null` for the thicker of the two edges that meet at the corner. */
    size: number | null = null;

    constructor(owner: OutletOwner, corner: Corner) {
        super(owner, corner);
        this.corner = corner;
    }

    protected override place(): Rect {
        const geometry = this.owner.geometry();
        const { inset } = geometry;
        const top = this.corner.startsWith("top");
        const left = this.corner.endsWith("left");
        const side = this.size ?? Math.max(top ? inset.top : inset.bottom, left ? inset.left : inset.right);
        return cornerRect(geometry, top, left, side);
    }
}

/** The square at one corner of the frame, in the window's coordinates. */
function cornerRect({ width, height, inset }: FrameGeometry, top: boolean, left: boolean, side: number): Rect {
    const frameLeft = -inset.left;
    const frameTop = -inset.top;
    const frameRight = width + inset.right;
    const frameBottom = height + inset.bottom;
    return {
        x: left ? frameLeft : frameRight - side,
        y: top ? frameTop : frameBottom - side,
        width: side,
        height: side,
    };
}
