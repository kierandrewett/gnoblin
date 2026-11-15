// The cascade: the layers a client sits under (its own overrides, its workspace's, the collections' that hold it)
// resolved into one frozen style, with the layer each value came from.

import type { StyleElement } from "gnoblin/style/element";
import { LIST_ENTRY, SCALAR_CHECK, storeOf } from "gnoblin/style/lists";
import type { ListName, ScalarName, StyleProperty } from "gnoblin/style/lists";
import type { Style } from "gnoblin/style/style";
import type { Color, CornerRadii } from "gnoblin/style/values";

/** One layer of the cascade, with the name that `computedStyle` reports for the values it supplies. */
export interface StyleLayer {
    origin: string;
    style: Style;
}

/** The elements of `name` the highest layer that overrides it holds, or none. */
export function layeredList(layers: readonly StyleLayer[], name: ListName): readonly StyleElement[] {
    for (let at = layers.length - 1; at >= 0; at -= 1) {
        const list = storeOf(layers[at].style).lists.get(name);
        if (list !== undefined) {
            return list;
        }
    }
    return [];
}

/** A resolved border, as plain frozen data. */
export interface BorderValue {
    readonly width: number;
    readonly color: Color;
    readonly offset: number;
    readonly inset: boolean;
}

/** A resolved shadow, as plain frozen data. */
export interface ShadowValue {
    readonly x: number;
    readonly y: number;
    readonly blur: number;
    readonly spread: number;
    readonly color: Color;
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
    readonly opacity: number;
    readonly cornerRadius: number | Readonly<CornerRadii>;
    readonly background: Color | null;
    readonly borders: readonly BorderValue[];
    readonly shadows: readonly ShadowValue[];
    readonly filter: readonly FilterValue[];
    readonly backdropFilter: readonly FilterValue[];
    /** `default`, `global`, `workspace:<id>`, `where(<query>)` or `client`, for every property. */
    readonly origin: Readonly<Record<StyleProperty, string>>;
}

/** What a property resolves to when no layer sets it. */
const DEFAULT_SCALARS: Readonly<Record<ScalarName, unknown>> = { opacity: 1, cornerRadius: 0, background: null };

/** The style the layers resolve to, lowest layer first, per property: the highest layer that sets it wins. */
export function resolveStyle(layers: readonly StyleLayer[]): ComputedStyle {
    const origin = {} as Record<StyleProperty, string>;
    const resolved: Record<string, unknown> = {};
    for (const name of Object.keys(SCALAR_CHECK) as ScalarName[]) {
        origin[name] = "default";
        resolved[name] = DEFAULT_SCALARS[name];
        for (let at = layers.length - 1; at >= 0; at -= 1) {
            const scalars = storeOf(layers[at].style).scalars;
            if (scalars.has(name)) {
                resolved[name] = scalars.get(name);
                origin[name] = layers[at].origin;
                break;
            }
        }
    }
    for (const name of Object.keys(LIST_ENTRY) as ListName[]) {
        origin[name] = "default";
        resolved[name] = Object.freeze([]);
        for (let at = layers.length - 1; at >= 0; at -= 1) {
            const list = storeOf(layers[at].style).lists.get(name);
            if (list !== undefined) {
                resolved[name] = Object.freeze(list.map((element) => Object.freeze(element.toJSON())));
                origin[name] = layers[at].origin;
                break;
            }
        }
    }
    return Object.freeze({ ...resolved, origin: Object.freeze(origin) }) as ComputedStyle;
}
