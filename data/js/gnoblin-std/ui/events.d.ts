import type { Canvas } from "gnoblin-std/ui/canvas";
import type { UiRawEvent } from "gnoblin-core/host/ui_backend";
/**
 * The compositor sends this event to the canvas it hits, then to its parents until a handler calls `stop()`.
 * Coordinates are relative to the canvas whose handler is running.
 */
export declare class UiPointerEvent {
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
    stopped: boolean;
    constructor(raw: UiRawEvent, target: Canvas);
    /** Keeps the event from reaching the parents of the canvas whose handler is running. */
    stop(): void;
}
/**
 * The compositor sends this event to the canvas with keyboard focus, then to its parents until a handler calls
 * `stop()`.
 * `key` is the xkbcommon keysym name; `text` is the typed text, or empty when the key types nothing. `pressed` is
 * false on release. Typed text may include a passphrase and is not printed by the toolkit.
 */
export declare class UiKeyEvent {
    /** xkbcommon keysym name, such as `Return`, `Escape` or `a`. */
    readonly key: string;
    /** Text produced by the key, or an empty string when it produces none. */
    readonly text: string;
    /** True for a key press and false for its release. */
    readonly pressed: boolean;
    /** Canvas that held keyboard focus when the event was created. */
    readonly target: Canvas;
    /** Whether propagation to parent canvases has stopped. */
    stopped: boolean;
    constructor(raw: UiRawEvent, target: Canvas);
    /** Keeps the event from reaching the parents of the canvas whose handler is running. */
    stop(): void;
}
/** Maps each compositor pointer event kind to the corresponding canvas handler name. */
export declare const HANDLERS: Readonly<Record<string, "onPress" | "onRelease" | "onMotion" | "onScroll" | "onEnter" | "onLeave">>;
