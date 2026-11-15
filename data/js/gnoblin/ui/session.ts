// What the stock shell reads of the session through the toolkit's host: the clock, the keyboard focus, the lock, the
// screen and the seat.

import { host, refuse } from "gnoblin/ui/state";

/** Milliseconds on a clock that only goes forward: what the toolkit times its animations and delays by. */
export function uiNow(): number {
    return host().now();
}

/** The node with the keyboard focus, or `null`. */
export function uiFocused(): number | null {
    return host().focused() ?? null;
}

/** Gives the keyboard to a node: a window (or a canvas), or `null` to take it away. */
export function uiFocus(node: number | null): void {
    refuse(host().focus(node ?? 0), "focus");
}

/** Whether the session is locked or about to be. */
export function uiLocked(): boolean {
    return host().locked();
}

/** The size of the primary screen, or `null` while the tree has none. */
export function uiScreen(): { width: number; height: number } | null {
    const size = host().screen();
    return size === undefined ? null : { width: size[0], height: size[1] };
}

/** The seat's pointer in the compositor's logical coordinates, or `null` with no seat. */
export function uiPointer(): { x: number; y: number } | null {
    const x = host().seatNumber("pointer_x");
    const y = host().seatNumber("pointer_y");
    return x === undefined || y === undefined ? null : { x, y };
}

/** What a titlebar click does, from the seat: `key` is `double_click`, `middle_click` or `right_click`. */
export function uiSeatAction(key: "double_click" | "middle_click" | "right_click"): string | null {
    return host().seatText(key) ?? null;
}
