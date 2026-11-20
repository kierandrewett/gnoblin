import type { OutletOwner } from "gnoblin-std/frame/outlet";
import { Outlet } from "gnoblin-std/frame/outlet";
import type { Rect } from "gnoblin-core/types";
/** The four corners of a frame. */
export type Corner = "top-left" | "top-right" | "bottom-left" | "bottom-right";
/**
 * A corner of the frame: a `size` by `size` square whose outer corner is the frame's. Left `null`, `size` is the
 * thicker of the two edges that meet there. It lies over the edges and the content, in front of them, and takes
 * no room: an inset comes from the edges alone.
 */
export declare class CornerOutlet extends Outlet {
    /** The corner this outlet occupies. */
    readonly corner: Corner;
    /** The side of the square in pixels, or `null` for the thicker of the two edges that meet at the corner. */
    size: number | null;
    constructor(owner: OutletOwner, corner: Corner);
    protected place(): Rect;
}
