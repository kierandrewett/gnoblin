// `Collection`: a live, array-like view of the compositor's objects, and `MutableCollection`, one a script may
// change. The compositor owns which windows and outputs exist, so only the script's own collections (workspaces,
// a workspace's clients, profiles, displays) are mutable.

import { GnoblinError } from "gnoblin/errors";
import { CREATE } from "gnoblin/host/bridge";
import { collectionState, itemsOf } from "gnoblin/host/collection_state";
import type { CollectionState, Member, Mutation } from "gnoblin/host/collection_state";
import { ctx } from "gnoblin/host/context";
import { handlerOf, run } from "gnoblin/host/dispatch";
import { assertListener } from "gnoblin/host/listeners";
import type { ClientListener, CollectionListener } from "gnoblin/host/listeners";
import { focusedAmong } from "gnoblin/host/seat";
import { scopeStyle } from "gnoblin/host/style_scopes";
import { noteSignal } from "gnoblin/properties";
import { replaceStyle } from "gnoblin/style/style";
import type { ClientStyle, ClientStyleInit } from "gnoblin/style/style";
import type { Query } from "gnoblin/types";

/** A live, array-like view of the compositor's objects. Not an Array, and its membership is not writable. */
export class Collection<T extends Member> implements Iterable<T> {
    constructor(token: symbol, state: Omit<CollectionState, "self">) {
        if (token !== CREATE) {
            throw new TypeError("a Collection is made by the compositor, not by a script");
        }
        ctx.collections.set(this, { ...state, self: this as unknown as Collection<Member> });
    }

    get length(): number {
        const state = collectionState(this);
        noteSignal(state, "items");
        return state.parent === null ? state.list.length : itemsOf(state).length;
    }

    at(index: number): T | undefined {
        return itemsOf(collectionState(this)).at(index) as T | undefined;
    }

    /**
     * The member that holds the keyboard focus, or null: the first seat's focus, and with no seat the member whose
     * own `focused` is set. Only clients take the focus, so another collection's is null.
     */
    get focused(): T | null {
        return focusedAmong(itemsOf(collectionState(this))) as T | null;
    }

    get(id: string): T | undefined {
        return itemsOf(collectionState(this)).find((member) => member.id === id) as T | undefined;
    }

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
     * A live selection: it follows additions, removals and changes of the properties the query names. The
     * query is data, so the compositor side can evaluate it too: each key is a property and each value is
     * what it must equal, or an array of values of which one must. `priority` ranks the selection's `style`
     * among the others a client is in: the higher wins, and among equals the one whose style was made later.
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
     * The defaults this collection of clients gives: `gnoblin.clients.style` to all of them, a workspace's `clients` to
     * its own, a `where()` selection to the clients in it. Only the properties set here are overridden; see `Client.style`.
     * Assigning a `ClientStyle` or an object of properties replaces what this scope sets.
     */
    get style(): ClientStyle {
        return scopeStyle(collectionState(this)).style;
    }

    set style(value: ClientStyle | ClientStyleInit) {
        replaceStyle(scopeStyle(collectionState(this)).style, value);
    }

    /**
     * Subscribes `listener` to this collection. A `CollectionListener` is told of every member now, as if
     * each had just been added (replay on attach, so a config that reloads sees the clients that are
     * already there), and of every later addition and removal. A `ClientListener` is attached to every
     * current and future member and detaches from a member that leaves. `removeListener` undoes both.
     */
    addListener(listener: ClientListener | CollectionListener): void {
        assertListener(listener, "Collection.addListener");
        const state = collectionState(this);
        if (state.listeners.includes(listener)) {
            return;
        }
        state.listeners.push(listener);
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

    removeListener(listener: ClientListener | CollectionListener): void {
        const state = collectionState(this);
        const at = state.listeners.indexOf(listener);
        if (at < 0) {
            return;
        }
        state.listeners.splice(at, 1);
        if (state.parent !== null && state.listeners.length === 0 && state.members !== null) {
            state.members = null;
            ctx.tracked.splice(ctx.tracked.indexOf(state), 1);
        }
    }

    [Symbol.iterator](): Iterator<T> {
        return (itemsOf(collectionState(this)) as T[])[Symbol.iterator]();
    }
}

/**
 * A collection a script may change. `gnoblin.workspaces` and a workspace's `clients` are these;
 * `gnoblin.clients` is not, because the compositor owns which windows exist.
 */
export class MutableCollection<T extends Member> extends Collection<T> {
    private mutation(what: string): Mutation {
        const { mutation } = collectionState(this);
        if (!mutation) {
            throw new GnoblinError("read_only", `Collection.${what}`, "this collection's membership cannot be changed");
        }
        return mutation;
    }

    /** Adds a member. On a workspace's `clients` this moves the client there from its workspace. */
    add(member: T): void {
        this.mutation("add").add(member);
    }

    /**
     * Removes a member, given as itself or by id. A member that is not in the collection is ignored. True when a member was
     * removed; false when there was none to remove or the collection refused (`gnoblin.workspaces` keeps its last one).
     */
    remove(member: T | string): boolean {
        const target = typeof member === "string" ? this.get(member) : member;
        if (target === undefined || !collectionState(this).list.includes(target)) {
            return false;
        }
        return this.mutation("remove").remove(target) !== false;
    }

    clear(): void {
        this.mutation("clear").clear();
    }

    /**
     * Trades the places of two members in the collection's order. A workspace's `clients` is the order its layout
     * places by, so two clients of a tiled workspace trade tiles at the next frame. Both must be members.
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
