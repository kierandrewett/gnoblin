// The filter elements (`BlurFilter`, `SaturateFilter`, `BrightnessFilter`, `ContrastFilter`, `ShaderFilter`) and the shorthand a
// list accepts for them.

import { assignFields, StyleElement, write } from "gnoblin-std/style/element";
import { GRADE, RADIUS, shaderParams, shaderSource } from "gnoblin-std/style/values";

/** A `BlurFilter` as a literal. `radius` is in pixels. */
export interface BlurFilterInit {
    /** Blur radius in pixels; defaults to 12. */
    radius?: number;
}

/** A Gaussian-style blur. Of a `backdropFilter` it blurs what is behind the window. */
export class BlurFilter extends StyleElement {
    #radius = 12;

    constructor(init?: BlurFilterInit) {
        super();
        assignFields(this, init, ["radius"], "BlurFilter");
    }

    /** Returns the blur radius in pixels. */
    get radius(): number {
        return this.#radius;
    }

    /** Sets the blur radius in pixels. */
    set radius(value: number) {
        this.#radius = write(this, "radius", value, RADIUS);
    }

    /** Returns the filter as plain data. */
    toJSON(): { kind: "blur"; radius: number } {
        return { kind: "blur", radius: this.#radius };
    }
}

/** A `SaturateFilter`, `BrightnessFilter` or `ContrastFilter` as a literal: `amount` is 1 for no change. */
export interface AmountInit {
    /** Filter amount; 1 leaves the image unchanged. */
    amount?: number;
}

/** The colour saturation, 1 unchanged and 0 grey, up to 2. */
export class SaturateFilter extends StyleElement {
    #amount = 1;

    constructor(init?: AmountInit) {
        super();
        assignFields(this, init, ["amount"], "SaturateFilter");
    }

    /** Saturation amount, from 0 (grey) to 2; defaults to 1. */
    get amount(): number {
        return this.#amount;
    }

    /** Sets saturation from 0 (grey) to 2. */
    set amount(value: number) {
        this.#amount = write(this, "amount", value, GRADE);
    }

    /** Returns the filter as plain data. */
    toJSON(): { kind: "saturate"; amount: number } {
        return { kind: "saturate", amount: this.#amount };
    }
}

/** The brightness gain, 1 unchanged, up to 2. */
export class BrightnessFilter extends StyleElement {
    #amount = 1;

    constructor(init?: AmountInit) {
        super();
        assignFields(this, init, ["amount"], "BrightnessFilter");
    }

    /** Brightness gain, from 0 to 2; defaults to 1. */
    get amount(): number {
        return this.#amount;
    }

    /** Sets the brightness gain from 0 to 2. */
    set amount(value: number) {
        this.#amount = write(this, "amount", value, GRADE);
    }

    /** Returns the filter as plain data. */
    toJSON(): { kind: "brightness"; amount: number } {
        return { kind: "brightness", amount: this.#amount };
    }
}

/** The contrast gain about mid grey, 1 unchanged, up to 2. */
export class ContrastFilter extends StyleElement {
    #amount = 1;

    constructor(init?: AmountInit) {
        super();
        assignFields(this, init, ["amount"], "ContrastFilter");
    }

    /** Contrast gain about mid grey, from 0 to 2; defaults to 1. */
    get amount(): number {
        return this.#amount;
    }

    /** Sets the contrast gain from 0 to 2. */
    set amount(value: number) {
        this.#amount = write(this, "amount", value, GRADE);
    }

    /** Returns the filter as plain data. */
    toJSON(): { kind: "contrast"; amount: number } {
        return { kind: "contrast", amount: this.#amount };
    }
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
export class ShaderFilter extends StyleElement {
    #source = "";
    #params: number[] = [];

    constructor(init?: ShaderFilterInit) {
        super();
        assignFields(this, init, ["source", "params"], "ShaderFilter");
    }

    /** Returns the WGSL source. */
    get source(): string {
        return this.#source;
    }

    /** Sets the WGSL source: not blank, at most 16 KiB. */
    set source(value: string) {
        this.#source = write(this, "source", value, shaderSource);
    }

    /** Returns a copy of the numbers the source reads as `p`. */
    get params(): number[] {
        return [...this.#params];
    }

    /** Sets the numbers the source reads as `p`: at most 8, each finite. */
    set params(value: number[]) {
        this.#params = write(this, "params", value, shaderParams);
    }

    /** Returns the filter as plain data. */
    toJSON(): { kind: "shader"; source: string; params: number[] } {
        return { kind: "shader", source: this.#source, params: [...this.#params] };
    }
}

/** Any filter element of a `filter` or `backdropFilter` list. */
export type Filter = BlurFilter | SaturateFilter | BrightnessFilter | ContrastFilter | ShaderFilter;

/** The shorthand a filter list accepts: `{ blur: 18 }` is a `BlurFilter` of radius 18, `{ shader: { source } }` a `ShaderFilter`, and so on. */
export type FilterInit =
    | Filter
    | { blur: number }
    | { saturate: number }
    | { brightness: number }
    | { contrast: number }
    | { shader: ShaderFilterInit };

/** A one-key literal of a filter list: the element it makes and the checker of its value. */
interface Shorthand {
    make: (value: never) => Filter;
    check: (value: unknown, path: string) => unknown;
}

/** The one-key literals a filter list accepts, each with the element it makes and the checker of its value. */
export const FILTER_SHORTHAND: Readonly<Record<string, Shorthand>> = {
    blur: { make: (n: number) => new BlurFilter({ radius: n }), check: RADIUS },
    saturate: { make: (n: number) => new SaturateFilter({ amount: n }), check: GRADE },
    brightness: { make: (n: number) => new BrightnessFilter({ amount: n }), check: GRADE },
    contrast: { make: (n: number) => new ContrastFilter({ amount: n }), check: GRADE },
    shader: {
        make: (init: ShaderFilterInit) => new ShaderFilter(init),
        check: (value, path) => {
            if (value === null || typeof value !== "object" || Array.isArray(value)) {
                throw new TypeError(`${path}: must be an object of { source, params }`);
            }
            const { source, params } = value as ShaderFilterInit;
            return {
                source: shaderSource(source, `${path}.source`),
                params: shaderParams(params ?? [], `${path}.params`),
            };
        },
    },
};

/** Whether `value` is one of the five filter classes. */
export function isFilter(value: unknown): value is Filter {
    return (
        value instanceof BlurFilter ||
        value instanceof SaturateFilter ||
        value instanceof BrightnessFilter ||
        value instanceof ContrastFilter ||
        value instanceof ShaderFilter
    );
}
