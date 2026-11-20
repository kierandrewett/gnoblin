import type { Rect } from "gnoblin-core/types";
/** The room a window's frame takes outside its content, in pixels. Every side is 0 or more. */
export interface FrameInset {
    /** Space reserved above the content, in logical pixels. */
    readonly top: number;
    /** Space reserved to the right of the content, in logical pixels. */
    readonly right: number;
    /** Space reserved below the content, in logical pixels. */
    readonly bottom: number;
    /** Space reserved to the left of the content, in logical pixels. */
    readonly left: number;
}
/** No room on any side: the inset of a window with no frame, and of a fullscreen one. */
export declare const NO_INSET: FrameInset;
/** Whether `inset` takes no room on any side. */
export declare function isNoInset(inset: FrameInset): boolean;
/** Whether two insets are the same on every side. */
export declare function sameInset(a: FrameInset, b: FrameInset): boolean;
/** The frame of a content rectangle: `content` with `inset` added on each side. */
export declare function growRect(content: Rect, inset: FrameInset): Rect;
/** The content rectangle of a frame: `frame` less `inset` on each side, never smaller than nothing. */
export declare function shrinkRect(frame: Rect, inset: FrameInset): Rect;
/** `inset` read from the four numbers a generation snapshot keeps: top, right, bottom, left. */
export declare function insetFromNumbers(numbers: readonly number[] | null | undefined): FrameInset | null;
/** The four numbers a generation snapshot keeps for `inset`: top, right, bottom, left. */
export declare function insetToNumbers(inset: FrameInset): number[];
