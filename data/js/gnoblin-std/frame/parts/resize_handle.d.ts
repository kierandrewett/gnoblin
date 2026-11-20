import type { ResizeCorner, ResizeEdge } from "gnoblin-std/bindings/resize";
import type { FrameContext } from "gnoblin-std/frame/client";
import { Box } from "gnoblin-std/ui/box";
import type { BoxOptions } from "gnoblin-std/ui/box";
/** The side or corner of the window a handle holds: it moves with the pointer, the opposite one stays. */
export type ResizeSide = ResizeCorner | ResizeEdge;
/**
 * A resize handle. A left press on it, and a drag, moves the side or corner it holds by how far the pointer moves in
 * the handle's own coordinates. The handle is a child of the window, so those coordinates stand still while the
 * window follows the pointer, and the move is taken in steps: each motion moves the window by what is left. The
 * window keeps the size the client states as its least (`minSize`) and most (`maxSize`), or the stock least.
 * A maximised or fullscreen window is not resized: the handle is hidden then. The press is the window's too
 * (`hooks.press`): it takes the focus.
 *
 * It paints nothing, so it has no look. Under a parent that leaves its nodes where they are (a corner outlet, a layer)
 * it is as large as the parent unless `fitParent` is false; in a row or column it takes the room `grow` gives it.
 */
export declare class ResizeHandle extends Box {
    #private;
    /** Window geometry and hooks used to resize and focus it. */
    readonly context: FrameContext;
    /** The edge or corner that stays under the pointer while resizing. */
    readonly side: ResizeSide;
    /** Whether the handle is as large as its parent while the parent leaves its nodes where they are. */
    fitParent: boolean;
    constructor(context: FrameContext, side: ResizeSide, options?: BoxOptions);
    /** Updates visibility and fits the handle to its parent when configured. */
    refresh(): void;
    /** Returns no paint operations; the handle has no visible drawing. */
    draw(): [];
}
