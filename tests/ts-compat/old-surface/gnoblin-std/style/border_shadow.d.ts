import { StyleElement } from "gnoblin-std/style/element";
import type { Color } from "gnoblin-std/style/values";
/** The fields of a `Border`, in the order `toJSON` writes them. */
export declare const BORDER_FIELDS: readonly ["width", "color", "offset", "inset"];
/** A `Border` as a literal: any field may be left out. */
export interface BorderInit {
    /** Stroke width in pixels; defaults to 1. */
    width?: number;
    /** Stroke colour; defaults to white. */
    color?: Color;
    /** Distance from the frame in pixels; negative values move the stroke inwards. */
    offset?: number;
    /** Draws the stroke inside the frame when true. */
    inset?: boolean;
}
/**
 * One stroke around the window. Several make concentric rings, `offset` apart. `width` and `offset` are in pixels;
 * `offset` moves the ring out from the frame (negative moves it in).
 */
export declare class Border extends StyleElement {
    #private;
    constructor(init?: BorderInit);
    /** Stroke width in pixels. */
    get width(): number;
    /** Sets the stroke width in pixels. */
    set width(value: number);
    /** Stroke colour. */
    get color(): Color;
    /** Sets the stroke colour. */
    set color(value: Color);
    /** Distance from the frame in pixels; negative values move the stroke inwards. */
    get offset(): number;
    /** Sets the distance from the frame in pixels. */
    set offset(value: number);
    /** Whether the stroke is drawn inside the frame. */
    get inset(): boolean;
    /** Sets whether the stroke is drawn inside the frame. */
    set inset(value: boolean);
    /** Returns the border as plain data. */
    toJSON(): {
        width: number;
        color: Color;
        offset: number;
        inset: boolean;
    };
}
/** The fields of a `Shadow`, in the order `toJSON` writes them. */
export declare const SHADOW_FIELDS: readonly ["x", "y", "blur", "spread", "color", "inset"];
/** A `Shadow` as a literal: any field may be left out. Offsets, blur and spread are in pixels. */
export interface ShadowInit {
    /** Horizontal offset in pixels; defaults to 0. */
    x?: number;
    /** Vertical offset in pixels; defaults to 0. */
    y?: number;
    /** Blur radius in pixels; defaults to 12. */
    blur?: number;
    /** Spread in pixels; defaults to 0. */
    spread?: number;
    /** Shadow colour; defaults to translucent black. */
    color?: Color;
    /** Draws the shadow inside the window when true. */
    inset?: boolean;
}
/**
 * A shadow behind the window, or inside it with `inset`. `x` and `y` move it, `blur` softens its edge and `spread`
 * grows it, all in pixels.
 */
export declare class Shadow extends StyleElement {
    #private;
    constructor(init?: ShadowInit);
    /** Horizontal offset in pixels. */
    get x(): number;
    /** Sets the horizontal offset in pixels. */
    set x(value: number);
    /** Vertical offset in pixels. */
    get y(): number;
    /** Sets the vertical offset in pixels. */
    set y(value: number);
    /** Blur radius in pixels. */
    get blur(): number;
    /** Sets the blur radius in pixels. */
    set blur(value: number);
    /** Spread in pixels. */
    get spread(): number;
    /** Sets the spread in pixels. */
    set spread(value: number);
    /** Shadow colour. */
    get color(): Color;
    /** Sets the shadow colour. */
    set color(value: Color);
    /** Whether the shadow is drawn inside the window. */
    get inset(): boolean;
    /** Sets whether the shadow is drawn inside the window. */
    set inset(value: boolean);
    /** Returns the shadow as plain data. */
    toJSON(): {
        x: number;
        y: number;
        blur: number;
        spread: number;
        color: Color;
        inset: boolean;
    };
}
/** A border element or the literal that makes one, as a `borders` list takes it. */
export type BorderLike = Border | BorderInit;
/** A shadow element or the literal that makes one, as a `shadows` list takes it. */
export type ShadowLike = Shadow | ShadowInit;
