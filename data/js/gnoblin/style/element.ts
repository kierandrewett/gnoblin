// What every list element shares: it can be copied and written out as plain data, and each field write goes through
// `write`, which checks it and runs the copy-on-write and liveness hooks of the list the element sits in.

import { styleState } from "gnoblin/style/state";
import type { Checker } from "gnoblin/style/values";

/** Where a list element sits, for its error paths, and what runs before one of its fields is written. */
export interface Home {
    /** The list containing this element, or null when it is detached. */
    list: StyleElement[] | null;
    /** Returns this element's error path. */
    path(element: StyleElement): string;
    /** Runs before a field write, for liveness checks and copy-on-write. */
    beforeWrite?: () => void;
}

/** One accepted write of an element's field: the checked value, with the copy-on-write and liveness hooks run. */
export function write<T>(element: StyleElement, name: string, value: unknown, check: Checker<T>): T {
    const home = styleState.homes.get(element);
    const checked = check(value, `${home ? home.path(element) : element.constructor.name}.${name}`);
    home?.beforeWrite?.();
    if (styleState.building === 0) {
        styleState.epoch += 1;
    }
    return checked;
}

/** Sets each field of the literal `init` through the element's checked setters; an unknown field is a TypeError. */
export function assignFields(target: object, init: unknown, fields: readonly string[], label: string): void {
    if (init === undefined) {
        return;
    }
    if (init === null || typeof init !== "object" || Array.isArray(init)) {
        throw new TypeError(`${label}: expects an object of fields`);
    }
    for (const key of Object.keys(init)) {
        if (!fields.includes(key)) {
            throw new TypeError(`${label}: no field "${key}" (known: ${fields.join(", ")})`);
        }
        (target as Record<string, unknown>)[key] = (init as Record<string, unknown>)[key];
    }
}

/** What every list element shares: it can be copied and written out as plain data. */
export abstract class StyleElement {
    /** Returns this element's fields as plain data. */
    abstract toJSON(): Record<string, unknown>;
}

/** A list element class, as `entryOf` constructs it. */
export type ElementClass = new (init?: never) => StyleElement;

/** A copy of `element` that no list owns. Built under `building`, so making it is not a change. */
export function cloneElement(element: StyleElement): StyleElement {
    const Class = element.constructor as new (init: unknown) => StyleElement;
    styleState.building += 1;
    try {
        // A filter writes its `kind` out for the reader of `toJSON`; the constructor takes only its fields.
        const { kind: _kind, ...fields } = element.toJSON();
        return new Class(fields);
    } finally {
        styleState.building -= 1;
    }
}
