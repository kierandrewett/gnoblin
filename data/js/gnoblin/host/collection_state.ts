// Private collection state and the functions that query its current members.

import type { Display } from "gnoblin/display/display";
import type { OutputProfile } from "gnoblin/display/output_profile";
import type { Client } from "gnoblin/host/client";
import type { Collection } from "gnoblin/host/collections";
import { ctx } from "gnoblin/host/context";
import type { Source } from "gnoblin/host/dispatch";
import type { Output } from "gnoblin/host/output";
import type { Workspace } from "gnoblin/host/workspace";
import { noteSignal } from "gnoblin/properties";
import type { Query } from "gnoblin/types";

/** Any object kind supported by a host collection. */
export type Member = Client | Workspace | Output | Display | OutputProfile;

/** Operations that mutate a script-owned collection. */
export interface Mutation {
    /** Adds a member to a mutable collection. */
    add(member: Member): void;
    /** False when the collection refuses to let the member go; anything else is taken as done. */
    remove(member: Member): boolean | void;
    /** Clears all members from a mutable collection. */
    clear(): void;
    /** Told after two members traded places in the order. */
    swapped?(a: Member, b: Member): void;
}

/** Private view state for root collections and their filtered selections. */
export interface CollectionState extends Source {
    /** Public collection object represented by this state. */
    self: Collection<Member>;
    /** The collection a `where` narrows; null for a root collection, which holds `list`. */
    parent: CollectionState | null;
    /** Query used by a selection; null for a root collection. */
    query: Query | null;
    /** Root collection members in order; selections compute their members from the parent and query. */
    list: Member[];
    /** The members a view last told its listeners of, kept only while it has listeners. */
    members: Set<Member> | null;
    /** Properties available to a selection's query. */
    keys: readonly string[];
    /** Set on a collection scripts may change: what `add`, `remove` and `clear` do. */
    mutation?: Mutation;
    /** What a `where()` selection ranks by among the selections that style a client: higher wins. */
    priority?: number;
}

/** Returns the private state of a collection. Throws for non-collection values. */
export function collectionState(collection: object): CollectionState {
    const state = ctx.collections.get(collection);
    if (!state) {
        throw new TypeError("not a gnoblin collection");
    }
    return state;
}

/** Fields supported by `where` for each kind of collection. */
export const QUERY_KEYS: Record<string, readonly string[]> = {
    Client: ["id", "appId", "title", "focused", "fullscreen", "x", "y", "width", "height"],
    Workspace: ["id", "name", "index", "active"],
    Output: ["id", "name", "make", "model", "mode", "scale", "primary", "enabled", "x", "y", "width", "height"],
    Display: ["id", "mode"],
    OutputProfile: ["id", "enabled", "scale", "transform"],
};

/** Tests a member against the data-only property query. */
function matches(query: Query, member: Member): boolean {
    for (const [key, want] of Object.entries(query)) {
        const have = Reflect.get(member, key);
        if (Array.isArray(want) ? !want.includes(have) : have !== want) {
            return false;
        }
    }
    return true;
}

/** Whether `member` belongs to the collection now, worked out from the objects and not from what was told. */
export function contains(state: CollectionState, member: Member): boolean {
    if (state.parent === null) {
        return state.list.includes(member);
    }
    return contains(state.parent, member) && matches(state.query as Query, member);
}

/** Returns the current members in collection order. */
export function itemsOf(state: CollectionState): Member[] {
    if (state.parent === null) {
        noteSignal(state, "items");
        return state.list.slice();
    }
    const query = state.query as Query;
    return itemsOf(state.parent).filter((member) => matches(query, member));
}

/** Finds the root collection that owns a selection's members. */
export function rootOf(state: CollectionState): CollectionState {
    return state.parent === null ? state : rootOf(state.parent);
}

/** Creates a root collection state for objects of `kind`. */
export function rootState(kind: string): Omit<CollectionState, "self"> {
    return {
        parent: null,
        query: null,
        list: [],
        members: null,
        keys: QUERY_KEYS[kind],
        listeners: [],
    };
}
