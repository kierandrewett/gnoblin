import type { FrameInset } from "gnoblin-core/inset";
import type { Rect } from "gnoblin-core/types";
import type { SnapClient, SnapRecord } from "gnoblin-std/snap";
import { Box } from "gnoblin-std/ui/box";
import type { UiKeyEvent } from "gnoblin-std/ui/events";
/** How far an arrow key moves or resizes the window, in pixels. */
export declare const REPOSITION_STEP = 16;
/** The least width a resize keeps, in pixels. */
export declare const REPOSITION_MIN_WIDTH = 120;
/** The least height a resize keeps, in pixels. */
export declare const REPOSITION_MIN_HEIGHT = 80;
/** What an interactive drag does to a window: `move` carries it, `resize` changes its size. */
export type RepositionMode = "move" | "resize";
/** What the mode needs of a window. A `Client` of the host module is one. */
export interface RepositionClient extends SnapClient {
    /** False after the window closes. */
    readonly alive: boolean;
    /** Whether the window fills its output. */
    readonly fullscreen: boolean;
    /** Whether the window is maximised. */
    readonly maximised: boolean;
    /** Whether the window is arranged by a layout. */
    readonly tiled: boolean;
    /** Whether the window's contents are secure. */
    readonly secure?: boolean;
    /** Frame extents around the content, in logical pixels. */
    readonly frameInset?: FrameInset;
    /** Current workspace, or null when the window has none. */
    readonly workspace: {
        readonly active: boolean;
    } | null;
    /** Saved floating rectangle, in logical pixels. */
    floatingBounds?: Rect | null;
}
/** What the mode needs of the shell around it. */
export interface RepositionWorld {
    /** The area the window may move in: the work area of the output it is on. */
    workArea(client: RepositionClient): Rect | null;
    /** The seat's `drag_constraint`: how far a window may leave the area. */
    dragConstraint(): unknown;
    /** Gives the keyboard to the window. */
    focus(client: RepositionClient): void;
}
/** The node that holds the keyboard while the mode runs. It draws one line: what the keys do. */
export declare class RepositionGrab extends Box {
    #private;
    /** Whether arrow keys move or resize the window. */
    readonly mode: RepositionMode;
    /** Window being moved or resized. */
    readonly client: RepositionClient;
    /** Original rectangle, restored when the mode is cancelled. */
    readonly from: Rect;
    /** Previous snap state, restored when the mode is cancelled. */
    readonly snapped: SnapRecord | null;
    constructor(owner: Repositioning, client: RepositionClient, mode: RepositionMode, from: Rect, snapped: SnapRecord | null);
    /** One arrow key: the window moves, or grows or shrinks, by `dx`, `dy`. */
    step(dx: number, dy: number, world: RepositionWorld): void;
    /** Handles the repositioning mode's arrow, accept and cancel keys. */
    onKey(event: UiKeyEvent): void;
}
/** The keyboard move and resize mode of the shell: at most one window is repositioned at a time. */
export declare class Repositioning {
    #private;
    /** Starts watching for the window going and the session locking. A shell does this once. */
    start(world: RepositionWorld): void;
    /** The window being repositioned, or `null`. */
    get current(): RepositionClient | null;
    /** The node that holds the keyboard while the mode runs, or `null`. */
    get grab(): RepositionGrab | null;
    /**
     * Starts the mode for `client`. False for a window that is fullscreen, maximised, tiled, dead or not showing on its
     * output; a snapped one is unsnapped first.
     */
    begin(client: RepositionClient, mode: RepositionMode): boolean;
    /** One arrow key of the running mode. */
    step(grab: RepositionGrab, dx: number, dy: number): void;
    /**
     * Ends the mode. `keep` keeps where the window is; otherwise it goes back to where the mode found it, snapped again
     * if it was. False when no mode was running.
     */
    finish(keep: boolean): boolean;
}
/** The mode of the stock shell. Started by the module that owns the clients. */
export declare const repositioning: Repositioning;
