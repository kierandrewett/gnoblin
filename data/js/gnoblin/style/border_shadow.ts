// The two list elements that draw around a window: `Border` and `Shadow`.

import { assignFields, StyleElement, write } from "gnoblin/style/element";
import { colour, FINITE, flag, NON_NEGATIVE, RADIUS, SPREAD } from "gnoblin/style/values";
import type { Color } from "gnoblin/style/values";

/** The fields of a `Border`, in the order `toJSON` writes them. */
export const BORDER_FIELDS = ["width", "color", "offset", "inset"] as const;

/** A `Border` as a literal: any field may be left out. */
export interface BorderInit {
    width?: number;
    color?: Color;
    offset?: number;
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

    get width(): number {
        return this.#width;
    }

    set width(value: number) {
        this.#width = write(this, "width", value, NON_NEGATIVE);
    }

    get color(): Color {
        return this.#color;
    }

    set color(value: Color) {
        this.#color = write(this, "color", value, colour);
    }

    get offset(): number {
        return this.#offset;
    }

    set offset(value: number) {
        this.#offset = write(this, "offset", value, FINITE);
    }

    get inset(): boolean {
        return this.#inset;
    }

    set inset(value: boolean) {
        this.#inset = write(this, "inset", value, flag);
    }

    toJSON(): { width: number; color: Color; offset: number; inset: boolean } {
        return { width: this.#width, color: this.#color, offset: this.#offset, inset: this.#inset };
    }
}

/** The fields of a `Shadow`, in the order `toJSON` writes them. */
export const SHADOW_FIELDS = ["x", "y", "blur", "spread", "color", "inset"] as const;

/** A `Shadow` as a literal: any field may be left out. Offsets, blur and spread are in pixels. */
export interface ShadowInit {
    x?: number;
    y?: number;
    blur?: number;
    spread?: number;
    color?: Color;
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

    get x(): number {
        return this.#x;
    }

    set x(value: number) {
        this.#x = write(this, "x", value, FINITE);
    }

    get y(): number {
        return this.#y;
    }

    set y(value: number) {
        this.#y = write(this, "y", value, FINITE);
    }

    get blur(): number {
        return this.#blur;
    }

    set blur(value: number) {
        this.#blur = write(this, "blur", value, RADIUS);
    }

    get spread(): number {
        return this.#spread;
    }

    set spread(value: number) {
        this.#spread = write(this, "spread", value, SPREAD);
    }

    get color(): Color {
        return this.#color;
    }

    set color(value: Color) {
        this.#color = write(this, "color", value, colour);
    }

    get inset(): boolean {
        return this.#inset;
    }

    set inset(value: boolean) {
        this.#inset = write(this, "inset", value, flag);
    }

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
