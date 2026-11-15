// Session values exposed by the canvas toolkit: time, focus, lock state, screen size and pointer position.

import { host, refuse } from "gnoblin/ui/state";

/** Returns the current time in milliseconds from a clock that only moves forward. */
export function uiNow(): number {
    return host().now();
}

/** Returns the focused compositor node id, or `null` when no node has focus. */
export function uiFocused(): number | null {
    return host().focused() ?? null;
}

/** Gives keyboard focus to a window or canvas, or clears focus with `null`. */
export function uiFocus(node: number | null): void {
    refuse(host().focus(node ?? 0), "focus");
}

/** Returns whether the session is locked or about to lock. */
export function uiLocked(): boolean {
    return host().locked();
}

/** Returns the primary screen size in logical pixels, or `null` when no screen is available. */
export function uiScreen(): { width: number; height: number } | null {
    const size = host().screen();
    return size === undefined ? null : { width: size[0], height: size[1] };
}

/** Returns the pointer position in logical screen coordinates, or `null` when no seat is available. */
export function uiPointer(): { x: number; y: number } | null {
    const x = host().seatNumber("pointer_x");
    const y = host().seatNumber("pointer_y");
    return x === undefined || y === undefined ? null : { x, y };
}

/** Returns the configured action for a titlebar click, or `null` when no value is available. */
export function uiSeatAction(key: "double_click" | "middle_click" | "right_click"): string | null {
    return host().seatText(key) ?? null;
}
