// The lists and scalars a style holds: what a scope stores, how an entry becomes an element, the copy of an
// inherited list a script is handed, and the once-per-frame comparison that finds an in-place change.

import { Border, BORDER_FIELDS, Shadow, SHADOW_FIELDS } from "gnoblin/style/border_shadow";
import { assignFields, cloneElement, StyleElement } from "gnoblin/style/element";
import type { ElementClass } from "gnoblin/style/element";
import { FILTER_SHORTHAND, isFilter } from "gnoblin/style/filters";
import { styleState } from "gnoblin/style/state";
import type { Style } from "gnoblin/style/style";
import { background, cornerRadius, fail, MAX_STYLE_LIST, UNIT } from "gnoblin/style/values";
import type { Checker } from "gnoblin/style/values";

/** The element a list entry stands for: an element of that class (copied when another list owns it) or a literal. */
function entryOf(
    value: unknown,
    path: string,
    target: StyleElement[],
    Class: ElementClass,
    fields: readonly string[],
): StyleElement {
    if (value instanceof Class) {
        const home = styleState.homes.get(value);
        return home?.list && home.list !== target ? cloneElement(value) : value;
    }
    if (value === null || typeof value !== "object" || Array.isArray(value) || value instanceof StyleElement) {
        throw new TypeError(`${path}: must be a ${Class.name} or an object of its fields`);
    }
    styleState.building += 1;
    try {
        const element = new Class();
        styleState.homes.set(element, { list: null, path: () => path });
        assignFields(element, value, fields, path);
        return element;
    } finally {
        styleState.building -= 1;
    }
}

/** The element a `filter` or `backdropFilter` entry stands for: a filter element or a one-key literal. */
function filterEntry(value: unknown, path: string, target: StyleElement[]): StyleElement {
    if (isFilter(value)) {
        const home = styleState.homes.get(value);
        return home?.list && home.list !== target ? cloneElement(value) : value;
    }
    if (value !== null && typeof value === "object" && !Array.isArray(value) && !(value instanceof StyleElement)) {
        const keys = Object.keys(value);
        const key = keys[0] as keyof typeof FILTER_SHORTHAND;
        if (keys.length === 1 && Object.hasOwn(FILTER_SHORTHAND, key)) {
            const entry = FILTER_SHORTHAND[key];
            const amount = entry.check((value as Record<string, unknown>)[key], `${path}.${key}`);
            styleState.building += 1;
            try {
                return entry.make(amount);
            } finally {
                styleState.building -= 1;
            }
        }
    }
    throw new TypeError(
        `${path}: must be a filter: { blur: 18 }, { saturate: 1.2 }, { brightness: 1.1 }, { contrast: 1.1 } or a ` +
            "BlurFilter, SaturateFilter, BrightnessFilter or ContrastFilter",
    );
}

/** The properties that are lists. */
export type ListName = "borders" | "shadows" | "filter" | "backdropFilter";

/** The properties that are one value. */
export type ScalarName = "opacity" | "cornerRadius" | "background";

/** Any style property, as `Style.unset` takes it. */
export type StyleProperty = ListName | ScalarName;

/** How an entry of each list becomes an element. */
export const LIST_ENTRY: Readonly<
    Record<ListName, (value: unknown, path: string, target: StyleElement[]) => StyleElement>
> = {
    borders: (value, path, target) => entryOf(value, path, target, Border as ElementClass, BORDER_FIELDS),
    shadows: (value, path, target) => entryOf(value, path, target, Shadow as ElementClass, SHADOW_FIELDS),
    filter: filterEntry,
    backdropFilter: filterEntry,
};

/** The checker of each scalar property. */
export const SCALAR_CHECK: Readonly<Record<ScalarName, Checker>> = {
    opacity: UNIT,
    cornerRadius,
    background,
};

/** What a scope tells its style: where it is, what it inherits, and whether it may still be written. */
export interface StyleScope {
    /** `Client#42.style`, `gnoblin.clients.style`: the front of every error path. */
    path(): string;
    /** The list the layers below this scope resolve to, as live elements. */
    below(name: ListName): readonly StyleElement[];
    /** Throws when the scope is gone (a destroyed client). */
    guard?(): void;
}

/** A copy of an inherited list handed to a script, valid while the epoch has not moved. */
interface View {
    epoch: number;
    list: StyleElement[];
    /** The entries `list` had when it was handed out. */
    seen: StyleElement[];
}

/** What one style object holds. It lives in a WeakMap so that a script cannot reach it through the object. */
export interface Store {
    label: string;
    scope: StyleScope | null;
    scalars: Map<ScalarName, unknown>;
    lists: Map<ListName, StyleElement[]>;
    /** The entries each override list had when last checked. */
    seen: Map<ListName, StyleElement[]>;
    views: Map<ListName, View>;
}

/** The store of `style`; a TypeError when `style` is not one of ours. */
export function storeOf(style: Style): Store {
    const store = styleState.stores.get(style);
    if (store === undefined) {
        throw new TypeError("not a style object");
    }
    return store;
}

/** The error path of property `name` of the style held by `store`. */
export function pathOf(store: Store, name: string): string {
    return `${store.scope?.path() ?? store.label}.${name}`;
}

/** Whether two lists hold the same element objects in the same order. */
function sameEntries(a: readonly StyleElement[], b: readonly StyleElement[]): boolean {
    return a.length === b.length && a.every((element, at) => element === b[at]);
}

/**
 * The array a style list property hands out, with the two mutators the specification's `ObservableCollection` has.
 * Both check and commit at once, where `push` and `splice` are seen only by the frame comparison (`syncStyleLists`).
 */
export interface StyleList<T, L = T> extends Array<T> {
    /**
     * Appends an element, or the literal that makes one, and returns the element. Throws, leaving the list as it was,
     * when the entry is bad, the list is full (`MAX_STYLE_LIST`) or the element is already in it.
     */
    add(entry: L): T;
    /** Takes `element` out of the list. False when it is not in it. */
    remove(element: T): boolean;
}

/**
 * Gives `list` its `add` and `remove`. They are own, non-enumerable properties, so `for...in`, spreading and `JSON`
 * see a plain array. `commit` makes the list the scope's own after a change (a copy of an inherited list becomes the
 * override, as the first in-place write to it does).
 */
function observable(store: Store, name: ListName, list: StyleElement[], commit: () => void): void {
    const add = (entry: unknown): StyleElement => {
        store.scope?.guard?.();
        const path = pathOf(store, name);
        if (list.length >= MAX_STYLE_LIST) {
            return fail(path, `holds at most ${MAX_STYLE_LIST} entries, so another cannot be added`);
        }
        const element = LIST_ENTRY[name](entry, `${path}[${list.length}]`, list);
        if (list.includes(element)) {
            throw new TypeError(`${path}: that element is already in the list`);
        }
        list.push(element);
        commit();
        return element;
    };
    const remove = (element: StyleElement): boolean => {
        store.scope?.guard?.();
        const at = list.indexOf(element);
        if (at < 0) {
            return false;
        }
        list.splice(at, 1);
        styleState.homes.set(element, { list: null, path: () => element.constructor.name });
        commit();
        return true;
    };
    Object.defineProperties(list, {
        add: { value: add, configurable: true, writable: true },
        remove: { value: remove, configurable: true, writable: true },
    });
}

/** Makes `list` the entries of override list `name`, with every element knowing where it sits. */
export function own(store: Store, name: ListName, list: StyleElement[]): void {
    for (const element of list) {
        styleState.homes.set(element, {
            list,
            path: (e) => `${pathOf(store, name)}[${list.indexOf(e)}]`,
            beforeWrite: () => store.scope?.guard?.(),
        });
    }
    observable(store, name, list, () => {
        own(store, name, list);
        styleState.epoch += 1;
    });
    store.lists.set(name, list);
    store.seen.set(name, [...list]);
    store.views.delete(name);
}

/** The first write to a copy of an inherited list: the copy becomes the scope's own list. */
function commitView(store: Store, name: ListName, view: View): void {
    store.scope?.guard?.();
    if (store.lists.has(name) || store.views.get(name) !== view) {
        return;
    }
    own(store, name, view.list);
    styleState.epoch += 1;
}

/** `value` as a list of elements for property `name`, each entry checked; the first bad entry throws with its path. */
function checkedList(store: Store, name: ListName, value: unknown, target: StyleElement[]): StyleElement[] {
    const path = pathOf(store, name);
    if (!Array.isArray(value)) {
        throw new TypeError(`${path}: must be an array`);
    }
    if (value.length > MAX_STYLE_LIST) {
        return fail(path, `holds at most ${MAX_STYLE_LIST} entries, got ${value.length}`);
    }
    const entries: StyleElement[] = [];
    for (let at = 0; at < value.length; at += 1) {
        entries.push(LIST_ENTRY[name](value[at], `${path}[${at}]`, target));
    }
    return entries;
}

/** The setter of a list property: check every entry, then make the new list the scope's own. */
export function setList(style: Style, name: ListName, value: unknown): void {
    const store = storeOf(style);
    store.scope?.guard?.();
    if (value === undefined) {
        unsetOne(store, name);
        return;
    }
    const target: StyleElement[] = [];
    const entries = checkedList(store, name, value, target);
    target.push(...entries);
    own(store, name, target);
    styleState.epoch += 1;
}

/**
 * The getter of a list property. A scope that overrides the list returns its own. Otherwise the script gets a copy
 * of the inherited list (cached for one epoch) whose first write makes the copy the scope's own.
 */
export function getList(style: Style, name: ListName): StyleElement[] {
    const store = storeOf(style);
    const overriding = store.lists.get(name);
    if (overriding !== undefined) {
        return overriding;
    }
    const held = store.views.get(name);
    if (held !== undefined && held.epoch === styleState.epoch) {
        return held.list;
    }
    const list: StyleElement[] = [];
    const view: View = { epoch: styleState.epoch, list, seen: list };
    styleState.building += 1;
    try {
        for (const inherited of store.scope?.below(name) ?? []) {
            const copy = cloneElement(inherited);
            styleState.homes.set(copy, {
                list,
                path: (e) => `${pathOf(store, name)}[${list.indexOf(e)}]`,
                beforeWrite: () => commitView(store, name, view),
            });
            list.push(copy);
        }
    } finally {
        styleState.building -= 1;
    }
    view.seen = [...list];
    observable(store, name, list, () => commitView(store, name, view));
    store.views.set(name, view);
    return list;
}

/** The setter of a scalar property; `undefined` takes it back to inheriting. */
export function setScalar(style: Style, name: ScalarName, value: unknown): void {
    const store = storeOf(style);
    store.scope?.guard?.();
    if (value === undefined) {
        unsetOne(store, name);
        return;
    }
    store.scalars.set(name, SCALAR_CHECK[name](value, pathOf(store, name)));
    styleState.epoch += 1;
}

/** Takes one property back to inheriting, in whichever store it sits. */
export function unsetOne(store: Store, name: StyleProperty): void {
    store.scope?.guard?.();
    store.scalars.delete(name as ScalarName);
    store.lists.delete(name as ListName);
    store.seen.delete(name as ListName);
    store.views.delete(name as ListName);
    styleState.epoch += 1;
}

/**
 * Compares every array of `style` that a script may have changed in place with what it held last time. Entries
 * that were added get checked and made elements; a bad change is undone and returned as an error, and a change of
 * a copy of an inherited list makes the copy the scope's own. Cheap when nothing moved.
 */
export function syncStyleLists(style: Style): Error[] {
    const store = storeOf(style);
    const errors: Error[] = [];
    for (const [name, list] of store.lists) {
        const seen = store.seen.get(name) ?? [];
        if (!sameEntries(list, seen)) {
            recheck(store, name, list, seen, errors, () => own(store, name, list));
        }
    }
    for (const [name, view] of store.views) {
        if (!sameEntries(view.list, view.seen)) {
            recheck(store, name, view.list, view.seen, errors, () => commitView(store, name, view));
        }
    }
    return errors;
}

/** Checks the list that changed in place; on success `accept` runs, on failure the old entries come back. */
function recheck(
    store: Store,
    name: ListName,
    list: StyleElement[],
    seen: StyleElement[],
    errors: Error[],
    accept: () => void,
): void {
    try {
        store.scope?.guard?.();
        const entries = checkedList(store, name, [...list], list);
        list.splice(0, list.length, ...entries);
        accept();
        styleState.epoch += 1;
    } catch (error) {
        list.splice(0, list.length, ...seen);
        errors.push(error instanceof Error ? error : new Error(String(error)));
    }
}
