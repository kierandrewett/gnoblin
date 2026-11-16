// Live views of compositor objects and mutable collections owned by scripts.

import { GnoblinError } from "gnoblin-core/errors";
import { CREATE } from "gnoblin-core/host/bridge";
import { collectionState, itemsOf } from "gnoblin-core/host/collection_state";
import type { CollectionState, Member, Mutation } from "gnoblin-core/host/collection_state";
import { ctx } from "gnoblin-core/host/context";
import { attach, detach, handlerOf, run } from "gnoblin-core/host/dispatch";
import { assertListener } from "gnoblin-core/host/listeners";
import type { ClientListener, CollectionListener, OutputListener } from "gnoblin-core/host/listeners";
import { focusedAmong } from "gnoblin-core/host/seat";
import { noteSignal } from "gnoblin-core/properties";
import type { Query } from "gnoblin-core/types";

/** A live, array-like view of compositor objects. It is not an Array and its membership cannot be changed. */
export class Collection<T extends Member> implements Iterable<T> {
    constructor(token: symbol, state: Omit<CollectionState, "self">) {
        if (token !== CREATE) {
            throw new TypeError("a Collection is made by the compositor, not by a script");
        }
        ctx.collections.set(this, { ...state, self: this as unknown as Collection<Member> });
    }

    /** Current number of members. */
    get length(): number {
        const state = collectionState(this);
        noteSignal(state, "items");
        return state.parent === null ? state.list.length : itemsOf(state).length;
    }

    /** Returns the member at `index`, or undefined when the index is out of range. */
    at(index: number): T | undefined {
        return itemsOf(collectionState(this)).at(index) as T | undefined;
    }

    /**
     * The member with keyboard focus, or null. This is the first seat's focus, or the member whose `focused`
     * property is set when there is no seat. Only clients can hold focus.
     */
    get focused(): T | null {
        return focusedAmong(itemsOf(collectionState(this))) as T | null;
    }

    /** Returns the first member with this id, or undefined when no member matches. */
    get(id: string): T | undefined {
        return itemsOf(collectionState(this)).find((member) => member.id === id) as T | undefined;
    }

    /** Returns the first member accepted by `predicate`, or undefined when none matches. */
    find(predicate: (item: T) => boolean): T | undefined {
        return (itemsOf(collectionState(this)) as T[]).find((item) => predicate(item));
    }

    /** A snapshot: the array does not follow later changes. */
    filter(predicate: (item: T) => boolean): T[] {
        return (itemsOf(collectionState(this)) as T[]).filter((item) => predicate(item));
    }

    /** A snapshot, like `filter`. */
    map<U>(mapper: (item: T) => U): U[] {
        return (itemsOf(collectionState(this)) as T[]).map((item) => mapper(item));
    }

    /**
     * A live selection that tracks members matching the query's properties. Each value must equal the property, or
     * be one of the values in an array. `priority` ranks this selection's client style: higher values win, then the
     * style created later wins ties.
     */
    where(query: Query, options: { priority?: number } = {}): Collection<T> {
        const state = collectionState(this);
        if (typeof query !== "object" || query === null || Array.isArray(query)) {
            throw new TypeError("where: the query must be an object of property values");
        }
        for (const [key, want] of Object.entries(query)) {
            if (!state.keys.includes(key)) {
                throw new TypeError(`where: no property \`${key}\` to select on (known: ${state.keys.join(", ")})`);
            }
            const values = Array.isArray(want) ? want : [want];
            if (!values.every((value) => ["string", "number", "boolean"].includes(typeof value))) {
                throw new TypeError(`where: \`${key}\` must be a string, number or boolean, or an array of them`);
            }
        }
        const priority = options?.priority ?? 0;
        if (typeof priority !== "number" || !Number.isFinite(priority)) {
            throw new TypeError("where: priority must be a finite number");
        }
        return new Collection<T>(CREATE, {
            parent: state,
            query: { ...query },
            list: [],
            members: null,
            keys: state.keys,
            listeners: [],
            priority,
        });
    }

    /**
     * Subscribes `listener` to this collection. A `CollectionListener` is told of every member now, as if each had
     * just been added (replay on attach, so a config that reloads sees the clients that are already there), and of
     * every later addition and removal. A `ClientListener` or an `OutputListener` (the one that fits what the
     * collection holds) is attached to every current and future member and detaches from a member that leaves.
     * `removeListener` undoes both.
     */
    addListener(listener: ClientListener | OutputListener | CollectionListener): void {
        assertListener(listener, "Collection.addListener");
        const state = collectionState(this);
        if (!attach(state.listeners, listener)) {
            return;
        }
        if (state.parent !== null && state.members === null) {
            state.members = new Set(itemsOf(state));
            ctx.tracked.push(state);
        }
        if (typeof handlerOf(listener, "onAdded") === "function") {
            for (const member of itemsOf(state)) {
                run({ source: state, listener, method: "onAdded", args: [member, this] });
            }
        }
    }

    /** Removes a listener and detaches it from the members it was attached to. */
    removeListener(listener: ClientListener | OutputListener | CollectionListener): void {
        const state = collectionState(this);
        if (!detach(state.listeners, listener)) {
            return;
        }
        if (state.parent !== null && state.listeners.length === 0 && state.members !== null) {
            state.members = null;
            ctx.tracked.splice(ctx.tracked.indexOf(state), 1);
        }
    }

    /** Iterates over a snapshot of the current members. */
    [Symbol.iterator](): Iterator<T> {
        return (itemsOf(collectionState(this)) as T[])[Symbol.iterator]();
    }
}

/**
 * A collection whose membership scripts can change. Workspace collections are mutable; `gnoblin.clients` is not
 * because the compositor controls which windows exist.
 */
export class MutableCollection<T extends Member> extends Collection<T> {
    private mutation(what: string): Mutation {
        const { mutation } = collectionState(this);
        if (!mutation) {
            throw new GnoblinError("read_only", `Collection.${what}`, "this collection's membership cannot be changed");
        }
        return mutation;
    }

    /** Adds a member. On a workspace's `clients`, this moves the client from its current workspace. */
    add(member: T): void {
        this.mutation("add").add(member);
    }

    /**
     * Removes a member by object or id. Members not in the collection are ignored. Returns true when removed, or
     * false when absent or refused (for example, `gnoblin.workspaces` retains its last workspace).
     */
    remove(member: T | string): boolean {
        const target = typeof member === "string" ? this.get(member) : member;
        if (target === undefined || !collectionState(this).list.includes(target)) {
            return false;
        }
        return this.mutation("remove").remove(target) !== false;
    }

    /** Removes every member when this collection allows mutation. */
    clear(): void {
        this.mutation("clear").clear();
    }

    /**
     * Swaps two members in the collection order. For a workspace's `clients`, the layout uses this order to place
     * tiled clients at the next frame. Both members must belong to the collection.
     */
    swap(a: T, b: T): void {
        const mutation = this.mutation("swap");
        const list = collectionState(this).list;
        const first = list.indexOf(a);
        const second = list.indexOf(b);
        if (first < 0 || second < 0) {
            throw new GnoblinError("invalid_argument", "Collection.swap", "both members must be in the collection");
        }
        if (first === second) {
            return;
        }
        list[first] = b;
        list[second] = a;
        mutation.swapped?.(a, b);
    }
}
