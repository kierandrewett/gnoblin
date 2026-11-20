/** A colour written as `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`. */
export type Color = string;
/** One drawing operation. Later operations paint over earlier ones; coordinates use the node's logical pixels. */
export type PaintOp = {
    op: "rect";
    x: number;
    y: number;
    width: number;
    height: number;
    radius?: number;
    fill?: Color;
    stroke?: Color;
    strokeWidth?: number;
} | {
    op: "line";
    x1: number;
    y1: number;
    x2: number;
    y2: number;
    width?: number;
    color?: Color;
} | {
    op: "text";
    text: string;
    x?: number;
    y?: number;
    size?: number;
    color?: Color;
    font?: string;
} | {
    op: "icon";
    name: string;
    x?: number;
    y?: number;
    size?: number;
    color?: Color;
} | {
    op: "pushClip";
    x: number;
    y: number;
    width: number;
    height: number;
    radius?: number;
} | {
    op: "popClip";
} | {
    op: "pushOpacity";
    alpha: number;
} | {
    op: "popOpacity";
};
/** Default font name used by text operations. */
export declare const DEFAULT_FONT = "sans";
/** Default text size in logical pixels. */
export declare const DEFAULT_TEXT_SIZE = 14;
/** Default icon size in logical pixels. */
export declare const DEFAULT_ICON_SIZE = 16;
/** The default text colour, matching the stock menus. */
export declare const DEFAULT_TEXT_COLOUR = "#cdd6f4";
/** Encodes `text` as UTF-8 bytes. A lone surrogate is replaced with U+FFFD. */
export declare function utf8(text: string): number[];
/**
 * Encodes drawing operations for a canvas's `content` property. The bytes start with `"GNPL"`, a version byte and a
 * `u32` operation count, followed by each operation's tag and fields. If an operation cannot be encoded, the error
 * names its position (for example, `draw op 2: ...`) and no paint list is returned.
 */
export declare function encodePaintList(ops: readonly PaintOp[]): number[];
