// The two list elements that draw around a window: `Border` and `Shadow`.

import { assignFields, StyleElement, write } from "gnoblin-std/style/element";
import { colour, FINITE, flag, NON_NEGATIVE, RADIUS, SPREAD } from "gnoblin-std/style/values";
import type { Color } from "gnoblin-std/style/values";

/** The fields of a `Border`, in the order `toJSON` writes them. */
export const BORDER_FIELDS = ["width", "color", "offset", "inset"] as const;

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
export class Border extends StyleElement {
    #width = 1;
    #color: Color = "#ffffff";
    #offset = 0;
    #inset = false;

    constructor(init?: BorderInit) {
        super();
        assignFields(this, init, BORDER_FIELDS, "Border");
    }

    /** Stroke width in pixels. */
    get width(): number {
        return this.#width;
    }

    /** Sets the stroke width in pixels. */
    set width(value: number) {
        this.#width = write(this, "width", value, NON_NEGATIVE);
    }

    /** Stroke colour. */
    get color(): Color {
        return this.#color;
    }

    /** Sets the stroke colour. */
    set color(value: Color) {
        this.#color = write(this, "color", value, colour);
    }

    /** Distance from the frame in pixels; negative values move the stroke inwards. */
    get offset(): number {
        return this.#offset;
    }

    /** Sets the distance from the frame in pixels. */
    set offset(value: number) {
        this.#offset = write(this, "offset", value, FINITE);
    }

    /** Whether the stroke is drawn inside the frame. */
    get inset(): boolean {
        return this.#inset;
    }

    /** Sets whether the stroke is drawn inside the frame. */
    set inset(value: boolean) {
        this.#inset = write(this, "inset", value, flag);
    }

    /** Returns the border as plain data. */
    toJSON(): { width: number; color: Color; offset: number; inset: boolean } {
        return { width: this.#width, color: this.#color, offset: this.#offset, inset: this.#inset };
    }
}

/** The fields of a `Shadow`, in the order `toJSON` writes them. */
export const SHADOW_FIELDS = ["x", "y", "blur", "spread", "color", "inset"] as const;

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
export class Shadow extends StyleElement {
    #x = 0;
    #y = 0;
    #blur = 12;
    #spread = 0;
    #color: Color = "#00000080";
    #inset = false;

    constructor(init?: ShadowInit) {
        super();
        assignFields(this, init, SHADOW_FIELDS, "Shadow");
    }

    /** Horizontal offset in pixels. */
    get x(): number {
        return this.#x;
    }

    /** Sets the horizontal offset in pixels. */
    set x(value: number) {
        this.#x = write(this, "x", value, FINITE);
    }

    /** Vertical offset in pixels. */
    get y(): number {
        return this.#y;
    }

    /** Sets the vertical offset in pixels. */
    set y(value: number) {
        this.#y = write(this, "y", value, FINITE);
    }

    /** Blur radius in pixels. */
    get blur(): number {
        return this.#blur;
    }

    /** Sets the blur radius in pixels. */
    set blur(value: number) {
        this.#blur = write(this, "blur", value, RADIUS);
    }

    /** Spread in pixels. */
    get spread(): number {
        return this.#spread;
    }

    /** Sets the spread in pixels. */
    set spread(value: number) {
        this.#spread = write(this, "spread", value, SPREAD);
    }

    /** Shadow colour. */
    get color(): Color {
        return this.#color;
    }

    /** Sets the shadow colour. */
    set color(value: Color) {
        this.#color = write(this, "color", value, colour);
    }

    /** Whether the shadow is drawn inside the window. */
    get inset(): boolean {
        return this.#inset;
    }

    /** Sets whether the shadow is drawn inside the window. */
    set inset(value: boolean) {
        this.#inset = write(this, "inset", value, flag);
    }

    /** Returns the shadow as plain data. */
    toJSON(): { x: number; y: number; blur: number; spread: number; color: Color; inset: boolean } {
        return {
            x: this.#x,
            y: this.#y,
            blur: this.#blur,
            spread: this.#spread,
            color: this.#color,
            inset: this.#inset,
        };
    }
}

/** A border element or the literal that makes one, as a `borders` list takes it. */
export type BorderLike = Border | BorderInit;

/** A shadow element or the literal that makes one, as a `shadows` list takes it. */
export type ShadowLike = Shadow | ShadowInit;
