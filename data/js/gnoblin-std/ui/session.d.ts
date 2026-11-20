/** Returns the current time in milliseconds from a clock that only moves forward. */
export declare function uiNow(): number;
/** Returns the focused compositor node id, or `null` when no node has focus. */
export declare function uiFocused(): number | null;
/** Gives keyboard focus to a window or canvas, or clears focus with `null`. */
export declare function uiFocus(node: number | null): void;
/** Returns whether the session is locked or about to lock. */
export declare function uiLocked(): boolean;
/** Returns the primary screen size in logical pixels, or `null` when no screen is available. */
export declare function uiScreen(): {
    width: number;
    height: number;
} | null;
/** Returns the pointer position in logical screen coordinates, or `null` when no seat is available. */
export declare function uiPointer(): {
    x: number;
    y: number;
} | null;
/** Returns the configured action for a titlebar click, or `null` when no value is available. */
export declare function uiSeatAction(key: "double_click" | "middle_click" | "right_click"): string | null;
