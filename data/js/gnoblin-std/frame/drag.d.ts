import type { FrameContext } from "gnoblin-std/frame/client";
import type { Canvas } from "gnoblin-std/ui/canvas";
import type { UiPointerEvent } from "gnoblin-std/ui/events";
/**
 * The window behaviour of a drag area. A left press takes the window (`hooks.press`: it takes the focus), and a drag
 * moves the client by how far the pointer is from where it pressed, in the node's own coordinates. The node is a child
 * of the window, so those coordinates stand still while the window follows the pointer. A second left press within half
 * a second runs the seat's `double_click` action instead of starting a drag. A right or middle press is left to its
 * action: the window menu a right click opens holds the keyboard, and the press is heard later.
 * the window menu a right click opens holds the keyboard, and the press is heard later.
 */
export declare class WindowDrag {
    #private;
    /** Window and shell operations used by this drag behaviour. */
    readonly context: FrameContext;
    constructor(context: FrameContext);
    /** Makes `canvas` a drag area: its press, motion and release handlers are this behaviour's. */
    attach(canvas: Canvas): void;
    /** Handles a pointer press and starts a drag or invokes a seat gesture. */
    press(event: UiPointerEvent): void;
    /** Moves the window while a left-button drag is active. */
    motion(event: UiPointerEvent): void;
    /** Ends an active drag when the pointer button is released. */
    release(event: UiPointerEvent): void;
}
