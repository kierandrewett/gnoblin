import type { Checker } from "gnoblin-std/style/values";
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
export declare function write<T>(element: StyleElement, name: string, value: unknown, check: Checker<T>): T;
/** Sets each field of the literal `init` through the element's checked setters; an unknown field is a TypeError. */
export declare function assignFields(target: object, init: unknown, fields: readonly string[], label: string): void;
/** What every list element shares: it can be copied and written out as plain data. */
export declare abstract class StyleElement {
    /** Returns this element's fields as plain data. */
    abstract toJSON(): Record<string, unknown>;
}
/** A list element class, as `entryOf` constructs it. */
export type ElementClass = new (init?: never) => StyleElement;
/** A copy of `element` that no list owns. Built under `building`, so making it is not a change. */
export declare function cloneElement(element: StyleElement): StyleElement;
