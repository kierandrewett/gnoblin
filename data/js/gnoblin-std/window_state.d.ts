import type { Rect } from "gnoblin-core/bindings/types";
import type { FrameInset } from "gnoblin-core/inset";
import type { UiRequest } from "gnoblin-std/ui/frame";
/** Every request the compositor makes of a window. `menu` is answered by the window menu (`gnoblin/menus`). */
export type ClientRequestKind = "maximize" | "unmaximize" | "fullscreen" | "unfullscreen" | "minimize" | "unminimize" | "close" | "raise" | "lower" | "activate" | "move" | "resize" | "menu";
/** Every `ClientRequestKind`, in the order the type lists them. Frozen, so a caller cannot widen the set. */
export declare const CLIENT_REQUEST_KINDS: readonly ClientRequestKind[];
/** How near two sides of a window a resize request's pointer must be for the corner between them to move. */
export declare const RESIZE_CORNER_ZONE = 16;
/**
 * A drag a request started ends after this long even when no button edge arrives, so it cannot follow the pointer
 * indefinitely.
 */
export declare const REQUEST_DRAG_LIMIT_MS = 30000;
/**
 * A request a client made of its window. A listener answers it by calling `handle()` (see the precedence in the header
 * of this module); a listener that does not leaves it to the next one and then to the built-in answer.
 */
export declare class ClientRequest<C = unknown> {
    #private;
    /** Kind of compositor request. */
    readonly kind: ClientRequestKind;
    /** Window that received the request. */
    readonly client: C;
    /** Where a `menu` request is anchored, in the compositor's logical coordinates; 0 for every other kind. */
    readonly x: number;
    /** Vertical position of the menu request anchor in logical pixels. */
    readonly y: number;
    constructor(kind: ClientRequestKind, client: C, x?: number, y?: number);
    /** Whether a listener has answered the request. */
    get handled(): boolean;
    /** Takes the request: no later listener hears it and the built-in answer does not run. */
    handle(): void;
}
/** What the policy reads and writes of a window. The host's `Client` satisfies it. */
export interface StateClient {
    /** Whether the window still exists. */
    readonly alive: boolean;
    /** Whether the window covers its output. */
    fullscreen: boolean;
    /** Whether the window fills its work area. */
    maximised: boolean;
    /** Whether the window is hidden. */
    minimised: boolean;
    /** A window a layout tiles is placed by the layout, so a move or resize request leaves it alone. */
    readonly tiled: boolean;
    /** Window rectangle in logical pixels. */
    bounds: Rect;
    /** The rectangle a snap restores for a maximised window; kept current here so a snap after a maximise is right. */
    floatingBounds?: Rect | null;
    /** Frame inset, when the window has a frame. */
    readonly frameInset?: FrameInset;
    /** Minimum content size in pixels, when set. */
    minSize?: {
        width: number;
        height: number;
    };
    /** Maximum content size in pixels, when set. */
    maxSize?: {
        width: number;
        height: number;
    };
}
/** What the policy needs of the shell around the windows. */
export interface WindowStateWorld<C extends StateClient> {
    /** Finds a window by its compositor node id. */
    clientOf(node: number): C | undefined;
    /** The windows of the workspace `client` is on, back to front. */
    siblings(client: C): readonly C[];
    /** Puts the window last among its siblings and keeps a fullscreen sibling over it. */
    raise(client: C): void;
    /** Lowers the window among its siblings. */
    lower(client: C): void;
    /** Closes the window. */
    close(client: C): void;
    /** The window with the keyboard focus, or null. */
    focused(): C | null;
    /** Gives the keyboard to `client`, or to nobody. A tree with no seat has nobody to give it to. */
    focus(client: C | null): void;
    /** Whether the window's workspace is the one on show. */
    onActiveWorkspace(client: C): boolean;
    /** The whole output `client` is on: what a fullscreen window covers. Null while the tree has no screen. */
    output(client?: C): Rect | null;
    /** The output `client` is on less what the bars take: what a maximised window fills. */
    workArea(client?: C): Rect | null;
    /** Converts a frame rectangle to the content rectangle by applying the frame inset. */
    framed(client: C, frame: Rect): Rect;
    /** Whether the window has a snap position. */
    snapped(client: C): boolean;
    /** Removes the window's snap position. */
    unsnap(client: C): void;
    /** Calls the `onRequest` of the client's listeners, in order, until one handles `request`. */
    dispatch(client: C, request: ClientRequest<C>): void;
    /** Reports an error from a window request. */
    report(site: string, error: unknown): void;
    /** Returns the shell clock reading in milliseconds. */
    now(): number;
    /** Returns the current pointer drag constraint, if any. */
    dragConstraint(): string | undefined;
    /** Whether edge tiling is enabled. */
    edgeTiling(): boolean;
}
/** Whether a listener handled the compositor's request (`handle()`), as the window menu asks before it opens. */
export declare function isRequestHandled(request: UiRequest): boolean;
/** The rectangles a window goes back to when it stops being maximised or fullscreen. */
export interface KeptRects {
    /** Floating rectangle restored when maximising ends, or `null` if none was saved. */
    floating: Rect | null;
    /** Windowed rectangle restored when fullscreen ends, or `null` if none was saved. */
    windowed: Rect | null;
}
/**
 * The stock answers to a window's requests, and the drags that `move` and `resize` start.
 *
 * - `maximize` fills the work area (the area left by bars) and keeps the floating rectangle. A snapped window is
 *   unsnapped first. Refused if already maximised or fullscreen.
 * - `fullscreen` covers the whole output (bars hide through `barsGiveWayTo` in gnoblin-std/widgets), keeps the windowed
 *   rectangle and puts the window last among its siblings. Refused if already fullscreen.
 * - `minimize` hides the window and, if it held focus, focuses the topmost visible sibling or nobody. `unminimize`
 *   shows it and focuses it only when nothing else has focus.
 * - `activate` presents a window only on the active workspace: unminimise, focus and raise. Other workspaces are
 *   unchanged.
 * - `raise` puts the window last among its siblings, below any fullscreen sibling (`Client.raise`).
 * - `move` and `resize` start the pointer drags used by `ClientMoveBinding` and `ClientResizeBinding`. Tiled,
 *   fullscreen and maximised windows are not dragged. The pointer chooses the resize edge or corner.
 */
export declare class WindowState {
    #private;
    /**
     * The rectangles `client` returns to when it leaves maximised or fullscreen. They live here, in the shell's script,
     * so a reload takes them with it unless the host carries them across: it reads this before it goes and `keep`s it
     * in the next.
     */
    kept(client: StateClient): KeptRects;
    /** Gives `client` back the rectangles `kept` took, so it can leave maximised or fullscreen as it would have. */
    keep(client: StateClient, kept: Partial<KeptRects>): void;
    /** Whether the policy answers the compositor's requests. */
    get started(): boolean;
    /** Starts answering requests. A shell does this once, before the window menu starts. */
    start<C extends StateClient>(world: WindowStateWorld<C>): void;
    /** Answers `kind` for `client` as the built-in policy does. A listener that handled a request may call it. */
    answer(client: StateClient, kind: ClientRequestKind): void;
    /** Fills the work area and keeps the floating rectangle. Refused (false) when maximised or fullscreen already. */
    maximise(client: StateClient): boolean;
    /** Leaves maximised and returns the floating rectangle. Refused (false) when it is not maximised. */
    unmaximise(client: StateClient): boolean;
    /**
     * Covers the whole output, bars included, and goes last among its siblings. The rectangle it had is kept, so
     * leaving fullscreen returns a floating, snapped or maximised window to it. Refused (false) when it is already
     * fullscreen.
     */
    enterFullscreen(client: StateClient): boolean;
    /** Returns the rectangle the window had before it went fullscreen. Refused (false) when it is not fullscreen. */
    leaveFullscreen(client: StateClient): boolean;
    /**
     * Hides the window and hands the keyboard focus on, as the window manager did: the topmost sibling still showing,
     * or nobody. A window that did not hold the focus leaves it where it is. Refused (false) when minimised already.
     */
    minimise(client: StateClient): boolean;
    /**
     * Shows the window again. The keyboard goes to it only when nothing holds it. Refused (false) when not minimised.
     */
    unminimise(client: StateClient): boolean;
    /**
     * An activation: raises and focuses a window on the active workspace, and brings a minimised one back. A window on
     * a workspace that is not showing is left alone. Returns whether it ran.
     */
    present(client: StateClient): boolean;
    /** Raises the window among its siblings. */
    raise(client: StateClient): void;
    /** Lowers the window among its siblings. */
    lower(client: StateClient): void;
    /** Starts the stock move drag for `client`, following the seat's pointer. False when none starts. */
    beginMove(client: StateClient): boolean;
    /** Starts the stock resize drag for `client`. The corner or side is chosen from where the pointer is. */
    beginResize(client: StateClient): boolean;
    /** Whether a drag a request started is following the pointer. */
    get dragging(): boolean;
    /**
     * The shell tells the policy of every pointer button edge on a window. A press or a release ends the drag a request
     * started, after one last step to where the pointer is.
     */
    pointerEdge(kind: string): void;
}
/** The stock policy. The shell starts it; a script calls its methods to answer a request it handled. */
export declare const windowState: WindowState;
