import { StyleElement } from "gnoblin-std/style/element";
import type { Style } from "gnoblin-std/style/style";
import type { Checker } from "gnoblin-std/style/values";
/** The properties that are lists. */
export type ListName = "borders" | "shadows" | "filter" | "backdropFilter";
/** The properties that are one value. */
export type ScalarName = "opacity" | "cornerRadius" | "background";
/** Any style property, as `Style.unset` takes it. */
export type StyleProperty = ListName | ScalarName;
/** How an entry of each list becomes an element. */
export declare const LIST_ENTRY: Readonly<Record<ListName, (value: unknown, path: string, target: StyleElement[]) => StyleElement>>;
/** The checker of each scalar property. */
export declare const SCALAR_CHECK: Readonly<Record<ScalarName, Checker>>;
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
    /** Label used to start error paths for an unbound style. */
    label: string;
    /** Scope that supplies inherited values and checks liveness, or null. */
    scope: StyleScope | null;
    /** Scalar values explicitly set on this style. */
    scalars: Map<ScalarName, unknown>;
    /** List values explicitly set on this style. */
    lists: Map<ListName, StyleElement[]>;
    /** The entries each override list had when last checked. */
    seen: Map<ListName, StyleElement[]>;
    /** Cached copies of inherited lists, keyed by property. */
    views: Map<ListName, View>;
}
/** The store of `style`; a TypeError when `style` is not one of ours. */
export declare function storeOf(style: Style): Store;
/** The error path of property `name` of the style held by `store`. */
export declare function pathOf(store: Store, name: string): string;
/**
 * The array a style list property hands out. `add` and `remove` check and commit at once; `push` and `splice` are
 * checked once per frame by `syncStyleLists`.
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
/** Makes `list` the entries of override list `name`, with every element knowing where it sits. */
export declare function own(store: Store, name: ListName, list: StyleElement[]): void;
/** The setter of a list property: check every entry, then make the new list the scope's own. */
export declare function setList(style: Style, name: ListName, value: unknown): void;
/**
 * The getter of a list property. A scope that overrides the list returns its own. Otherwise the script gets a copy
 * of the inherited list (cached for one epoch) whose first write makes the copy the scope's own.
 */
export declare function getList(style: Style, name: ListName): StyleElement[];
/** The setter of a scalar property; `undefined` takes it back to inheriting. */
export declare function setScalar(style: Style, name: ScalarName, value: unknown): void;
/** Takes one property back to inheriting, in whichever store it sits. */
export declare function unsetOne(store: Store, name: StyleProperty): void;
/**
 * Compares every array of `style` that a script may have changed in place with what it held last time. Entries
 * that were added get checked and made elements; a bad change is undone and returned as an error, and a change of
 * a copy of an inherited list makes the copy the scope's own. Cheap when nothing moved.
 */
export declare function syncStyleLists(style: Style): Error[];
export {};
