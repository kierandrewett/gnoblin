import type { FrameInset } from "gnoblin-core/inset";
import { Layout } from "gnoblin-std/layout";
import type { LayoutContext } from "gnoblin-std/layout";
import type { Rect } from "gnoblin-core/types";
/** `smart` is the first spot in the area that overlaps nothing; `center` and `cascade` are as named. */
export type FloatingPlacement = "smart" | "center" | "cascade";
/** The internal placement mode `placeRect` takes, one per `FloatingPlacement`. */
export type PlaceMode = "free_space" | "center" | "cascade";
/** Options of `FloatingLayout`. */
export interface FloatingOptions {
    /** Placement strategy for new windows. Default `smart`. */
    placement?: FloatingPlacement;
    /** Distance between cascade positions, in logical pixels. Default 32. */
    cascadeStep?: number;
    /** `undefined` keeps the default trigger, a string replaces it and `null` leaves the binding out. */
    triggers?: {
        move?: string | null;
        resize?: string | null;
    };
}
/**
 * The part of a client the placement reads and writes. `floatingBounds` is set by the host once it has a position the
 * user chose.
 */
export interface FloatingClient {
    /** Current content bounds, in logical pixels. */
    bounds: Rect;
    /** Client-committed content size, in logical pixels, when available. */
    size?: {
        width: number;
        height: number;
    };
    /** Host-set bounds for a window the user has already positioned. */
    floatingBounds?: Rect | null;
    /** Whether this window is included when choosing a placement. */
    visible?: boolean;
    /** Whether the window currently uses tiled placement. */
    tiled?: boolean;
    /**
     * The room the window's frame takes round the content. A window is placed as a frame: the content with this much
     * on each side.
     */
    readonly frameInset?: FrameInset;
}
/**
 * A layout that does not tile. Each window is placed once when it first appears; the user then moves and resizes it.
 */
export declare class FloatingLayout extends Layout {
    #private;
    /** Strategy used to place new windows. Default `smart`. */
    placement: FloatingPlacement;
    /** Distance between cascade positions, in logical pixels. Default 32. */
    cascadeStep: number;
    /** Resolved `move` and `resize` triggers; `null` means that binding is disabled. */
    readonly triggers: Readonly<Record<"move" | "resize", string | null>>;
    private readonly placed;
    constructor(options?: FloatingOptions);
    /** The rectangle for a new window of `width` by `height` in `area`, avoiding `occupied` where possible. */
    place(width: number, height: number, area: Rect, occupied: readonly Rect[]): Rect;
    /** Places a newly added client once unless it already has floating bounds or was placed by this layout. */
    onClientAdded(client: FloatingClient, context: LayoutContext): void;
}
/**
 * The rectangle for a new window of `width` by `height` in `area`, avoiding `occupied` where possible. A size
 * below 1 on an axis is raised to 1, so the window covers something. The placement is the library's.
 */
export declare function placeRect(mode: PlaceMode, width: number, height: number, area: Rect, occupied: readonly Rect[], cascadeStep?: number): Rect;
