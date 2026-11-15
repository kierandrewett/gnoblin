// The shapes a binding handler receives and the options every binding takes: the event, the client it acts on, a
// drag in progress. Types only, so a layout or a test can name them without the binding classes.

import type { FrameInset } from "gnoblin/frame/inset";

/** What the host passes to a hook. The fields are the host's; the library only reads the ones a hook names. */
export interface BindingEvent {
    readonly timestamp: number;
    readonly [field: string]: unknown;
}

/** A position in pixels. */
export interface Point {
    x: number;
    y: number;
}

/** A rectangle in pixels: the top-left corner and the size. */
export interface Rect {
    x: number;
    y: number;
    width: number;
    height: number;
}

/** What a stock binding reads and writes of a client, so the library never names the host's `Client`. */
export interface BindingClient {
    readonly alive: boolean;
    readonly fullscreen: boolean;
    /**
     * A maximised client declines a move and a resize: the layout, not the pointer, decides its size. A snap
     * un-maximises it.
     */
    maximised?: boolean;
    /** A minimised client is not under the pointer, so the hit test skips it. */
    readonly minimised?: boolean;
    bounds: Rect;
    /** The least the client may be resized to. The tree carries no such limit today, so it is usually absent. */
    minSize?: { width: number; height: number };
    /** The most the client may be resized to. Absent means no limit. */
    maxSize?: { width: number; height: number };
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
    readonly context: unknown;
    readonly device: unknown;
    readonly client?: BindingClient;
    readonly layout?: unknown;
    readonly position?: Point;
    readonly localPosition?: Point;
    readonly x?: number;
    readonly y?: number;
    readonly button?: number;
    readonly workArea?: Rect;
    readonly dragConstraint?: string;
    /** The seat's `edge_tiling`: whether a drop on a screen edge tiles the window. */
    readonly edgeTiling?: boolean;
    /**
     * Seconds since the previous event of this drag, at most 0.1. The one time value in seconds: the specification
     * gives `deltaTime` in seconds so a simulation can integrate it without a conversion.
     */
    readonly deltaTime: number;
    readonly modifiers: ReadonlySet<string>;
    readonly trigger: string;
    readonly phase?: string;
    readonly dx?: number;
    readonly dy?: number;
    readonly scale?: number;
    readonly rotation?: number;
    readonly progress?: number;
}

/** The options every binding takes. `id` and `trigger` are required; the rest are optional. */
export interface BindingOptions {
    /** Unique among the bindings of one collection. */
    id: string;
    /** The chord, gesture or drag spelling, for example `Super+MouseLeft+Drag`. */
    trigger: string;
    enabled?: boolean;
    /** Higher wins when two bindings of one collection match one input. */
    priority?: number;
}
