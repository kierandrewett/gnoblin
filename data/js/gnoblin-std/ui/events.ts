// Pointer and keyboard events delivered to canvas handlers.

import type { Canvas } from "gnoblin-std/ui/canvas";
import type { UiRawEvent } from "gnoblin-std/ui/backend";

/**
 * The compositor sends this event to the canvas it hits, then to its parents until a handler calls `stop()`.
 * Coordinates are relative to the canvas whose handler is running.
 */
export class UiPointerEvent {
    /** Compositor pointer event kind, such as `press` or `motion`. */
    readonly kind: string;
    /** Horizontal position in the current handler's logical-pixel coordinates. */
    x: number;
    /** Vertical position in the current handler's logical-pixel coordinates. */
    y: number;
    /** Button number associated with the event. */
    readonly button: number;
    /** Horizontal movement or scroll distance in logical pixels. */
    readonly dx: number;
    /** Vertical movement or scroll distance in logical pixels. */
    readonly dy: number;
    /** Canvas originally targeted by the compositor. */
    readonly target: Canvas;
    /** Whether propagation to parent canvases has stopped. */
    stopped = false;

    constructor(raw: UiRawEvent, target: Canvas) {
        this.kind = raw.kind;
        this.x = raw.x;
        this.y = raw.y;
        this.button = raw.button;
        this.dx = raw.dx;
        this.dy = raw.dy;
        this.target = target;
    }

    /** Keeps the event from reaching the parents of the canvas whose handler is running. */
    stop(): void {
        this.stopped = true;
    }
}

/**
 * The compositor sends this event to the canvas with keyboard focus, then to its parents until a handler calls
 * `stop()`.
 * `key` is the xkbcommon keysym name; `text` is the typed text, or empty when the key types nothing. `pressed` is
 * false on release. Typed text may include a passphrase and is not printed by the toolkit.
 */
export class UiKeyEvent {
    /** xkbcommon keysym name, such as `Return`, `Escape` or `a`. */
    readonly key: string;
    /** Text produced by the key, or an empty string when it produces none. */
    readonly text: string;
    /** True for a key press and false for its release. */
    readonly pressed: boolean;
    /** Canvas that held keyboard focus when the event was created. */
    readonly target: Canvas;
    /** Whether propagation to parent canvases has stopped. */
    stopped = false;

    constructor(raw: UiRawEvent, target: Canvas) {
        this.key = raw.key;
        this.text = raw.text;
        this.pressed = raw.pressed;
        this.target = target;
    }

    /** Keeps the event from reaching the parents of the canvas whose handler is running. */
    stop(): void {
        this.stopped = true;
    }
}

/** Maps each compositor pointer event kind to the corresponding canvas handler name. */
export const HANDLERS: Readonly<
    Record<string, "onPress" | "onRelease" | "onMotion" | "onScroll" | "onEnter" | "onLeave">
> = {
    press: "onPress",
    release: "onRelease",
    motion: "onMotion",
    scroll: "onScroll",
    enter: "onEnter",
    leave: "onLeave",
};
