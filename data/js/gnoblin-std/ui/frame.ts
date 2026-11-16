// Delivers compositor events, runs frame hooks and updates canvas trees once per frame.

import { UiKeyEvent, UiPointerEvent } from "gnoblin-std/ui/events";
import { attached, host, live, measuring, reportError } from "gnoblin-std/ui/state";

/** Marks a compositor node and all canvases in its subtree as gone. */
export function uiNodeGone(node: number): void {
    live.get(node)?.markGone();
}

/** Destroys all canvases made in the current session and clears pending measurements. */
export function uiReleaseAll(): void {
    for (const canvas of [...live.values()]) {
        if (canvas.parent === null) {
            canvas.destroy();
        }
    }
    measuring.clear();
}

/** Returns the number of live canvases. Intended for tests and diagnostics. */
export function uiCanvasCount(): number {
    return live.size;
}

/** A compositor request for a window, such as a menu, with its anchor in logical screen coordinates. */
export interface UiRequest {
    /** The window's node. */
    readonly node: number;
    /** The request kind, such as `menu`. */
    readonly request: string;
    /** Horizontal anchor position in logical screen pixels. */
    readonly x: number;
    /** Vertical anchor position in logical screen pixels. */
    readonly y: number;
}

const requestHandlers: ((request: UiRequest) => void)[] = [];
// Frame callbacks run after compositor events and before canvas updates.
const frameHooks: (() => void)[] = [];

/** Listens for compositor window requests. Returns a function that removes this listener. */
export function onUiRequest(handler: (request: UiRequest) => void): () => void {
    requestHandlers.push(handler);
    return () => {
        const at = requestHandlers.indexOf(handler);
        if (at >= 0) {
            requestHandlers.splice(at, 1);
        }
    };
}

/** Runs `hook` after event delivery and before canvas updates on each frame. Returns a function that removes it. */
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
