// `WindowDrag` handles presses on frame parts: dragging moves a window, while double, middle and right clicks run the
// seat's `double_click`, `middle_click` and `right_click` actions. `DragArea` uses it; other canvases can use `attach`.
// a right click do what the seat's `double_click`, `middle_click` and `right_click` say. `DragArea` is a node that
// has it; any other canvas can take it with `attach`.

import type { FrameContext } from "gnoblin-std/frame/client";
import type { Canvas } from "gnoblin-std/ui/canvas";
import type { UiPointerEvent } from "gnoblin-std/ui/events";

/** The buttons of the pointer, as the compositor numbers them (Linux `BTN_LEFT`, `BTN_RIGHT`, `BTN_MIDDLE`). */
const LEFT = 272;
const RIGHT = 273;
const MIDDLE = 274;

/** Two presses within this many milliseconds are a double click. */
const DOUBLE_CLICK_MS = 500;

/**
 * The window behaviour of a drag area. A left press takes the window (`hooks.press`: it takes the focus), and a drag
 * moves the client by how far the pointer is from where it pressed, in the node's own coordinates. The node is a child
 * of the window, so those coordinates stand still while the window follows the pointer. A second left press within half
 * a second runs the seat's `double_click` action instead of starting a drag. A right or middle press is left to its
 * action: the window menu a right click opens holds the keyboard, and the press is heard later.
 * the window menu a right click opens holds the keyboard, and the press is heard later.
 */
export class WindowDrag {
    /** Window and shell operations used by this drag behaviour. */
    readonly context: FrameContext;
    #lastPress: number | null = null;
    #from: { x: number; y: number } | null = null;

    constructor(context: FrameContext) {
        this.context = context;
    }

    /** Makes `canvas` a drag area: its press, motion and release handlers are this behaviour's. */
    attach(canvas: Canvas): void {
        canvas.onPress = (event) => this.press(event);
        canvas.onMotion = (event) => this.motion(event);
        canvas.onRelease = (event) => this.release(event);
    }

    /** Handles a pointer press and starts a drag or invokes a seat gesture. */
    press(event: UiPointerEvent): void {
        event.stop();
        const { client, hooks, actions } = this.context;
        if (event.button === LEFT) {
            hooks.press?.(client, { x: event.x, y: event.y, button: event.button });
        }
        if (event.button === MIDDLE) {
            actions.gesture("middle_click");
            return;
        }
        if (event.button === RIGHT) {
            actions.gesture("right_click");
            return;
        }
        if (event.button !== LEFT) {
            return;
        }
        const now = actions.now();
        const previous = this.#lastPress;
        this.#lastPress = now;
        if (previous !== null && now - previous <= DOUBLE_CLICK_MS) {
            this.#lastPress = null;
            actions.gesture("double_click");
            return;
        }
        this.#from = { x: event.x, y: event.y };
    }

    /** Moves the window while a left-button drag is active. */
    motion(event: UiPointerEvent): void {
        const from = this.#from;
        if (from === null) {
            return;
        }
        event.stop();
        this.context.client.x += event.x - from.x;
        this.context.client.y += event.y - from.y;
    }

    /** Ends an active drag when the pointer button is released. */
    release(event: UiPointerEvent): void {
        const { client, hooks } = this.context;
        hooks.release?.(client, { x: event.x, y: event.y, button: event.button });
        if (this.#from !== null) {
            event.stop();
            this.#from = null;
        }
    }
}
