import type { FrameInset } from "gnoblin-core/inset";
import type { Client } from "gnoblin-core/host/client";
/** What the host passes to a hook. The fields are the host's; the library only reads the ones a hook names. */
export interface BindingEvent {
    /** Event time supplied by the host. */
    readonly timestamp: number;
    readonly [field: string]: unknown;
}
/** A position in pixels. */
export interface Point {
    /** Horizontal position in logical pixels. */
    x: number;
    /** Vertical position in logical pixels. */
    y: number;
}
/** A rectangle in pixels: the top-left corner and the size. */
export interface Rect {
    /** Left edge in logical pixels. */
    x: number;
    /** Top edge in logical pixels. */
    y: number;
    /** Width in logical pixels. */
    width: number;
    /** Height in logical pixels. */
    height: number;
}
/** What a stock binding reads and writes of a client, so the library never names the host's `Client`. */
export interface BindingClient {
    /** Whether the client still exists. */
    readonly alive: boolean;
    /** Whether the client is fullscreen. */
    readonly fullscreen: boolean;
    /**
     * A maximised client declines a move and a resize: the layout, not the pointer, decides its size. A snap
     * un-maximises it.
     */
    maximised?: boolean;
    /** A minimised client is not under the pointer, so the hit test skips it. */
    readonly minimised?: boolean;
    /** Current client bounds in logical pixels. */
    bounds: Rect;
    /** Minimum client size in logical pixels, when specified by the client. */
    minSize?: {
        width: number;
        height: number;
    };
    /** The most the client may be resized to. Absent means no limit. */
    maxSize?: {
        width: number;
        height: number;
    };
    /** Saved bounds used when the client is floating, or null when unset. */
    floatingBounds?: Rect | null;
    /**
     * The room the window's frame takes around the content on each side. The frame, which must stay on screen, is the
     * content with this much around it.
     */
    readonly frameInset?: FrameInset;
}
/**
 * What the matcher passes to a drag or gesture hook. `position` is the pointer in the compositor's logical
 * space and `localPosition` is relative to the hit client's origin. A pointer drag also has `x` and `y` (the
 * same as `position`), `dx` and `dy` (how far the pointer is from the press), `button` (X11 numbering: 1 left, 2
 * middle, 3 right), `workArea` (the rectangle a dragged window stays in) and `dragConstraint` (the seat's
 * `drag_constraint`). A gesture event also has `phase`, `dx`, `dy`, `scale`, `rotation` and `progress` (0 to 1,
 * how far a directed gesture has gone).
 */
export interface DragEvent extends BindingEvent {
    /** Host input context associated with the event. */
    readonly context: unknown;
    /** Input device associated with the event. */
    readonly device: unknown;
    /**
     * The window under the pointer. The shell builds the event with its own `Client`, so a hook calls `close()` or
     * reads `title` as it would on any client; the stock bindings read the `BindingClient` part of it.
     */
    readonly client?: Client;
    /** Layout that received the event, when present. */
    readonly layout?: unknown;
    /** Pointer position in compositor logical coordinates. */
    readonly position?: Point;
    /** Pointer position relative to the hit client's origin. */
    readonly localPosition?: Point;
    /** Horizontal pointer position in logical pixels. */
    readonly x?: number;
    /** Vertical pointer position in logical pixels. */
    readonly y?: number;
    /** X11 button number: 1 left, 2 middle, 3 right. */
    readonly button?: number;
    /** Work area used to constrain a moved window. */
    readonly workArea?: Rect;
    /** Seat drag constraint mode, such as `work_area` or `minimum_strip`. */
    readonly dragConstraint?: string;
    /** The seat's `edge_tiling`: whether a drop on a screen edge tiles the window. */
    readonly edgeTiling?: boolean;
    /**
     * Seconds since the previous event of this drag, at most 0.1 seconds. `deltaTime` is in seconds so a simulation
     * can use it directly.
     */
    readonly deltaTime: number;
    /** Modifier names active for this input. */
    readonly modifiers: ReadonlySet<string>;
    /** Canonical trigger spelling that matched. */
    readonly trigger: string;
    /** Gesture phase, when the event is a gesture. */
    readonly phase?: string;
    /** Horizontal gesture or pointer travel in logical pixels. */
    readonly dx?: number;
    /** Vertical gesture or pointer travel in logical pixels. */
    readonly dy?: number;
    /** Gesture scale value, when supplied by the host. */
    readonly scale?: number;
    /** Gesture rotation in radians, when supplied by the host. */
    readonly rotation?: number;
    /** Directed gesture progress from 0 to 1, when supplied. */
    readonly progress?: number;
}
/** The options every binding takes. `id` and `trigger` are required; the rest are optional. */
export interface BindingOptions {
    /** Unique among the bindings of one collection. */
    id: string;
    /** The chord, gesture or drag spelling, for example `Super+MouseLeft+Drag`. */
    trigger: string;
    /** Whether this binding is enabled. Defaults to true. */
    enabled?: boolean;
    /** Higher wins when two bindings of one collection match one input. */
    priority?: number;
}
