// `FrameInset`: the room a window's frame takes on each side of its content, and the arithmetic on it.
// A frame is the content with its inset around it. The layout driver, floating placement, drags and snaps place frames
// and write the content rectangle that is left. Pure functions, so the host and the stock layouts share one rule.
// design/window-frame.md, decision 5.

import type { Rect } from "gnoblin/types";

/** The room a window's frame takes outside its content, in pixels. Every side is 0 or more. */
export interface FrameInset {
    readonly top: number;
    readonly right: number;
    readonly bottom: number;
    readonly left: number;
}

/** No room on any side: the inset of a window with no frame, and of a fullscreen one. */
export const NO_INSET: FrameInset = Object.freeze({ top: 0, right: 0, bottom: 0, left: 0 });

/** Whether `inset` takes no room on any side. */
export function isNoInset(inset: FrameInset): boolean {
    return inset.top === 0 && inset.right === 0 && inset.bottom === 0 && inset.left === 0;
}

/** Whether two insets are the same on every side. */
export function sameInset(a: FrameInset, b: FrameInset): boolean {
    return a.top === b.top && a.right === b.right && a.bottom === b.bottom && a.left === b.left;
}

/** The frame of a content rectangle: `content` with `inset` added on each side. */
export function growRect(content: Rect, inset: FrameInset): Rect {
    return {
        x: content.x - inset.left,
        y: content.y - inset.top,
        width: content.width + inset.left + inset.right,
        height: content.height + inset.top + inset.bottom,
    };
}

/** The content rectangle of a frame: `frame` less `inset` on each side, never smaller than nothing. */
export function shrinkRect(frame: Rect, inset: FrameInset): Rect {
    return {
        x: frame.x + inset.left,
        y: frame.y + inset.top,
        width: Math.max(0, frame.width - inset.left - inset.right),
        height: Math.max(0, frame.height - inset.top - inset.bottom),
    };
}

/** `inset` read from the four numbers a generation snapshot keeps: top, right, bottom, left. */
export function insetFromNumbers(numbers: readonly number[] | null | undefined): FrameInset | null {
    if (numbers === null || numbers === undefined || numbers.length !== 4) {
        return null;
    }
    return { top: numbers[0], right: numbers[1], bottom: numbers[2], left: numbers[3] };
}

/** The four numbers a generation snapshot keeps for `inset`: top, right, bottom, left. */
export function insetToNumbers(inset: FrameInset): number[] {
    return [inset.top, inset.right, inset.bottom, inset.left];
}
