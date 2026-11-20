// The styles of the library. A script's style objects (`ClientStyle` and the lists of elements in `gnoblin-std/style/style`) are the
// spelling of the API: checked fields, elements that edit in place, lists a script changes with `push`. What a style means is the
// library's (`gnoblin-std/generated`'s `Style`): which scope a client sits under, the cascade, what a resolved style becomes on the window
// node, what nothing draws. This module hands the library what a style object holds, and builds what a script reads back from it.

import { CollectionKind, EntryKind, PropKind, StyleKind, TermKind } from "gnoblin-std/generated/constants";
import { Collection, Style as NativeStyle } from "gnoblin-std/generated/classes";
import { Prop } from "gnoblin-std/generated/records";
import type { QueryTerm, StyleEntry } from "gnoblin-std/generated/records";
import type { Init } from "gnoblin-std/generated/native";
import { stdFailure } from "gnoblin-std/native_error";
import type { ComputedStyle } from "gnoblin-std/style/computed";
import type { Color, CornerRadii } from "gnoblin-std/style/values";

/** The library's number of each scalar and list property, by the script's name. */
const PROPS = {
    opacity: StyleKind.OPACITY,
    cornerRadius: StyleKind.CORNER_RADIUS,
    background: StyleKind.BACKGROUND,
    borders: StyleKind.BORDERS,
    shadows: StyleKind.SHADOWS,
    filter: StyleKind.FILTER,
    backdropFilter: StyleKind.BACKDROP_FILTER,
} as const;

export type NativeProp = keyof typeof PROPS;

const RADII_KEYS = ["topLeft", "topRight", "bottomRight", "bottomLeft"] as const;

/** A scalar as the library takes it. */
function scalarProp(name: "opacity" | "cornerRadius" | "background", value: unknown): Init<Prop> {
    if (name === "opacity") {
        return Prop.number(value as number);
    }
    if (name === "background") {
        return value === null ? Prop.null_() : Prop.text(value as Color);
    }
    if (typeof value === "number") {
        return Prop.number(value);
    }
    const radii = value as CornerRadii;
    return Prop.corners(radii.topLeft, radii.topRight, radii.bottomRight, radii.bottomLeft);
}

/** The elements of a list as `toJSON` writes them, as the library's entries. */
function entryOf(data: Record<string, unknown>): Init<StyleEntry> {
    const inset = data.inset === true ? 1 : 0;
    switch (data.kind) {
        case "blur":
            return { kind: EntryKind.BLUR, a: data.radius as number };
        case "saturate":
            return { kind: EntryKind.SATURATE, a: data.amount as number };
        case "brightness":
            return { kind: EntryKind.BRIGHTNESS, a: data.amount as number };
        case "contrast":
            return { kind: EntryKind.CONTRAST, a: data.amount as number };
        case "shader":
            return { kind: EntryKind.SHADER, source: data.source as string, params: data.params as number[] };
        default:
            break;
    }
    if ("width" in data) {
        return {
            kind: EntryKind.BORDER,
            inset,
            a: data.width as number,
            b: data.offset as number,
            color: data.color as string,
        };
    }
    return {
        kind: EntryKind.SHADOW,
        inset,
        a: data.x as number,
        b: data.y as number,
        c: data.blur as number,
        d: data.spread as number,
        color: data.color as string,
    };
}

/** A resolved entry as the plain data the script API has always handed out. */
function dataOf(entry: StyleEntry): Record<string, unknown> {
    switch (entry.kind) {
        case EntryKind.BORDER:
            return { width: entry.a, color: entry.color, offset: entry.b, inset: entry.inset !== 0 };
        case EntryKind.SHADOW:
            return {
                x: entry.a,
                y: entry.b,
                blur: entry.c,
                spread: entry.d,
                color: entry.color,
                inset: entry.inset !== 0,
            };
        case EntryKind.BLUR:
            return { kind: "blur", radius: entry.a };
        case EntryKind.SATURATE:
            return { kind: "saturate", amount: entry.a };
        case EntryKind.BRIGHTNESS:
            return { kind: "brightness", amount: entry.a };
        case EntryKind.SHADER:
            return { kind: "shader", source: entry.source, params: entry.params };
        default:
            return { kind: "contrast", amount: entry.a };
    }
}

/** What the library holds of one scope, and what it was last given. */
export interface Scope {
    readonly native: NativeStyle;
    /** The content of the script's style the library was last given, as JSON. */
    given: string;
}

/** The library's style of the scope of `target`: a collection of clients, or one client's node. */
export function scopeOf(kind: "collection" | "client", target: number): NativeStyle {
    try {
        const made = NativeStyle.scope(
            kind === "collection" ? StyleKind.SCOPE_COLLECTION : StyleKind.SCOPE_CLIENT,
            target,
        );
        if (made === null) {
            throw new Error("the library made no style");
        }
        return made;
    } catch (error) {
        throw stdFailure(error, "gnoblin.style");
    }
}

/** A query of `where` as the library's terms: its keys are the library's (`appId` is `app_id`), a list is alternatives. */
export function termsOf(query: Record<string, unknown>): Init<QueryTerm>[] {
    const terms: Init<QueryTerm>[] = [];
    for (const [name, want] of Object.entries(query)) {
        const key = name.replace(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`);
        for (const one of Array.isArray(want) ? want : [want]) {
            if (typeof one === "string") {
                terms.push({ kind: TermKind.TEXT, key, text: one });
            } else if (typeof one === "number") {
                terms.push({ kind: TermKind.NUMBER, key, number: one });
            } else {
                terms.push({ kind: TermKind.BOOL, key, flag: one === true ? 1 : 0 });
            }
        }
    }
    return terms;
}

/** The library's root collection of clients. */
export function clientsRoot(): Collection {
    const root = Collection.root(CollectionKind.CLIENTS);
    if (root === null) {
        throw new Error("the library has no collection of clients");
    }
    return root;
}

/** Hands the library the scalars and lists a style holds (`toJSON`), replacing what it had: nothing is written when they did not change. */
export function give(scope: Scope, content: Record<string, unknown>): void {
    const json = JSON.stringify(content);
    if (json === scope.given) {
        return;
    }
    scope.given = json;
    try {
        scope.native.unset(0);
        for (const [name, value] of Object.entries(content)) {
            const prop = PROPS[name as NativeProp];
            if (Array.isArray(value)) {
                scope.native.setList(
                    prop,
                    value.map((element) => entryOf(element as Record<string, unknown>)),
                );
            } else {
                scope.native.set(prop, scalarProp(name as "opacity" | "cornerRadius" | "background", value));
            }
        }
    } catch (error) {
        throw stdFailure(error, "gnoblin.style");
    }
}

/** The list a scope would hand a script that does not override `name`: what it inherits, as plain data. */
export function inherited(
    scope: Scope,
    name: "borders" | "shadows" | "filter" | "backdropFilter",
): Record<string, unknown>[] {
    return scope.native.list(PROPS[name]).map(dataOf);
}

/** The value the library resolved for a scalar property of a client, as the script reads it. */
function scalarOf(node: number, name: "opacity" | "cornerRadius" | "background"): unknown {
    const prop = NativeStyle.computed(node, PROPS[name]);
    switch (prop.kind) {
        case PropKind.CORNERS:
            return Object.freeze(
                Object.fromEntries(RADII_KEYS.map((key, at) => [key, [prop.a, prop.b, prop.c, prop.d][at]])),
            );
        case PropKind.NULL:
        case PropKind.UNSET:
            return null;
        case PropKind.TEXT:
            return prop.text;
        default:
            return prop.a;
    }
}

/** A client's resolved style: frozen data, and which layer each property came from. */
export function computedOf(node: number): ComputedStyle {
    const out: Record<string, unknown> = {};
    const origin: Record<string, string> = {};
    for (const name of ["opacity", "cornerRadius", "background"] as const) {
        out[name] = scalarOf(node, name);
        origin[name] = NativeStyle.origin(node, PROPS[name]);
    }
    for (const name of ["borders", "shadows", "filter", "backdropFilter"] as const) {
        out[name] = Object.freeze(
            NativeStyle.computedList(node, PROPS[name]).map((entry) => Object.freeze(dataOf(entry))),
        );
        origin[name] = NativeStyle.origin(node, PROPS[name]);
    }
    return Object.freeze({ ...out, origin: Object.freeze(origin) }) as unknown as ComputedStyle;
}

/** What a client's style asks for that nothing draws, one note each. */
export function notesOf(node: number): string[] {
    const text = NativeStyle.notes(node);
    return text === "" ? [] : text.split("\n");
}

/** What a client that is gone and was never asked resolves to: nothing set. */
export function emptyComputed(): ComputedStyle {
    const origin: Record<string, string> = {};
    for (const name of Object.keys(PROPS)) {
        origin[name] = "default";
    }
    return Object.freeze({
        opacity: 1,
        cornerRadius: 0,
        background: null,
        borders: Object.freeze([]),
        shadows: Object.freeze([]),
        filter: Object.freeze([]),
        backdropFilter: Object.freeze([]),
        origin: Object.freeze(origin),
    }) as unknown as ComputedStyle;
}
