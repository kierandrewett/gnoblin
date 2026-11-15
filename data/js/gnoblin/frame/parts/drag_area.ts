// `DragArea`: a box that moves the window when it is dragged and answers the seat's double, middle and right clicks.

import type { FrameContext } from "gnoblin/frame/client";
import { WindowDrag } from "gnoblin/frame/drag";
import { Box } from "gnoblin/ui";
import type { BoxOptions } from "gnoblin/ui";

/**
 * A `Box` that is a handle for its window. A press on it, or on anything inside it that does not take the press
 * itself (a title, an icon), drags the window; see `WindowDrag` for the clicks. Put the title and the space between the
 * buttons in one, or give a custom bar the behaviour by making the bar a `DragArea`.
 */
export class DragArea extends Box {
    readonly context: FrameContext;
    readonly drag: WindowDrag;

    constructor(context: FrameContext, options: BoxOptions = {}) {
        super(options);
        this.context = context;
        this.drag = new WindowDrag(context);
        this.drag.attach(this);
    }
}
