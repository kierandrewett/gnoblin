// The filter elements (`BlurFilter`, `SaturateFilter`, `BrightnessFilter`, `ContrastFilter`) and the shorthand a
// list accepts for them.

import { assignFields, StyleElement, write } from "gnoblin-std/style/element";
import { GRADE, RADIUS } from "gnoblin-std/style/values";

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

/** Any filter element of a `filter` or `backdropFilter` list. */
export type Filter = BlurFilter | SaturateFilter | BrightnessFilter | ContrastFilter;

/** The shorthand a filter list accepts: `{ blur: 18 }` is a `BlurFilter` of radius 18, and so on. */
export type FilterInit =
    Filter | { blur: number } | { saturate: number } | { brightness: number } | { contrast: number };

/** The one-key literals a filter list accepts, each with the element it makes and the checker of its number. */
export const FILTER_SHORTHAND = {
    blur: { make: (n: number) => new BlurFilter({ radius: n }), check: RADIUS },
    saturate: { make: (n: number) => new SaturateFilter({ amount: n }), check: GRADE },
    brightness: { make: (n: number) => new BrightnessFilter({ amount: n }), check: GRADE },
    contrast: { make: (n: number) => new ContrastFilter({ amount: n }), check: GRADE },
} as const;

/** Whether `value` is one of the four filter classes. */
export function isFilter(value: unknown): value is Filter {
    return (
        value instanceof BlurFilter ||
        value instanceof SaturateFilter ||
        value instanceof BrightnessFilter ||
        value instanceof ContrastFilter
    );
}
