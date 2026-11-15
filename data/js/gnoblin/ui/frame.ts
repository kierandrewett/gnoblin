// The toolkit's frame: what the compositor sent since the last frame is delivered, the frame hooks run and each root
// canvas updates its tree. Also the listeners for window requests and the release of a generation's canvases.

import { UiKeyEvent, UiPointerEvent } from "gnoblin/ui/events";
import { attached, host, live, measuring, reportError } from "gnoblin/ui/state";

/** The host tells the toolkit a node was destroyed (the compositor closed a window, and its canvases went). */
export function uiNodeGone(node: number): void {
    live.get(node)?.markGone();
}

/** Destroys every canvas this generation made. A generation that is replaced calls this, so none is left behind. */
export function uiReleaseAll(): void {
    for (const canvas of [...live.values()]) {
        if (canvas.parent === null) {
            canvas.destroy();
        }
    }
    measuring.clear();
}

/** The number of live canvases. For tests and diagnostics. */
export function uiCanvasCount(): number {
    return live.size;
}

/** A request the compositor made of a window: `request` is `menu` for its window menu, anchored at `x`, `y`. */
export interface UiRequest {
    /** The window's node. */
    readonly node: number;
    readonly request: string;
    /** Where the menu is anchored, in the compositor's logical coordinates. */
    readonly x: number;
    readonly y: number;
}

/** Listeners for window requests, in the order they were added. `runUiFrame` delivers each request to all of them. */
const requestHandlers: ((request: UiRequest) => void)[] = [];
const frameHooks: (() => void)[] = [];

/** Hears every request the compositor makes of a window. Returns the function that stops listening. */
export function onUiRequest(handler: (request: UiRequest) => void): () => void {
    requestHandlers.push(handler);
    return () => {
        const at = requestHandlers.indexOf(handler);
        if (at >= 0) {
            requestHandlers.splice(at, 1);
        }
    };
}

/** Runs `hook` once a frame, after the events are delivered and before the canvases update. Returns the stop. */
export function onUiFrame(hook: () => void): () => void {
    frameHooks.push(hook);
    return () => {
        const at = frameHooks.indexOf(hook);
        if (at >= 0) {
            frameHooks.splice(at, 1);
        }
    };
}

/**
 * Delivers what the compositor sent since the last frame. Answers come first, so a handler that reads a text's size
 * sees the measurement that just arrived. Requests go to the listeners, not to a canvas: a window's menu is not a
 * canvas event.
 */
function deliver(): void {
    const toolkit = host();
    for (const answer of toolkit.takeAnswers()) {
        const asked = measuring.get(answer.token);
        measuring.delete(answer.token);
        if (asked !== undefined && asked.text.alive) {
            asked.text.answered(asked.key, answer);
        }
    }
    for (const raw of toolkit.takeEvents()) {
        if (raw.kind === "request") {
            const request: UiRequest = { node: raw.node, request: raw.request, x: raw.x, y: raw.y };
            for (const handler of [...requestHandlers]) {
                try {
                    handler(request);
                } catch (error) {
                    reportError("onUiRequest", error);
                }
            }
            continue;
        }
        const target = live.get(raw.node);
        if (target === undefined) {
            continue;
        }
        if (raw.kind === "key") {
            target.dispatchKey(new UiKeyEvent(raw, target));
        } else {
            target.dispatch(new UiPointerEvent(raw, target));
        }
    }
}

/**
 * One frame of the toolkit, called by the host after the layouts. In order: the measurements and pointer or key
 * events the compositor sent are delivered; the frame hooks run; then each root canvas updates its whole tree.
 * Does nothing until a host is attached.
 */
export function runUiFrame(): void {
    // No host is attached, so there is no tree to update.
    if (!attached()) {
        return;
    }
    deliver();
    for (const hook of [...frameHooks]) {
        try {
            hook();
        } catch (error) {
            reportError("onUiFrame", error);
        }
    }
    for (const canvas of [...live.values()]) {
        if (canvas.parent === null) {
            canvas.update();
        }
    }
}
