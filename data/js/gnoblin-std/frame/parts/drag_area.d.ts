import type { FrameContext } from "gnoblin-std/frame/client";
import { WindowDrag } from "gnoblin-std/frame/drag";
import { Box } from "gnoblin-std/ui/box";
import type { BoxOptions } from "gnoblin-std/ui/box";
/**
 * A `Box` that is a handle for its window. A press on it, or on anything inside it that does not take the press
 * itself (a title, an icon), drags the window; see `WindowDrag` for the clicks. Put the title and the space between the
 * buttons in one, or give a custom bar the behaviour by making the bar a `DragArea`.
 */
export declare class DragArea extends Box {
    /** Window and shell operations this area delegates to. */
    readonly context: FrameContext;
    /** Window, shell hooks and actions used by this drag area. */
    readonly drag: WindowDrag;
    constructor(context: FrameContext, options?: BoxOptions);
}
