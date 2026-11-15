// The events the compositor sends to a canvas, as a handler receives them, and the handler each pointer kind runs.

import type { Canvas } from "gnoblin/ui/canvas";
import type { UiRawEvent } from "gnoblin/ui/backend";

/**
 * A pointer event on a canvas. It goes to the canvas the compositor hit and then up through its parents until a
 * handler calls `stop()`. `x` and `y` are in the coordinates of the canvas whose handler runs.
 */
export class UiPointerEvent {
    readonly kind: string;
    x: number;
    y: number;
    readonly button: number;
    readonly dx: number;
    readonly dy: number;
    readonly target: Canvas;
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
 * A key sent to the canvas that holds the keyboard focus (`Canvas.focus()`), and from it up through its parents until a
 * handler calls `stop()`. `key` is the xkbcommon name of the keysym (`Return`, `Escape`, `a`), `text` what the key
 * types (empty for a key that types nothing), and `pressed` is false for the release. A key typed into an entry can
 * be a passphrase: nothing here prints it.
 */
export class UiKeyEvent {
    readonly key: string;
    readonly text: string;
    readonly pressed: boolean;
    readonly target: Canvas;
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

/** The handler a canvas may define for each pointer event kind the compositor sends. */
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
