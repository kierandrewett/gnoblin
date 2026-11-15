// `Canvas`, the node of the compositor's tree whose content is a paint list: its fields, its children, the frame that
// writes what changed to the host and the delivery of pointer and key events up through its parents.

import { disposeBindingsOf, propertiesOf } from "gnoblin/properties";
import type { PropertyBindings } from "gnoblin/properties";
import { HANDLERS } from "gnoblin/ui/events";
import type { UiKeyEvent, UiPointerEvent } from "gnoblin/ui/events";
import { encodePaintList } from "gnoblin/ui/paint";
import type { PaintOp } from "gnoblin/ui/paint";
import { host, live, refuse, reportError } from "gnoblin/ui/state";

/**
 * Where a canvas that is a child of a window sits: `frame` (the default) in front of the window's content and part of
 * the window's frame, so rounded corners, the border and a frame shadow follow it; `over` in front and not part of the
 * frame; `under` painted behind the window's content and not part of the frame. Under any other parent a canvas is
 * painted in tree order and `placement` only says whether it counts as part of a frame.
 */
export type CanvasPlacement = "frame" | "over" | "under";

/** Options for `Canvas`. Each field given is set before the first frame. Sizes are pixels. */
export interface CanvasOptions {
    x?: number;
    y?: number;
    width?: number;
    height?: number;
    visible?: boolean;
    opacity?: number;
    /** Never captured by a `View`: the window it belongs to is secure. */
    secure?: boolean;
    /** Whether the pointer lands on the canvas. A canvas with `input` false lets the pointer through to what is under it. */
    input?: boolean;
    placement?: CanvasPlacement;
}

/** The node properties a canvas keeps in step with its fields, and the value the tree starts them at. */
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
 * A node that paints what `draw()` returns. Override `draw()` for a custom look; use a `Box` for the common one.
 * A canvas is made with its node, so it exists in the tree from the next transaction; `destroy()` ends it.
 */
export class Canvas {
    /** The node's id in the tree. */
    readonly node: number;
    /** The canvas this one is a child of, or `null` for the root of a tree (or one mounted on a window). */
    parent: Canvas | null = null;
    readonly children: Canvas[] = [];
    x = 0;
    y = 0;
    visible = true;
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
     * The reactive properties of the canvas: `canvas.properties.bind("visible", () => !client.focused)` sets the field
     * now and again whenever a client or node property the function read changes, with no script running. The bindings
     * end with the canvas.
     */
    get properties(): PropertyBindings {
        return propertiesOf(this, () => `${this.constructor.name}#${this.node}`);
    }

    /** The width in pixels: what was assigned, else the size the content naturally has. `null` goes back to natural. */
    get width(): number {
        return this.#width ?? this.naturalWidth();
    }

    set width(value: number | null) {
        this.#width = value;
    }

    get height(): number {
        return this.#height ?? this.naturalHeight();
    }

    set height(value: number | null) {
        this.#height = value;
    }

    /** True until the canvas is destroyed, or the compositor destroyed its node. */
    get alive(): boolean {
        return this.#alive;
    }

    /** The width the canvas has when none was assigned. A subclass whose content has a size answers it. */
    protected naturalWidth(): number {
        return 0;
    }

    protected naturalHeight(): number {
        return 0;
    }

    /** The ops this canvas paints. Called once a frame, so it may read live state. */
    draw(): PaintOp[] {
        return [];
    }

    /** Called once a frame before the canvas arranges its children: read live state into fields here. */
    refresh(): void {}

    /** Places the children. The base class leaves them where they are. */
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

    /** Takes `child` out of this canvas and destroys it. */
    remove(child: Canvas): void {
        if (child.parent !== this) {
            throw new RangeError("that canvas is not a child of this one");
        }
        child.destroy();
    }

    /** Makes this canvas the last child of a node that is not a canvas (a client's window, say). */
    mount(parentNode: number): void {
        this.#check("mount");
        this.parent?.children.splice(this.parent.children.indexOf(this), 1);
        this.parent = null;
        refuse(host().insert(this.node, parentNode, -1), "mount");
    }

    /**
     * Makes this canvas the last child of the root, over every window: a menu, a dialog or a bar. The compositor's
     * windows have no parent until the shell gives them one, so the host parents them to the root first, in the order
     * they have, and a canvas put there is above them.
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

    /** Whether `other` is this canvas or lies beneath it. */
    contains(other: Canvas | null): boolean {
        for (let at = other; at !== null; at = at.parent) {
            if (at === this) {
                return true;
            }
        }
        return false;
    }

    /** Destroys this canvas and every canvas beneath it, in the tree and here. */
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

    /** The node is gone from the tree (the compositor destroyed it, or its parent's subtree). */
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
     * One frame of this canvas and its children: refresh, arrange, update the children, then write what changed. The
     * order matters: `arrange()` sets the children's geometry before they update, and this canvas's own paint is
     * written last, once every size is final.
     */
    update(): void {
        if (!this.#alive) {
            return;
        }
        guard(this, "refresh", () => this.refresh());
        guard(this, "arrange", () => this.arrange());
        for (const child of [...this.children]) {
            child.update();
        }
        guard(this, "draw", () => this.#flush());
    }

    /** Writes the node properties that changed, then the paint list if its bytes changed. */
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
export interface Canvas {
    onPress?(event: UiPointerEvent): void;
    onRelease?(event: UiPointerEvent): void;
    onMotion?(event: UiPointerEvent): void;
    onScroll?(event: UiPointerEvent): void;
    onEnter?(event: UiPointerEvent): void;
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
