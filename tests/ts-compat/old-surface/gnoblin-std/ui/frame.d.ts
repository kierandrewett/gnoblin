/** Marks a compositor node and all canvases in its subtree as gone. */
export declare function uiNodeGone(node: number): void;
/** Destroys all canvases made in the current session and clears pending measurements. */
export declare function uiReleaseAll(): void;
/** Returns the number of live canvases. Intended for tests and diagnostics. */
export declare function uiCanvasCount(): number;
/** A compositor request for a window, such as a menu, with its anchor in logical screen coordinates. */
export interface UiRequest {
    /** The window's node. */
    readonly node: number;
    /** The request kind, such as `menu`. */
    readonly request: string;
    /** Horizontal anchor position in logical screen pixels. */
    readonly x: number;
    /** Vertical anchor position in logical screen pixels. */
    readonly y: number;
}
/** Listens for compositor window requests. Returns a function that removes this listener. */
export declare function onUiRequest(handler: (request: UiRequest) => void): () => void;
/** Runs `hook` after event delivery and before canvas updates on each frame. Returns a function that removes it. */
export declare function onUiFrame(hook: () => void): () => void;
/**
 * One frame of the toolkit, called by the host after the layouts. In order: the measurements and pointer or key
 * events the compositor sent are delivered; the frame hooks run; then each root canvas updates its whole tree.
 * Does nothing until a host is attached.
 */
export declare function runUiFrame(): void;
