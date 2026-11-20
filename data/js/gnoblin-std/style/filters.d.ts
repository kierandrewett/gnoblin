import { StyleElement } from "gnoblin-std/style/element";
/** A `BlurFilter` as a literal. `radius` is in pixels. */
export interface BlurFilterInit {
    /** Blur radius in pixels; defaults to 12. */
    radius?: number;
}
/** A Gaussian-style blur. Of a `backdropFilter` it blurs what is behind the window. */
export declare class BlurFilter extends StyleElement {
    #private;
    constructor(init?: BlurFilterInit);
    /** Returns the blur radius in pixels. */
    get radius(): number;
    /** Sets the blur radius in pixels. */
    set radius(value: number);
    /** Returns the filter as plain data. */
    toJSON(): {
        kind: "blur";
        radius: number;
    };
}
/** A `SaturateFilter`, `BrightnessFilter` or `ContrastFilter` as a literal: `amount` is 1 for no change. */
export interface AmountInit {
    /** Filter amount; 1 leaves the image unchanged. */
    amount?: number;
}
/** The colour saturation, 1 unchanged and 0 grey, up to 2. */
export declare class SaturateFilter extends StyleElement {
    #private;
    constructor(init?: AmountInit);
    /** Saturation amount, from 0 (grey) to 2; defaults to 1. */
    get amount(): number;
    /** Sets saturation from 0 (grey) to 2. */
    set amount(value: number);
    /** Returns the filter as plain data. */
    toJSON(): {
        kind: "saturate";
        amount: number;
    };
}
/** The brightness gain, 1 unchanged, up to 2. */
export declare class BrightnessFilter extends StyleElement {
    #private;
    constructor(init?: AmountInit);
    /** Brightness gain, from 0 to 2; defaults to 1. */
    get amount(): number;
    /** Sets the brightness gain from 0 to 2. */
    set amount(value: number);
    /** Returns the filter as plain data. */
    toJSON(): {
        kind: "brightness";
        amount: number;
    };
}
/** The contrast gain about mid grey, 1 unchanged, up to 2. */
export declare class ContrastFilter extends StyleElement {
    #private;
    constructor(init?: AmountInit);
    /** Contrast gain about mid grey, from 0 to 2; defaults to 1. */
    get amount(): number;
    /** Sets the contrast gain from 0 to 2. */
    set amount(value: number);
    /** Returns the filter as plain data. */
    toJSON(): {
        kind: "contrast";
        amount: number;
    };
}
/** A `ShaderFilter` as a literal. */
export interface ShaderFilterInit {
    /** The WGSL source: it defines `fn effect(c: vec4<f32>, uv: vec2<f32>, p: array<f32, 8>, t: f32) -> vec4<f32>`. */
    source?: string;
    /** The numbers the source reads as `p[0]` to `p[7]`: at most 8, each finite. */
    params?: number[];
}
/**
 * A shader of your own, drawn over the window. `c` is the window's premultiplied colour at `uv` (the window's rectangle, `(0, 0)` at the
 * top left), `p` is `params` and `t` is the time in seconds; the source may call `backdrop(uv)` for the colour of what is behind the
 * window. It is one `shader` effect of the window's node, in either filter list.
 *
 * The compositor draws it only when `compositor.user-shaders` is on; while it is off the client's `styleWarnings` say so. The source is
 * checked by the compositor, and a source it refuses leaves the window's whole effect list as it was.
 */
export declare class ShaderFilter extends StyleElement {
    #private;
    constructor(init?: ShaderFilterInit);
    /** Returns the WGSL source. */
    get source(): string;
    /** Sets the WGSL source: not blank, at most 16 KiB. */
    set source(value: string);
    /** Returns a copy of the numbers the source reads as `p`. */
    get params(): number[];
    /** Sets the numbers the source reads as `p`: at most 8, each finite. */
    set params(value: number[]);
    /** Returns the filter as plain data. */
    toJSON(): {
        kind: "shader";
        source: string;
        params: number[];
    };
}
/** Any filter element of a `filter` or `backdropFilter` list. */
export type Filter = BlurFilter | SaturateFilter | BrightnessFilter | ContrastFilter | ShaderFilter;
/** The shorthand a filter list accepts: `{ blur: 18 }` is a `BlurFilter` of radius 18, `{ shader: { source } }` a `ShaderFilter`, and so on. */
export type FilterInit = Filter | {
    blur: number;
} | {
    saturate: number;
} | {
    brightness: number;
} | {
    contrast: number;
} | {
    shader: ShaderFilterInit;
};
/** A one-key literal of a filter list: the element it makes and the checker of its value. */
interface Shorthand {
    make: (value: never) => Filter;
    check: (value: unknown, path: string) => unknown;
}
/** The one-key literals a filter list accepts, each with the element it makes and the checker of its value. */
export declare const FILTER_SHORTHAND: Readonly<Record<string, Shorthand>>;
/** Whether `value` is one of the five filter classes. */
export declare function isFilter(value: unknown): value is Filter;
export {};
