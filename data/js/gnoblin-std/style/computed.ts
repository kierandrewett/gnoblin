// What `computedStyle` hands out: a client's resolved style as frozen plain data. The resolving is the library's (`gnoblin-std/style/native`).

import type { StyleProperty } from "gnoblin-std/style/lists";
import type { Color, CornerRadii } from "gnoblin-std/style/values";

/** A resolved border, as plain frozen data. */
export interface BorderValue {
    /** Border width in pixels. */
    readonly width: number;
    /** Border colour. */
    readonly color: Color;
    /** Border offset from the frame in pixels. */
    readonly offset: number;
    /** Whether the border is inset. */
    readonly inset: boolean;
}

/** A resolved shadow, as plain frozen data. */
export interface ShadowValue {
    /** Horizontal shadow offset in pixels. */
    readonly x: number;
    /** Vertical shadow offset in pixels. */
    readonly y: number;
    /** Shadow blur radius in pixels. */
    readonly blur: number;
    /** Shadow spread in pixels. */
    readonly spread: number;
    /** Shadow colour. */
    readonly color: Color;
    /** Whether the shadow is inset. */
    readonly inset: boolean;
}

/** A resolved filter, as plain frozen data. */
export type FilterValue =
    | { readonly kind: "blur"; readonly radius: number }
    | { readonly kind: "saturate"; readonly amount: number }
    | { readonly kind: "brightness"; readonly amount: number }
    | { readonly kind: "contrast"; readonly amount: number };

/** A client's resolved style: frozen data, and which layer each property came from. */
export interface ComputedStyle {
    /** Opacity from 0 (transparent) to 1 (opaque). */
    readonly opacity: number;
    /** Corner radius in pixels, or separate radii for each corner. */
    readonly cornerRadius: number | Readonly<CornerRadii>;
    /** Background colour, or null when unset. */
    readonly background: Color | null;
    /** Resolved borders. */
    readonly borders: readonly BorderValue[];
    /** Resolved shadows. */
    readonly shadows: readonly ShadowValue[];
    /** Resolved foreground filters. */
    readonly filter: readonly FilterValue[];
    /** Resolved backdrop filters. */
    readonly backdropFilter: readonly FilterValue[];
    /** `default`, `global`, `workspace:<id>`, `where(<query>)` or `client`, for every property. */
    readonly origin: Readonly<Record<StyleProperty, string>>;
}
