// The filter elements (`BlurFilter`, `SaturateFilter`, `BrightnessFilter`, `ContrastFilter`) and the shorthand a
// list accepts for them.

import { assignFields, StyleElement, write } from "gnoblin/style/element";
import { GRADE, RADIUS } from "gnoblin/style/values";

/** A `BlurFilter` as a literal. `radius` is in pixels. */
export interface BlurFilterInit {
    radius?: number;
}

/** A Gaussian-style blur. Of a `backdropFilter` it blurs what is behind the window. */
export class BlurFilter extends StyleElement {
    #radius = 12;

    constructor(init?: BlurFilterInit) {
        super();
        assignFields(this, init, ["radius"], "BlurFilter");
    }

    get radius(): number {
        return this.#radius;
    }

    set radius(value: number) {
        this.#radius = write(this, "radius", value, RADIUS);
    }

    toJSON(): { kind: "blur"; radius: number } {
        return { kind: "blur", radius: this.#radius };
    }
}

/** A `SaturateFilter`, `BrightnessFilter` or `ContrastFilter` as a literal: `amount` is 1 for no change. */
export interface AmountInit {
    amount?: number;
}

/** The colour saturation, 1 unchanged and 0 grey, up to 2. */
export class SaturateFilter extends StyleElement {
    #amount = 1;

    constructor(init?: AmountInit) {
        super();
        assignFields(this, init, ["amount"], "SaturateFilter");
    }

    get amount(): number {
        return this.#amount;
    }

    set amount(value: number) {
        this.#amount = write(this, "amount", value, GRADE);
    }

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

    get amount(): number {
        return this.#amount;
    }

    set amount(value: number) {
        this.#amount = write(this, "amount", value, GRADE);
    }

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

    get amount(): number {
        return this.#amount;
    }

    set amount(value: number) {
        this.#amount = write(this, "amount", value, GRADE);
    }

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
