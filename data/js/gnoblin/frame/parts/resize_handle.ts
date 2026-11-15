// `ResizeHandle`: a node that resizes its window when it is dragged. Put it on any edge or corner, inside the frame or
// outside the window's geometry: the compositor hits a node outside its parent's rectangle on its own.

import { resizedRect } from "gnoblin/bindings/resize";
import type { ResizeCorner, ResizeEdge } from "gnoblin/bindings/resize";
import type { FrameContext } from "gnoblin/frame/client";
import { Box } from "gnoblin/ui";
import type { BoxOptions, UiPointerEvent } from "gnoblin/ui";

/** The side or corner of the window a handle holds: it moves with the pointer, the opposite one stays. */
export type ResizeSide = ResizeCorner | ResizeEdge;

/** The buttons of the pointer, as the compositor numbers them (Linux `BTN_LEFT`). */
const LEFT = 272;

/**
 * A resize handle. A left press on it, and a drag, moves the side or corner it holds by how far the pointer moves in
 * the handle's own coordinates. The handle is a child of the window, so those coordinates stand still while the
 * window follows the pointer, and the move is taken in steps: each motion moves the window by what is left. The
 * window keeps the size the client states as its least (`minSize`) and most (`maxSize`), or the stock least. A maximised
 * or fullscreen window is not resized: the handle is hidden then. The press is the window's too (`hooks.press`):
 * it takes the focus.
 *
 * It paints nothing, so it has no look. Under a parent that leaves its nodes where they are (a corner outlet, a layer)
 * it is as large as the parent unless `fitParent` is false; in a row or column it takes the room `grow` gives it.
 */
export class ResizeHandle extends Box {
    readonly context: FrameContext;
    readonly side: ResizeSide;
    /** Whether the handle is as large as its parent while the parent leaves its nodes where they are. */
    fitParent = true;
    #from: { x: number; y: number } | null = null;

    constructor(context: FrameContext, side: ResizeSide, options: BoxOptions = {}) {
        super(options);
        this.context = context;
        this.side = side;
        this.onPress = (event) => this.#press(event);
        this.onMotion = (event) => this.#motion(event);
        this.onRelease = (event) => this.#release(event);
    }

    override refresh(): void {
        const { client } = this.context;
        this.visible = !client.maximised && !client.fullscreen;
        // In a corner outlet or a layer, which leave their nodes where they are, the handle is as large as the place.
        const { parent } = this;
        if (parent instanceof Box && parent.direction === "none" && this.fitParent) {
            this.x = 0;
            this.y = 0;
            this.width = parent.width;
            this.height = parent.height;
        }
    }

    override draw(): [] {
        return [];
    }

    #press(event: UiPointerEvent): void {
        event.stop();
        if (event.button !== LEFT) {
            return;
        }
        const { client, hooks } = this.context;
        hooks.press?.(client, { x: event.x, y: event.y, button: event.button });
        this.#from = { x: event.x, y: event.y };
    }

    #motion(event: UiPointerEvent): void {
        const from = this.#from;
        const { client } = this.context;
        if (from === null || client.bounds === undefined) {
            return;
        }
        event.stop();
        const delta = { x: event.x - from.x, y: event.y - from.y };
        client.bounds = resizedRect(client.bounds, this.side, delta, client.minSize, client.maxSize);
    }

    #release(event: UiPointerEvent): void {
        const { client, hooks } = this.context;
        hooks.release?.(client, { x: event.x, y: event.y, button: event.button });
        if (this.#from !== null) {
            event.stop();
            this.#from = null;
        }
    }
}
