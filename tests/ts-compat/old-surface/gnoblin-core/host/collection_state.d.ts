import type { Client } from "gnoblin-core/host/client";
import type { Collection } from "gnoblin-core/host/collections";
import type { Source } from "gnoblin-core/host/dispatch";
import type { Output } from "gnoblin-core/host/output";
import type { Query } from "gnoblin-core/types";
/**
 * The kinds of object a collection holds, by the name `where` and the console know them. A package that adds a kind
 * (workspaces, displays) merges its own into this interface, so `Member` and everything typed by it follow.
 */
export interface MemberKinds {
    Client: Client;
    Output: Output;
}
/** Any object kind supported by a host collection. */
export type Member = MemberKinds[keyof MemberKinds];
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
export declare function collectionState(collection: object): CollectionState;
/**
 * Fields supported by `where` for each kind of collection. A package that adds a kind of member names its fields here
 * when it is imported.
 */
export declare const QUERY_KEYS: Record<string, readonly string[]>;
/** Whether `member` belongs to the collection now, worked out from the objects and not from what was told. */
export declare function contains(state: CollectionState, member: Member): boolean;
/** Returns the current members in collection order. */
export declare function itemsOf(state: CollectionState): Member[];
/** Finds the root collection that owns a selection's members. */
export declare function rootOf(state: CollectionState): CollectionState;
/** Creates a root collection state for objects of `kind`. */
export declare function rootState(kind: string): Omit<CollectionState, "self">;
