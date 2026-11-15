// The paint list: the drawing ops a canvas returns from `draw()` and their encoding into the bytes of the node's
// `content`, which `crates/gnoblin-paint/src/list.rs` decodes.

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
export const DEFAULT_FONT = "sans";
export const DEFAULT_TEXT_SIZE = 14;
export const DEFAULT_ICON_SIZE = 16;
const DEFAULT_LINE_WIDTH = 1;
/** The colour of text when a `Text` names none. Matches the stock menus' text colour. */
export const DEFAULT_TEXT_COLOUR = "#cdd6f4";

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
