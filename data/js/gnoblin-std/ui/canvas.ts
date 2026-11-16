// A canvas is a compositor node that paints a list of drawing operations and can contain other canvases.

import { disposeBindingsOf, propertiesOf } from "gnoblin-core/properties";
import type { PropertyBindings } from "gnoblin-core/properties";
import { HANDLERS } from "gnoblin-std/ui/events";
import type { UiKeyEvent, UiPointerEvent } from "gnoblin-std/ui/events";
import { encodePaintList } from "gnoblin-std/ui/paint";
import type { PaintOp } from "gnoblin-std/ui/paint";
import { host, live, refuse, reportError } from "gnoblin-std/ui/state";

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

/** The last node properties written to the compositor, or their initial values. */
interface Written {
    x: number;
    y: number;
    width: number;
    height: number;
    visible: boolean;
    opacity: number;
    secure: boolean;
    input: boolean;
    placement: CanvasPlacement;
}

/**
 * A canvas paints the operations returned by `draw()`. Override it for a custom look, or use a `Box` for a background
 * and common layouts. Its compositor node is created with the canvas and destroyed by `destroy()`.
 * Construction throws if no host is attached or the shell cannot create a node.
 */
export class Canvas {
    /** The node's id in the tree. */
    readonly node: number;
    /** The canvas this one is a child of, or `null` for the root of a tree (or one mounted on a window). */
    parent: Canvas | null = null;
    /** Child canvases in paint and event order. */
    readonly children: Canvas[] = [];
    /** Horizontal position in logical pixels relative to the parent. */
    x = 0;
    /** Vertical position in logical pixels relative to the parent. */
    y = 0;
    /** Whether the canvas is painted and receives pointer input. */
    visible = true;
    /** Opacity from `0` (transparent) to `1` (opaque). */
    opacity = 1;
    /** Whether a capture of the screen masks this canvas. Set it on what shows a secure window's text. */
    secure = false;
    /**
     * Whether the pointer lands on this canvas. With `false` it goes through to what is under the canvas, as it does
     * for the empty layer of a frame; the canvas's children are still tested.
     */
    input = true;
    /** Where the canvas sits among a window's content and frame: see `CanvasPlacement`. */
    placement: CanvasPlacement = "frame";
    #width: number | null = null;
    #height: number | null = null;
    #alive = true;
    #written: Written = {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        visible: true,
        opacity: 1,
        secure: false,
        input: true,
        placement: "frame",
    };
    // The bytes last written. A frame skips the paint when the new bytes are the same.
    #content: number[] | null = null;
    // How many steps of the update pass of this canvas have failed.
    #failures = 0;

    constructor(options: CanvasOptions = {}) {
        const node = host().create();
        if (node === undefined) {
            throw new Error("the shell could not make a canvas node");
        }
        this.node = node;
        live.set(node, this);
        if (options.x !== undefined) this.x = options.x;
        if (options.y !== undefined) this.y = options.y;
        if (options.visible !== undefined) this.visible = options.visible;
        if (options.opacity !== undefined) this.opacity = options.opacity;
        if (options.secure !== undefined) this.secure = options.secure;
        if (options.input !== undefined) this.input = options.input;
        if (options.placement !== undefined) this.placement = options.placement;
        if (options.width !== undefined) this.#width = options.width;
        if (options.height !== undefined) this.#height = options.height;
    }

    /**
     * Reactive bindings update canvas fields when the properties they read change. For example,
     * `canvas.properties.bind("visible", () => !client.focused)` follows the client's focus without polling.
     * Bindings end when the canvas is destroyed.
     */
    get properties(): PropertyBindings {
        return propertiesOf(this, () => `${this.constructor.name}#${this.node}`);
    }

    /** The width in logical pixels: the assigned width, or the content's natural width. `null` clears it. */
    get width(): number {
        return this.#width ?? this.naturalWidth();
    }

    /** Sets an explicit width in logical pixels, or `null` to use the natural width. */
    set width(value: number | null) {
        this.#width = value;
    }

    /** The height in logical pixels: the assigned height, or the content's natural height. `null` clears it. */
    get height(): number {
        return this.#height ?? this.naturalHeight();
    }

    /** Sets an explicit height in logical pixels, or `null` to use the natural height. */
    set height(value: number | null) {
        this.#height = value;
    }

    /** Whether the canvas still exists in the compositor tree. */
    get alive(): boolean {
        return this.#alive;
    }

    /**
     * How many steps of the update pass (refresh, arrange, draw) have failed for this canvas and for every canvas under
     * it. A draw fails when `draw()` throws or when the host refuses what it writes. A screen that has to prove it is
     * alive (the lock screen) reads this before and after a frame: a count that grew is a frame that did not draw.
     * Handlers do not count: they act on one event and report their own failure.
     */
    get failures(): number {
        let total = this.#failures;
        for (const child of this.children) {
            total += child.failures;
        }
        return total;
    }

    /** The natural width in logical pixels. Subclasses can override this when their content has a natural size. */
    protected naturalWidth(): number {
        return 0;
    }

    /** The natural height in logical pixels. Subclasses can override this when their content has a natural size. */
    protected naturalHeight(): number {
        return 0;
    }

    /** The operations this canvas paints. Called once per frame and may read live state. */
    draw(): PaintOp[] {
        return [];
    }

    /** Updates fields from live state before the canvas arranges its children. */
    refresh(): void {}

    /** Arranges the children. The base implementation leaves their positions unchanged. */
    arrange(): void {}

    /** Puts `child` last among this canvas's children. A child that was somewhere else is moved. */
    add(child: Canvas): this {
        this.#check("add");
        child.#check("add");
        if (child === this || child.contains(this)) {
            throw new RangeError("a canvas cannot be its own ancestor");
        }
        child.parent?.children.splice(child.parent.children.indexOf(child), 1);
        child.parent = this;
        this.children.push(child);
        refuse(host().insert(child.node, this.node, -1), "add");
        return this;
    }

    /** Removes and destroys `child`, including its descendants. */
    remove(child: Canvas): void {
        if (child.parent !== this) {
            throw new RangeError("that canvas is not a child of this one");
        }
        child.destroy();
    }

    /** Mounts this canvas as the last child of a non-canvas node, such as a client's window. */
    mount(parentNode: number): void {
        this.#check("mount");
        this.parent?.children.splice(this.parent.children.indexOf(this), 1);
        this.parent = null;
        refuse(host().insert(this.node, parentNode, -1), "mount");
    }

    /**
     * Mounts this canvas at the root, above every window, for overlays such as menus, dialogs or bars.
     * Windows are parented to the root first, so a canvas mounted here appears above them.
     */
    mountTop(): void {
        this.#check("mountTop");
        this.parent?.children.splice(this.parent.children.indexOf(this), 1);
        this.parent = null;
        refuse(host().top(this.node), "mountTop");
    }

    /** Gives this canvas the seat's keyboard focus: the keys go to `onKey` here and up through its parents. */
    focus(): void {
        this.#check("focus");
        refuse(host().focus(this.node), "focus");
    }

    /** Whether this canvas holds the keyboard focus. */
    get focused(): boolean {
        return this.#alive && host().focused() === this.node;
    }

    /** Whether `other` is this canvas or one of its descendants. */
    contains(other: Canvas | null): boolean {
        for (let at = other; at !== null; at = at.parent) {
            if (at === this) {
                return true;
            }
        }
        return false;
    }

    /** Destroys this canvas and all its descendants. */
    destroy(): void {
        if (!this.#alive) {
            return;
        }
        this.parent?.children.splice(this.parent.children.indexOf(this), 1);
        this.parent = null;
        const code = host().destroy(this.node);
        if (code !== undefined) {
            reportError("Canvas.destroy", new Error(`the host refused to destroy node ${this.node} (${code})`));
        }
        this.#bury();
    }

    /** Marks this canvas gone after the compositor destroys its node or its parent's subtree. */
    markGone(): void {
        if (this.#alive) {
            this.parent?.children.splice(this.parent.children.indexOf(this), 1);
            this.parent = null;
            this.#bury();
        }
    }

    #bury(): void {
        this.#alive = false;
        disposeBindingsOf(this);
        live.delete(this.node);
        for (const child of [...this.children]) {
            child.parent = null;
            child.#bury();
        }
        this.children.length = 0;
    }

    #check(what: string): void {
        if (!this.#alive) {
            throw new Error(`Canvas.${what}: the canvas is destroyed`);
        }
    }

    /**
     * Updates this canvas and its descendants: refreshes fields, arranges children, updates them, then writes changes.
     * Children are arranged before they update, and this canvas paints after their sizes are final.
     */
    update(): void {
        if (!this.#alive) {
            return;
        }
        this.#step("refresh", () => this.refresh());
        this.#step("arrange", () => this.arrange());
        for (const child of [...this.children]) {
            child.update();
        }
        this.#step("draw", () => this.#flush());
    }

    /** Runs one step of the update pass. A failure is reported and counted, and the pass goes on. */
    #step(what: string, body: () => void): void {
        try {
            body();
        } catch (error) {
            this.#failures += 1;
            reportError(`${this.constructor.name}.${what}`, error);
        }
    }

    /** Writes changed node properties and sends the paint list only when its bytes differ. */
    #flush(): void {
        const written = this.#written;
        const now = {
            x: this.x,
            y: this.y,
            width: this.width,
            height: this.height,
            visible: this.visible,
            opacity: this.opacity,
            secure: this.secure,
            input: this.input,
            placement: this.placement,
        };
        for (const key of ["x", "y", "width", "height", "opacity"] as const) {
            if (now[key] !== written[key]) {
                this.#write(key, now[key]);
                written[key] = now[key];
            }
        }
        if (now.visible !== written.visible) {
            this.#write("visible", now.visible);
            written.visible = now.visible;
        }
        if (now.secure !== written.secure) {
            this.#write("secure", now.secure);
            written.secure = now.secure;
        }
        if (now.input !== written.input) {
            this.#write("input", now.input);
            written.input = now.input;
        }
        if (now.placement !== written.placement) {
            this.#write("placement", now.placement);
            written.placement = now.placement;
        }
        const bytes = encodePaintList(this.draw());
        if (this.#content === null || !sameBytes(this.#content, bytes)) {
            const code = host().paint(this.node, bytes);
            // The compositor destroyed the node, so the canvas is gone too. Its writes are no longer wanted.
            if (code === "unknown-node") {
                this.markGone();
                return;
            }
            if (code !== undefined) {
                throw new Error(`the host refused the paint list (${code})`);
            }
            this.#content = bytes;
        }
    }

    #write(key: string, value: number | boolean | string): void {
        const code = host().set(this.node, key, value);
        if (code === "unknown-node") {
            this.markGone();
        } else if (code !== undefined) {
            throw new Error(`the host refused ${key} (${code})`);
        }
    }

    /** Sends a pointer event to this canvas and then up its parents until a handler stops it. */
    dispatch(event: UiPointerEvent): void {
        const name = HANDLERS[event.kind];
        // eslint-disable-next-line @typescript-eslint/no-this-alias -- `at` is the mutable parent-walk cursor.
        for (let at: Canvas | null = this; at !== null && !event.stopped; at = at.parent) {
            const handler = name === undefined ? undefined : at[name];
            if (typeof handler === "function") {
                guard(at, name as string, () => handler.call(at, event));
            }
            // Each parent's offset is added after its handler runs, so the next handler sees the event in its
            // own space.
            event.x += at.x;
            event.y += at.y;
        }
    }

    /** Sends a key to this canvas and then up its parents until a handler stops it. */
    dispatchKey(event: UiKeyEvent): void {
        // eslint-disable-next-line @typescript-eslint/no-this-alias -- `at` is the mutable parent-walk cursor.
        for (let at: Canvas | null = this; at !== null && !event.stopped; at = at.parent) {
            const handler = at.onKey;
            if (typeof handler === "function") {
                guard(at, "onKey", () => handler.call(at, event));
            }
        }
    }
}

// Pointer handlers live on the interface, not the class: a class field would define an own `undefined` property in
// the base constructor and hide the method a subclass declares.
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

function guard(canvas: Canvas, what: string, body: () => void): void {
    try {
        body();
    } catch (error) {
        reportError(`${canvas.constructor.name}.${what}`, error);
    }
}

function sameBytes(a: readonly number[], b: readonly number[]): boolean {
    if (a.length !== b.length) {
        return false;
    }
    for (let i = 0; i < a.length; i++) {
        if (a[i] !== b[i]) {
            return false;
        }
    }
    return true;
}
