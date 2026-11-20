import type { PropertyBindings } from "gnoblin-core/properties";
import type { UiKeyEvent, UiPointerEvent } from "gnoblin-std/ui/events";
import type { PaintOp } from "gnoblin-std/ui/paint";
/**
 * Where a canvas that is a child of a window sits: `frame` (the default) in front of the window's content and part of
 * the window's frame, so rounded corners, the border and a frame shadow follow it; `over` in front and not part of the
 * frame; `under` painted behind the window's content and not part of the frame. Under any other parent a canvas is
 * painted in tree order and `placement` only says whether it counts as part of a frame.
 */
export type CanvasPlacement = "frame" | "over" | "under";
/** Options for `Canvas`. Positions and sizes are logical pixels; each field given is set before the first frame. */
export interface CanvasOptions {
    /** Horizontal position in logical pixels; defaults to `0`. */
    x?: number;
    /** Vertical position in logical pixels; defaults to `0`. */
    y?: number;
    /** Width in logical pixels; defaults to the content's natural width. */
    width?: number;
    /** Height in logical pixels; defaults to the content's natural height. */
    height?: number;
    /** Whether the canvas is visible; defaults to `true`. */
    visible?: boolean;
    /** Opacity from `0` (transparent) to `1` (opaque); defaults to `1`. */
    opacity?: number;
    /** Masks this canvas in screen captures. Use it for content belonging to a secure window. */
    secure?: boolean;
    /** Whether the pointer targets this canvas. When false, it passes through while children remain interactive. */
    input?: boolean;
    /** Placement relative to a window's content and frame; defaults to `frame`. */
    placement?: CanvasPlacement;
}
/**
 * A canvas paints the operations returned by `draw()`. Override it for a custom look, or use a `Box` for a background
 * and common layouts. Its compositor node is created with the canvas and destroyed by `destroy()`.
 * Construction throws if no host is attached or the shell cannot create a node.
 */
export declare class Canvas {
    #private;
    /** The node's id in the tree. */
    readonly node: number;
    /** The canvas this one is a child of, or `null` for the root of a tree (or one mounted on a window). */
    parent: Canvas | null;
    /** Child canvases in paint and event order. */
    readonly children: Canvas[];
    /** Horizontal position in logical pixels relative to the parent. */
    x: number;
    /** Vertical position in logical pixels relative to the parent. */
    y: number;
    /** Whether the canvas is painted and receives pointer input. */
    visible: boolean;
    /** Opacity from `0` (transparent) to `1` (opaque). */
    opacity: number;
    /** Whether a capture of the screen masks this canvas. Set it on what shows a secure window's text. */
    secure: boolean;
    /**
     * Whether the pointer lands on this canvas. With `false` it goes through to what is under the canvas, as it does
     * for the empty layer of a frame; the canvas's children are still tested.
     */
    input: boolean;
    /** Where the canvas sits among a window's content and frame: see `CanvasPlacement`. */
    placement: CanvasPlacement;
    constructor(options?: CanvasOptions);
    /**
     * Reactive bindings update canvas fields when the properties they read change. For example,
     * `canvas.properties.bind("visible", () => !client.focused)` follows the client's focus without polling.
     * Bindings end when the canvas is destroyed.
     */
    get properties(): PropertyBindings;
    /** The width in logical pixels: the assigned width, or the content's natural width. `null` clears it. */
    get width(): number;
    /** Sets an explicit width in logical pixels, or `null` to use the natural width. */
    set width(value: number | null);
    /** The height in logical pixels: the assigned height, or the content's natural height. `null` clears it. */
    get height(): number;
    /** Sets an explicit height in logical pixels, or `null` to use the natural height. */
    set height(value: number | null);
    /** Whether the canvas still exists in the compositor tree. */
    get alive(): boolean;
    /**
     * How many steps of the update pass (refresh, arrange, draw) have failed for this canvas and for every canvas under
     * it. A draw fails when `draw()` throws or when the host refuses what it writes. A screen that has to prove it is
     * alive (the lock screen) reads this before and after a frame: a count that grew is a frame that did not draw.
     * Handlers do not count: they act on one event and report their own failure.
     */
    get failures(): number;
    /** The natural width in logical pixels. Subclasses can override this when their content has a natural size. */
    protected naturalWidth(): number;
    /** The natural height in logical pixels. Subclasses can override this when their content has a natural size. */
    protected naturalHeight(): number;
    /** The operations this canvas paints. Called once per frame and may read live state. */
    draw(): PaintOp[];
    /** Updates fields from live state before the canvas arranges its children. */
    refresh(): void;
    /** Arranges the children. The base implementation leaves their positions unchanged. */
    arrange(): void;
    /** Puts `child` last among this canvas's children. A child that was somewhere else is moved. */
    add(child: Canvas): this;
    /** Removes and destroys `child`, including its descendants. */
    remove(child: Canvas): void;
    /** Mounts this canvas as the last child of a non-canvas node, such as a client's window. */
    mount(parentNode: number): void;
    /**
     * Mounts this canvas at the root, above every window, for overlays such as menus, dialogs or bars.
     * Windows are parented to the root first, so a canvas mounted here appears above them.
     */
    mountTop(): void;
    /** Gives this canvas the seat's keyboard focus: the keys go to `onKey` here and up through its parents. */
    focus(): void;
    /** Whether this canvas holds the keyboard focus. */
    get focused(): boolean;
    /** Whether `other` is this canvas or one of its descendants. */
    contains(other: Canvas | null): boolean;
    /** Destroys this canvas and all its descendants. */
    destroy(): void;
    /** Marks this canvas gone after the compositor destroys its node or its parent's subtree. */
    markGone(): void;
    /**
     * Updates this canvas and its descendants: refreshes fields, arranges children, updates them, then writes changes.
     * Children are arranged before they update, and this canvas paints after their sizes are final.
     */
    update(): void;
    /** Sends a pointer event to this canvas and then up its parents until a handler stops it. */
    dispatch(event: UiPointerEvent): void;
    /** Sends a key to this canvas and then up its parents until a handler stops it. */
    dispatchKey(event: UiKeyEvent): void;
}
/** Pointer and keyboard handlers that a canvas may implement. */
export interface Canvas {
    /** Handles a pointer press. */
    onPress?(event: UiPointerEvent): void;
    /** Handles a pointer release. */
    onRelease?(event: UiPointerEvent): void;
    /** Handles pointer movement. */
    onMotion?(event: UiPointerEvent): void;
    /** Handles pointer scrolling. */
    onScroll?(event: UiPointerEvent): void;
    /** Handles the pointer entering the canvas. */
    onEnter?(event: UiPointerEvent): void;
    /** Handles the pointer leaving the canvas. */
    onLeave?(event: UiPointerEvent): void;
    /** A key, to the canvas that has the keyboard focus. Pressed and released both arrive: see `event.pressed`. */
    onKey?(event: UiKeyEvent): void;
}
