import type { CollectionState, Member } from "gnoblin-core/host/collection_state";
import type { ClientListener, CollectionListener, OutputListener } from "gnoblin-core/host/listeners";
import type { Client } from "gnoblin-core/host/client";
import type { Output } from "gnoblin-core/host/output";
import type { Subscriptions, Unsubscribe } from "gnoblin-core/subscriptions";
import type { Query } from "gnoblin-core/types";
/** A live, array-like view of compositor objects. It is not an Array and its membership cannot be changed. */
export declare class Collection<T extends Member> implements Iterable<T> {
    constructor(token: symbol, state: Omit<CollectionState, "self">);
    /** Current number of members. */
    get length(): number;
    /** Returns the member at `index`, or undefined when the index is out of range. */
    at(index: number): T | undefined;
    /**
     * The member with keyboard focus, or null. This is the first seat's focus, or the member whose `focused`
     * property is set when there is no seat. Only clients can hold focus.
     */
    get focused(): T | null;
    /** Returns the first member with this id, or undefined when no member matches. */
    get(id: string): T | undefined;
    /** Returns the first member accepted by `predicate`, or undefined when none matches. */
    find(predicate: (item: T) => boolean): T | undefined;
    /** A snapshot: the array does not follow later changes. */
    filter(predicate: (item: T) => boolean): T[];
    /** A snapshot, like `filter`. */
    map<U>(mapper: (item: T) => U): U[];
    /**
     * A live selection that tracks members matching the query's properties. Each value must equal the property, or
     * be one of the values in an array. `priority` ranks this selection's client style: higher values win, then the
     * style created later wins ties.
     */
    where(query: Query, options?: {
        priority?: number;
    }): Collection<T>;
    /**
     * Subscribes `listener` to this collection. A `CollectionListener` is told of every member now, as if each had
     * just been added (replay on attach, so a config that reloads sees the clients that are already there), and of
     * every later addition and removal. A `ClientListener` or an `OutputListener` (the one that fits what the
     * collection holds) is attached to every current and future member and detaches from a member that leaves.
     * `removeListener` undoes both.
     */
    addListener(listener: ClientListener | OutputListener | CollectionListener): void;
    /** Removes a listener and detaches it from the members it was attached to. */
    removeListener(listener: ClientListener | OutputListener | CollectionListener): void;
    /** Iterates over a snapshot of the current members. */
    [Symbol.iterator](): Iterator<T>;
}
/** The listener that takes the events of one kind of member: a `Client`'s, an `Output`'s, and none for the others. */
type MemberListener<T> = T extends Client ? ClientListener : T extends Output ? OutputListener : never;
/** The handler of the listener method `K`, as the members `T` have it; `never` where they have no such method. */
type MemberHandler<L, K extends PropertyKey> = L extends unknown ? K extends keyof L ? NonNullable<L[K]> : never : never;
/**
 * The member events of a collection as named subscriptions: `gnoblin.clients.onTitleChanged(handler)` hears the title of
 * every client, and `gnoblin.outputs.onModeChanged(handler)` the mode of every output. A collection of another kind of
 * member has none: its handler type is `never`.
 */
export type MemberSubscriptions<T> = {
    readonly [K in keyof ClientListener | keyof OutputListener]-?: (handler: MemberHandler<MemberListener<T>, K>) => Unsubscribe;
};
export interface Collection<T extends Member> extends Subscriptions<CollectionListener<T>>, MemberSubscriptions<T> {
}
/**
 * A collection whose membership scripts can change. Workspace collections are mutable; `gnoblin.clients` is not
 * because the compositor controls which windows exist.
 */
export declare class MutableCollection<T extends Member> extends Collection<T> {
    private mutation;
    /** Adds a member. On a workspace's `clients`, this moves the client from its current workspace. */
    add(member: T): void;
    /**
     * Removes a member by object or id. Members not in the collection are ignored. Returns true when removed, or
     * false when absent or refused (for example, `gnoblin.workspaces` retains its last workspace).
     */
    remove(member: T | string): boolean;
    /** Removes every member when this collection allows mutation. */
    clear(): void;
    /**
     * Swaps two members in the collection order. For a workspace's `clients`, the layout uses this order to place
     * tiled clients at the next frame. Both members must belong to the collection.
     */
    swap(a: T, b: T): void;
}
export {};
