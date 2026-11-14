// The shell UI toolkit: `Canvas`, `Box`, `Text`, `Icon` and `Grid`, the classes the stock shell draws with.
//
// A `Canvas` is a node of the compositor's tree whose content is a paint list (crates/gnoblin-paint/src/list.rs):
// the rectangles, lines, text and icons its `draw()` returns. A `Box` lays out its children; `Text` and `Icon` draw
// one string or one icon. Fields are plain: change one and the next frame paints it.
//
// The toolkit talks to the tree only through `UiBackend`, which the host hands over with `attachUi`. It never
// imports the host, so a test can attach a fake. Frames are driven by `runUiFrame`, see its doc.

// ---- The host ----------------------------------------------------------------------------------------------------

/** The functions the shell host provides. Each refusal is a short code (`unknown-node`, `not-owned`, ...). */
export interface UiBackend {
    /** Makes a canvas node in the tree and returns its id, or `undefined` when the shell cannot make one. */
    create(): number | undefined;
    insert(node: number, parent: number, index: number): string | undefined;
    destroy(node: number): string | undefined;
    set(node: number, key: string, value: number | boolean): string | undefined;
    /** Writes the encoded paint list as the node's `content`. */
    paint(node: number, bytes: number[]): string | undefined;
    /** Asks the compositor how much room `text` takes. The answer comes back from `takeAnswers` under the token. */
    measure(node: number, font: string, size: number, text: string): number | undefined;
    takeAnswers(): UiAnswer[];
    takeEvents(): UiRawEvent[];
    /** Gives the seat's keyboard focus to a canvas or a window (`0` takes it away). A refusal is a short code. */
    focus(node: number): string | undefined;
    /** The node with the keyboard focus, or `undefined`. */
    focused(): number | undefined;
    /** Puts a canvas last under the root, over every window. */
    top(node: number): string | undefined;
    /** A number the seat publishes: `pointer_x`, `pointer_y`. `undefined` when the tree has no seat. */
    seatNumber(key: string): number | undefined;
    /** What a titlebar click does: the seat's `double_click`, `middle_click` or `right_click`. */
    seatText(key: string): string | undefined;
    /** Whether the session is locked or about to be. */
    locked(): boolean;
    /** The size of the primary screen, or `undefined` while the tree has none. */
    screen(): [number, number] | undefined;
    /** Milliseconds on a clock that only goes forward. */
    now(): number;
}

/** The compositor's answer to a measurement: `result` is `<width> <height> <ascent>` in whole pixels. */
export interface UiAnswer {
    token: number;
    result: string;
    error: string;
}

/**
 * What the compositor sent to the toolkit before it is routed: a pointer event that hit a canvas, a key sent to the
 * canvas that holds the keyboard focus, or a request made of a window (`request` is `menu` for its window menu, and
 * `node` is the window).
 */
export interface UiRawEvent {
    node: number;
    kind: string;
    x: number;
    y: number;
    button: number;
    dx: number;
    dy: number;
    key: string;
    text: string;
    pressed: boolean;
    request: string;
}

type Report = (site: string, error: unknown) => void;

// Set by `attachUi`. Until then making a canvas throws and `runUiFrame` does nothing.
let backend: UiBackend | null = null;
let report: Report = () => {};

/** Connects the toolkit to the shell host. `onError` receives what a canvas threw while it updated. */
export function attachUi(host: UiBackend, onError: Report): void {
    backend = host;
    report = onError;
}

function host(): UiBackend {
    if (backend === null) {
        throw new Error("the UI toolkit needs the shell host and cannot run without it");
    }
    return backend;
}

/** The canvases of this generation, by node. */
const live = new Map<number, Canvas>();

// ---- The paint list ----------------------------------------------------------------------------------------------

/** A colour is `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`. */
export type Color = string;

/** One drawing op. Later ops draw over earlier ones. The coordinates are node-local pixels. */
export type PaintOp =
    | {
          op: "rect";
          x: number;
          y: number;
          width: number;
          height: number;
          radius?: number;
          fill?: Color;
          stroke?: Color;
          strokeWidth?: number;
      }
    | { op: "line"; x1: number; y1: number; x2: number; y2: number; width?: number; color?: Color }
    | { op: "text"; text: string; x?: number; y?: number; size?: number; color?: Color; font?: string }
    | { op: "icon"; name: string; x?: number; y?: number; size?: number; color?: Color }
    | { op: "pushClip"; x: number; y: number; width: number; height: number; radius?: number }
    | { op: "popClip" }
    | { op: "pushOpacity"; alpha: number }
    | { op: "popOpacity" };

/** Defaults for ops that leave a field out. Sizes are pixels; colours are `0xRRGGBBAA` words. */
const OPAQUE_WHITE = 0xffffffff;
const DEFAULT_FONT = "sans";
const DEFAULT_TEXT_SIZE = 14;
const DEFAULT_ICON_SIZE = 16;
const DEFAULT_LINE_WIDTH = 1;
/** The colour of text when a `Text` names none. Matches the stock menus' text colour. */
const DEFAULT_TEXT_COLOUR = "#cdd6f4";

/** The paint list's format version. `crates/gnoblin-paint/src/list.rs` refuses any other. */
const PAINT_LIST_VERSION = 1;

/** The tag byte of each op kind, as `crates/gnoblin-paint/src/list.rs` reads it. */
const TAG = { rect: 1, line: 2, text: 3, icon: 4, pushClip: 5, popClip: 6, pushOpacity: 7, popOpacity: 8 } as const;

/** Little-endian bytes, as the paint list wants them. */
class ByteWriter {
    readonly bytes: number[] = [];
    readonly #scratch = new DataView(new ArrayBuffer(8));

    u8(value: number): void {
        this.bytes.push(value & 0xff);
    }

    u16(value: number): void {
        this.bytes.push(value & 0xff, (value >>> 8) & 0xff);
    }

    u32(value: number): void {
        this.bytes.push(value & 0xff, (value >>> 8) & 0xff, (value >>> 16) & 0xff, (value >>> 24) & 0xff);
    }

    f32(value: number): void {
        this.#scratch.setFloat32(0, value, true);
        for (let i = 0; i < 4; i++) {
            this.bytes.push(this.#scratch.getUint8(i));
        }
    }

    /** A string with a `u16` byte length in front. */
    str2(text: string): void {
        const bytes = utf8(text);
        if (bytes.length > 0xffff) {
            throw new RangeError("a font or icon name is over 65535 bytes");
        }
        this.u16(bytes.length);
        this.bytes.push(...bytes);
    }

    /** A string with a `u32` byte length in front. */
    str4(text: string): void {
        const bytes = utf8(text);
        this.u32(bytes.length);
        for (const byte of bytes) {
            this.bytes.push(byte);
        }
    }
}

/** The UTF-8 bytes of `text`. A lone surrogate is written as U+FFFD. */
export function utf8(text: string): number[] {
    const out: number[] = [];
    for (const char of text) {
        let code = char.codePointAt(0) as number;
        if (code >= 0xd800 && code <= 0xdfff) {
            code = 0xfffd;
        }
        if (code < 0x80) {
            out.push(code);
        } else if (code < 0x800) {
            out.push(0xc0 | (code >> 6), 0x80 | (code & 0x3f));
        } else if (code < 0x10000) {
            out.push(0xe0 | (code >> 12), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f));
        } else {
            out.push(
                0xf0 | (code >> 18),
                0x80 | ((code >> 12) & 0x3f),
                0x80 | ((code >> 6) & 0x3f),
                0x80 | (code & 0x3f),
            );
        }
    }
    return out;
}

/**
 * The `0xRRGGBBAA` word of a colour, or `fallback` when the colour is absent. Throws a `TypeError` for any other form.
 */
function colourWord(value: Color | undefined, fallback: number): number {
    if (value === undefined) {
        return fallback;
    }
    const match = typeof value === "string" ? /^#([0-9a-fA-F]+)$/.exec(value) : null;
    let hex = match === null ? "" : match[1];
    if (hex.length === 3 || hex.length === 4) {
        hex = [...hex].map((digit) => digit + digit).join("");
    }
    if (hex.length === 6) {
        hex += "ff";
    }
    if (hex.length !== 8) {
        throw new TypeError(`a colour is #rgb, #rgba, #rrggbb or #rrggbbaa, not ${String(value)}`);
    }
    return Number.parseInt(hex, 16);
}

/**
 * `value`, or `fallback` when it is absent. Throws on NaN and infinities: the compositor would read them as
 * garbage.
 */
function finite(value: number | undefined, fallback: number): number {
    if (value === undefined) {
        return fallback;
    }
    if (typeof value !== "number" || !Number.isFinite(value)) {
        throw new TypeError(`expected a finite number, not ${String(value)}`);
    }
    return value;
}

/** Writes one op: its tag byte, then its fields in the order `crates/gnoblin-paint/src/list.rs` reads them. */
function writeOp(out: ByteWriter, op: PaintOp): void {
    switch (op.op) {
        case "rect":
            out.u8(TAG.rect);
            out.f32(finite(op.x, 0));
            out.f32(finite(op.y, 0));
            out.f32(finite(op.width, 0));
            out.f32(finite(op.height, 0));
            out.f32(finite(op.radius, 0));
            out.u32(colourWord(op.fill, 0));
            out.u32(colourWord(op.stroke, 0));
            out.f32(finite(op.strokeWidth, 0));
            return;
        case "line":
            out.u8(TAG.line);
            out.f32(finite(op.x1, 0));
            out.f32(finite(op.y1, 0));
            out.f32(finite(op.x2, 0));
            out.f32(finite(op.y2, 0));
            out.f32(finite(op.width, DEFAULT_LINE_WIDTH));
            out.u32(colourWord(op.color, OPAQUE_WHITE));
            return;
        case "text":
            out.u8(TAG.text);
            out.f32(finite(op.x, 0));
            out.f32(finite(op.y, 0));
            out.f32(finite(op.size, DEFAULT_TEXT_SIZE));
            out.u32(colourWord(op.color, OPAQUE_WHITE));
            out.str2(op.font ?? DEFAULT_FONT);
            out.str4(String(op.text));
            return;
        case "icon":
            out.u8(TAG.icon);
            out.f32(finite(op.x, 0));
            out.f32(finite(op.y, 0));
            out.f32(finite(op.size, DEFAULT_ICON_SIZE));
            out.u32(colourWord(op.color, OPAQUE_WHITE));
            out.str2(String(op.name));
            return;
        case "pushClip":
            out.u8(TAG.pushClip);
            out.f32(finite(op.x, 0));
            out.f32(finite(op.y, 0));
            out.f32(finite(op.width, 0));
            out.f32(finite(op.height, 0));
            out.f32(finite(op.radius, 0));
            return;
        case "popClip":
            out.u8(TAG.popClip);
            return;
        case "pushOpacity":
            out.u8(TAG.pushOpacity);
            out.f32(finite(op.alpha, 1));
            return;
        case "popOpacity":
            out.u8(TAG.popOpacity);
            return;
        default:
            throw new TypeError("an op is a rect, line, text, icon, pushClip, popClip, pushOpacity or popOpacity");
    }
}

/**
 * The bytes of a canvas's `content` property for `ops`. The encoding is the one the retired Lua shell wrote
 * (`data/lua/gnoblin/shell/`) and the compositor decodes: `"GNPL"`, a version byte, a `u32` op count, then each op as a
 * tag byte and its fields. An op that cannot be written throws an error that names it (`draw op 2: ...`), and nothing
 * is written.
 */
export function encodePaintList(ops: readonly PaintOp[]): number[] {
    const body = new ByteWriter();
    ops.forEach((op, index) => {
        try {
            writeOp(body, op);
        } catch (error) {
            throw new Error(`draw op ${index + 1}: ${error instanceof Error ? error.message : String(error)}`);
        }
    });
    const out = new ByteWriter();
    out.bytes.push(0x47, 0x4e, 0x50, 0x4c); // "GNPL"
    out.u8(PAINT_LIST_VERSION);
    out.u32(ops.length);
    return out.bytes.concat(body.bytes);
}

// ---- Pointer events ----------------------------------------------------------------------------------------------

/**
 * A pointer event on a canvas. It goes to the canvas the compositor hit and then up through its parents until a
 * handler calls `stop()`. `x` and `y` are in the coordinates of the canvas whose handler runs.
 */
export class UiPointerEvent {
    readonly kind: string;
    x: number;
    y: number;
    readonly button: number;
    readonly dx: number;
    readonly dy: number;
    readonly target: Canvas;
    stopped = false;

    constructor(raw: UiRawEvent, target: Canvas) {
        this.kind = raw.kind;
        this.x = raw.x;
        this.y = raw.y;
        this.button = raw.button;
        this.dx = raw.dx;
        this.dy = raw.dy;
        this.target = target;
    }

    /** Keeps the event from reaching the parents of the canvas whose handler is running. */
    stop(): void {
        this.stopped = true;
    }
}

/**
 * A key sent to the canvas that holds the keyboard focus (`Canvas.focus()`), and from it up through its parents until a
 * handler calls `stop()`. `key` is the xkbcommon name of the keysym (`Return`, `Escape`, `a`), `text` what the key
 * types (empty for a key that types nothing), and `pressed` is false for the release. A key typed into an entry can
 * be a passphrase: nothing here prints it.
 */
export class UiKeyEvent {
    readonly key: string;
    readonly text: string;
    readonly pressed: boolean;
    readonly target: Canvas;
    stopped = false;

    constructor(raw: UiRawEvent, target: Canvas) {
        this.key = raw.key;
        this.text = raw.text;
        this.pressed = raw.pressed;
        this.target = target;
    }

    /** Keeps the event from reaching the parents of the canvas whose handler is running. */
    stop(): void {
        this.stopped = true;
    }
}

/** The handler a canvas may define for each pointer event kind the compositor sends. */
const HANDLERS: Readonly<Record<string, "onPress" | "onRelease" | "onMotion" | "onScroll" | "onEnter" | "onLeave">> = {
    press: "onPress",
    release: "onRelease",
    motion: "onMotion",
    scroll: "onScroll",
    enter: "onEnter",
    leave: "onLeave",
};

// ---- Canvas ------------------------------------------------------------------------------------------------------

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
    #width: number | null = null;
    #height: number | null = null;
    #alive = true;
    #written: Written = { x: 0, y: 0, width: 0, height: 0, visible: true, opacity: 1, secure: false };
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
        if (options.width !== undefined) this.#width = options.width;
        if (options.height !== undefined) this.#height = options.height;
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
            report("Canvas.destroy", new Error(`the host refused to destroy node ${this.node} (${code})`));
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

    #write(key: string, value: number | boolean): void {
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

function refuse(code: string | undefined, what: string): void {
    if (code !== undefined) {
        throw new Error(`Canvas.${what}: the host refused (${code})`);
    }
}

function guard(canvas: Canvas, what: string, body: () => void): void {
    try {
        body();
    } catch (error) {
        report(`${canvas.constructor.name}.${what}`, error);
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

// ---- Box ---------------------------------------------------------------------------------------------------------

/** How a box places its children: in a `row`, in a `column`, or `none` (they stay where they are). */
export type Direction = "row" | "column" | "none";

/** Options for `Box`. Sizes, gaps and padding are pixels. */
export interface BoxOptions extends CanvasOptions {
    direction?: Direction;
    gap?: number;
    padding?: number;
    /** A share of the space left in a row or column, among the children that grow. 0 keeps the child's own size. */
    grow?: number;
    fill?: Color;
    radius?: number;
    children?: readonly Canvas[];
}

/**
 * A canvas that fills itself and lays its child boxes out: `direction` `"row"` or `"column"` is a small flexbox, the
 * main axis in order with `gap` between and `padding` around; a child with `grow` takes its share of what is left;
 * every child is stretched across the other axis. Children that are not boxes stay where they are.
 */
export class Box extends Canvas {
    direction: Direction = "none";
    gap = 0;
    padding = 0;
    grow = 0;
    fill: Color = "#00000000";
    radius = 0;

    constructor(options: BoxOptions = {}) {
        super(options);
        if (options.direction !== undefined) this.direction = options.direction;
        if (options.gap !== undefined) this.gap = options.gap;
        if (options.padding !== undefined) this.padding = options.padding;
        if (options.grow !== undefined) this.grow = options.grow;
        if (options.fill !== undefined) this.fill = options.fill;
        if (options.radius !== undefined) this.radius = options.radius;
        for (const child of options.children ?? []) {
            this.add(child);
        }
    }

    override draw(): PaintOp[] {
        return [
            { op: "rect", x: 0, y: 0, width: this.width, height: this.height, radius: this.radius, fill: this.fill },
        ];
    }

    override arrange(): void {
        if (this.direction === "none") {
            return;
        }
        const column = this.direction === "column";
        const kids = this.children.filter((child): child is Box => child instanceof Box);
        const size = (box: Box): number => (column ? box.height : box.width);
        const pad = this.padding;
        let growing = 0;
        let used = 0;
        for (const kid of kids) {
            if (kid.grow > 0) {
                growing += kid.grow;
            } else {
                used += size(kid);
            }
        }
        const room = column ? this.height : this.width;
        const across = (column ? this.width : this.height) - 2 * pad;
        const free = room - 2 * pad - this.gap * Math.max(kids.length - 1, 0) - used;
        let at = pad;
        for (const kid of kids) {
            if (kid.grow > 0) {
                // A growing child takes its share of the free room. A negative share is clamped to zero.
                const share = Math.max(0, Math.floor((free * kid.grow) / growing));
                if (column) kid.height = share;
                else kid.width = share;
            }
            if (column) {
                kid.y = at;
                kid.x = pad;
                kid.width = across;
            } else {
                kid.x = at;
                kid.y = pad;
                kid.height = across;
            }
            at += size(kid) + this.gap;
        }
    }
}

// ---- Text --------------------------------------------------------------------------------------------------------

/** How much room a string takes: the width and height of its lines, and the distance down to the first baseline. */
export interface TextMetrics {
    width: number;
    height: number;
    ascent: number;
}

/**
 * Until the compositor answers a measurement, a text is sized by a guess: a sans glyph averages half the font size, a
 * line box is one and a half sizes tall and the baseline sits a little over one size below its top. The fractions are
 * rough on purpose: the answer replaces them within a frame, so they only need to keep the layout from jumping.
 */
export function estimateText(text: string, size: number): TextMetrics {
    let widest = 0;
    let lines = 0;
    for (const line of text.split("\n")) {
        widest = Math.max(widest, Math.floor(([...line].length * size) / 2));
        lines += 1;
    }
    return { width: widest, height: lines * Math.floor((size * 3) / 2), ascent: Math.floor((size * 11) / 10) };
}

/** The measurements asked and not yet answered, by token. */
const measuring = new Map<number, { text: Text; key: string }>();

/** Options for `Text`. Sizes are pixels. */
export interface TextOptions extends BoxOptions {
    text?: string;
    size?: number;
    color?: Color;
    font?: string;
    ellipsize?: boolean;
}

/**
 * One string. Its natural size is what the compositor measures for its font, size and text: the answer arrives a
 * frame later, and until then the size is `estimateText`'s. With `ellipsize` a text wider than the room its row gave
 * it is cut to the share that fits and ends in an ellipsis, so a row never has to know how wide a title is.
 */
export class Text extends Box {
    text = "";
    size = DEFAULT_TEXT_SIZE;
    color: Color = DEFAULT_TEXT_COLOUR;
    font = DEFAULT_FONT;
    ellipsize = false;
    // The answer in use, and the key it was measured under. It stops applying once the key changes.
    #measured: (TextMetrics & { key: string }) | null = null;
    // The key last asked. A text asks once per key: the compositor is not asked again for a key it was already asked
    // about. A changed font, size or text is a new key and asks again.
    #asked: string | null = null;

    constructor(options: TextOptions = {}) {
        super(options);
        if (options.text !== undefined) this.text = options.text;
        if (options.size !== undefined) this.size = options.size;
        if (options.color !== undefined) this.color = options.color;
        if (options.font !== undefined) this.font = options.font;
        if (options.ellipsize !== undefined) this.ellipsize = options.ellipsize;
    }

    /** Pixels from the top of the first line to its baseline: what texts of different sizes are lined up by. */
    get ascent(): number {
        return this.metrics().ascent;
    }

    /** What the compositor measured for the current font, size and text, or the estimate while it has not yet. */
    metrics(): TextMetrics {
        // The separator is a control character that does not occur in text, so two different triples do not
        // share a key.
        const key = `${this.font}\u0001${this.size}\u0001${this.text}`;
        if (this.#measured !== null && this.#measured.key === key) {
            return this.#measured;
        }
        if (this.#asked !== key && this.alive) {
            this.#asked = key;
            const token = host().measure(this.node, this.font, this.size, this.text);
            if (token !== undefined) {
                measuring.set(token, { text: this, key });
            }
        }
        return estimateText(this.text, this.size);
    }

    /** Takes a measurement's answer. An answer to a question the text has since moved on from is ignored. */
    answered(key: string, answer: UiAnswer): void {
        if (this.#asked !== key) {
            return;
        }
        const parsed = /^(\d+) (\d+) (\d+)$/.exec(answer.result);
        // An error keeps the estimate for this text: asking again would get the same refusal.
        const taken =
            parsed === null
                ? estimateText(this.text, this.size)
                : { width: Number(parsed[1]), height: Number(parsed[2]), ascent: Number(parsed[3]) };
        this.#measured = { ...taken, key };
        if (answer.error !== "") {
            report("Text.measure", new Error(`Text ${JSON.stringify(this.text)}: measure failed: ${answer.error}`));
        }
    }

    protected override naturalWidth(): number {
        return this.metrics().width;
    }

    protected override naturalHeight(): number {
        return this.metrics().height;
    }

    /** The text as drawn: all of it, or with `ellipsize` the part that fits the node's width and an ellipsis. */
    shown(): string {
        const measured = this.metrics();
        const chars = [...this.text];
        if (!this.ellipsize || this.width >= measured.width || measured.width === 0) {
            return this.text;
        }
        // One character of room is left for the ellipsis.
        const keep = Math.max(0, Math.floor((chars.length * this.width) / measured.width) - 1);
        return `${chars.slice(0, keep).join("")}\u2026`;
    }

    /**
     * The text block is centred in the node, so a row that squeezes the node shorter than the block (a titlebar)
     * trims both ends evenly and not just the bottom.
     */
    override draw(): PaintOp[] {
        const y = Math.floor((this.height - this.metrics().height) / 2);
        return [{ op: "text", text: this.shown(), x: 0, y, size: this.size, color: this.color, font: this.font }];
    }
}

// ---- Icon --------------------------------------------------------------------------------------------------------

/** Options for `Icon`. */
export interface IconOptions extends BoxOptions {
    icon?: string;
    size?: number;
    color?: Color;
}

/** One icon of the icon theme, `size` pixels square unless the node is given another size. */
export class Icon extends Box {
    icon = "";
    size = DEFAULT_ICON_SIZE;
    color: Color = DEFAULT_TEXT_COLOUR;

    constructor(options: IconOptions = {}) {
        super(options);
        if (options.icon !== undefined) this.icon = options.icon;
        if (options.size !== undefined) this.size = options.size;
        if (options.color !== undefined) this.color = options.color;
    }

    protected override naturalWidth(): number {
        return this.size;
    }

    protected override naturalHeight(): number {
        return this.size;
    }

    override draw(): PaintOp[] {
        return [{ op: "icon", name: this.icon, x: 0, y: 0, size: this.size, color: this.color }];
    }
}

// ---- Grid --------------------------------------------------------------------------------------------------------

/**
 * Children in a near-square grid of equal cells, `padding` around and `gap` between. Cells are floored to whole pixels,
 * so the spare pixels collect along the right and bottom edges.
 */
export class Grid extends Box {
    override arrange(): void {
        const kids = this.children;
        const pad = this.padding;
        const gap = this.gap;
        const cols = Math.max(1, Math.ceil(Math.sqrt(kids.length)));
        const rows = Math.max(1, Math.ceil(kids.length / cols));
        const w = Math.floor((this.width - 2 * pad - (cols - 1) * gap) / cols);
        const h = Math.floor((this.height - 2 * pad - (rows - 1) * gap) / rows);
        kids.forEach((kid, i) => {
            kid.x = pad + (i % cols) * (w + gap);
            kid.y = pad + Math.floor(i / cols) * (h + gap);
            kid.width = w;
            kid.height = h;
        });
    }
}

// ---- The frame ---------------------------------------------------------------------------------------------------

/** The host tells the toolkit a node was destroyed (the compositor closed a window, and its canvases went). */
export function uiNodeGone(node: number): void {
    live.get(node)?.markGone();
}

/** Destroys every canvas this generation made. A generation that is replaced calls this, so none is left behind. */
export function uiReleaseAll(): void {
    for (const canvas of [...live.values()]) {
        if (canvas.parent === null) {
            canvas.destroy();
        }
    }
    measuring.clear();
}

/** The number of live canvases. For tests and diagnostics. */
export function uiCanvasCount(): number {
    return live.size;
}

// ---- What the stock shell reads of the session -------------------------------------------------------------------

/** A request the compositor made of a window: `request` is `menu` for its window menu, anchored at `x`, `y`. */
export interface UiRequest {
    /** The window's node. */
    readonly node: number;
    readonly request: string;
    /** Where the menu is anchored, in the compositor's logical coordinates. */
    readonly x: number;
    readonly y: number;
}

/** Listeners for window requests, in the order they were added. `runUiFrame` delivers each request to all of them. */
const requestHandlers: ((request: UiRequest) => void)[] = [];
const frameHooks: (() => void)[] = [];

/** Hears every request the compositor makes of a window. Returns the function that stops listening. */
export function onUiRequest(handler: (request: UiRequest) => void): () => void {
    requestHandlers.push(handler);
    return () => {
        const at = requestHandlers.indexOf(handler);
        if (at >= 0) {
            requestHandlers.splice(at, 1);
        }
    };
}

/** Runs `hook` once a frame, after the events are delivered and before the canvases update. Returns the stop. */
export function onUiFrame(hook: () => void): () => void {
    frameHooks.push(hook);
    return () => {
        const at = frameHooks.indexOf(hook);
        if (at >= 0) {
            frameHooks.splice(at, 1);
        }
    };
}

/** Milliseconds on a clock that only goes forward: what the toolkit times its animations and delays by. */
export function uiNow(): number {
    return host().now();
}

/** The node with the keyboard focus, or `null`. */
export function uiFocused(): number | null {
    return host().focused() ?? null;
}

/** Gives the keyboard to a node: a window (or a canvas), or `null` to take it away. */
export function uiFocus(node: number | null): void {
    refuse(host().focus(node ?? 0), "focus");
}

/** Whether the session is locked or about to be. */
export function uiLocked(): boolean {
    return host().locked();
}

/** The size of the primary screen, or `null` while the tree has none. */
export function uiScreen(): { width: number; height: number } | null {
    const size = host().screen();
    return size === undefined ? null : { width: size[0], height: size[1] };
}

/** The seat's pointer in the compositor's logical coordinates, or `null` with no seat. */
export function uiPointer(): { x: number; y: number } | null {
    const x = host().seatNumber("pointer_x");
    const y = host().seatNumber("pointer_y");
    return x === undefined || y === undefined ? null : { x, y };
}

/** What a titlebar click does, from the seat: `key` is `double_click`, `middle_click` or `right_click`. */
export function uiSeatAction(key: "double_click" | "middle_click" | "right_click"): string | null {
    return host().seatText(key) ?? null;
}

/**
 * Delivers what the compositor sent since the last frame. Answers come first, so a handler that reads a text's size
 * sees the measurement that just arrived. Requests go to the listeners, not to a canvas: a window's menu is not a
 * canvas event.
 */
function deliver(): void {
    const toolkit = host();
    for (const answer of toolkit.takeAnswers()) {
        const asked = measuring.get(answer.token);
        measuring.delete(answer.token);
        if (asked !== undefined && asked.text.alive) {
            asked.text.answered(asked.key, answer);
        }
    }
    for (const raw of toolkit.takeEvents()) {
        if (raw.kind === "request") {
            const request: UiRequest = { node: raw.node, request: raw.request, x: raw.x, y: raw.y };
            for (const handler of [...requestHandlers]) {
                try {
                    handler(request);
                } catch (error) {
                    report("onUiRequest", error);
                }
            }
            continue;
        }
        const target = live.get(raw.node);
        if (target === undefined) {
            continue;
        }
        if (raw.kind === "key") {
            target.dispatchKey(new UiKeyEvent(raw, target));
        } else {
            target.dispatch(new UiPointerEvent(raw, target));
        }
    }
}

/**
 * One frame of the toolkit, called by the host after the layouts. In order: the measurements and pointer or key
 * events the compositor sent are delivered; the frame hooks run; then each root canvas updates its whole tree.
 * Does nothing until a host is attached.
 */
export function runUiFrame(): void {
    // No host is attached, so there is no tree to update.
    if (backend === null) {
        return;
    }
    deliver();
    for (const hook of [...frameHooks]) {
        try {
            hook();
        } catch (error) {
            report("onUiFrame", error);
        }
    }
    for (const canvas of [...live.values()]) {
        if (canvas.parent === null) {
            canvas.update();
        }
    }
}
