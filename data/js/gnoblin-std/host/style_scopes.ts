// The scopes of the styles a script holds: which collection or client each is the style of, and the library's scope that follows it.
// The cascade, the resolve, what a resolved style becomes on the node and what nothing draws are the library's (`gnoblin-std/style/native`);
// this module keeps the script's style objects and hands the library what they hold.

import { GnoblinError } from "gnoblin-core/errors";
import { host } from "gnoblin-core/host/bridge";
import type { Client } from "gnoblin-core/host/client";
import { QUERY_KEYS } from "gnoblin-core/host/collection_state";
import type { CollectionState } from "gnoblin-core/host/collection_state";
import { ctx } from "gnoblin-core/host/context";
import { std } from "gnoblin-std/host/state";
import { stateOf } from "gnoblin-core/host/node_state";
import type { Workspace } from "gnoblin-std/host/workspace";
import { wsState } from "gnoblin-std/host/workspace_state";
import type { Collection as NativeCollection } from "gnoblin-std/generated/classes";
import {
    clientsRoot,
    computedOf,
    emptyComputed,
    give,
    inherited,
    notesOf,
    scopeOf,
    termsOf,
} from "gnoblin-std/style/native";
import type { Scope } from "gnoblin-std/style/native";
import { LIST_ENTRY } from "gnoblin-std/style/lists";
import type { ListName } from "gnoblin-std/style/lists";
import type { StyleElement } from "gnoblin-std/style/element";
import { bindStyle, ClientStyle } from "gnoblin-std/style/style";
import type { ComputedStyle } from "gnoblin-std/style/computed";
import type { Style } from "gnoblin-std/style/style";
import { styleEpoch } from "gnoblin-std/style/state";

/** The style a collection of clients carries, and where it ranks among the selections. */
export interface ScopeStyle {
    /** The scope's mutable style values. */
    style: ClientStyle;
    /** The order the scope's style was first asked for in. */
    order: number;
    /** The library's scope of the collection. */
    scope: Scope;
}

/** What the script's side keeps of a client's style. */
export interface ClientStyleState {
    /** The client's own overrides, made the first time a script reads `client.style`. */
    style: ClientStyle | null;
    /** The library's scope of the client, made when the client is first asked about. */
    scope: Scope | null;
    /** The effective style values last computed for the client, kept after it is gone. */
    computed: ComputedStyle | null;
}

/** Lazily allocates the per-client style bookkeeping. */
export function styleState(client: Client): ClientStyleState {
    let state = std.clientStyles.get(client);
    if (state === undefined) {
        state = { style: null, scope: null, computed: null };
        std.clientStyles.set(client, state);
    }
    return state;
}

/** Finds which workspace client collection owns a style scope. */
function workspaceOfScope(state: CollectionState): Workspace | undefined {
    return (std.workspaceRoot.list as Workspace[]).find((workspace) => wsState(workspace).clientsState === state);
}

/** `gnoblin.clients`, `Workspace#dev.clients`, and `.where({...})` after either: the front of a scope's error paths. */
function scopePath(state: CollectionState): string {
    if (state.parent !== null) {
        return `${scopePath(state.parent)}.where(${JSON.stringify(state.query)})`;
    }
    return state === ctx.clientRoot ? "gnoblin.clients" : `Workspace#${workspaceOfScope(state)?.id ?? "?"}.clients`;
}

/** The library's collection that stands for a collection of clients of the script's. */
const natives = new WeakMap<CollectionState, NativeCollection>();

function nativeCollection(state: CollectionState): NativeCollection {
    let found = natives.get(state);
    if (found === undefined) {
        if (state.parent !== null) {
            const narrowed = nativeCollection(state.parent).where(termsOf(state.query ?? {}));
            if (narrowed === null) {
                throw new GnoblinError("internal", scopePath(state), "the library made no selection");
            }
            found = narrowed;
        } else if (state === ctx.clientRoot) {
            found = clientsRoot();
        } else {
            const workspace = workspaceOfScope(state);
            if (workspace === undefined) {
                throw new TypeError("style: only a collection of clients has a style");
            }
            found = wsState(workspace).nativeClients;
        }
        natives.set(state, found);
    }
    return found;
}

/** The elements of an inherited list, as a script gets them: copies, which no list owns. */
function elementsOf(name: ListName, data: Record<string, unknown>[]): StyleElement[] {
    return data.map((entry) => {
        const literal =
            typeof entry.kind === "string"
                ? { [entry.kind]: entry.kind === "blur" ? entry.radius : entry.amount }
                : entry;
        return LIST_ENTRY[name](literal, "inherited", []);
    });
}

/** The scope of `state`, made when first asked for. */
export function scopeStyle(state: CollectionState): ScopeStyle {
    if (state.keys !== QUERY_KEYS.Client) {
        throw new TypeError("style: only a collection of clients has a style");
    }
    let scope = std.scopeStyles.get(state);
    if (scope === undefined) {
        const style = new ClientStyle();
        const native = scopeOf("collection", nativeCollection(state).handle);
        scope = { style, order: std.flags.styleOrder, scope: { native, given: "{}" } };
        std.flags.styleOrder += 1;
        std.scopeStyles.set(state, scope);
        if (state.parent !== null) {
            std.styledSelections.push(state);
            if (state.priority !== undefined && state.priority !== 0) {
                native.rank(state.priority);
            }
        }
        const made = scope;
        bindStyle(style, {
            path: () => `${scopePath(state)}.style`,
            below: (name) => {
                pushStyles();
                return elementsOf(name, inherited(made.scope, name));
            },
        });
    }
    return scope;
}

/** The library's scope of a client, made when first needed. */
function clientScope(client: Client): Scope {
    const state = styleState(client);
    if (state.scope === null) {
        state.scope = { native: scopeOf("client", stateOf(client).node), given: "{}" };
    }
    return state.scope;
}

/** Creates and binds the client-owned style override layer. */
export function clientStyle(client: Client): ClientStyle {
    const state = styleState(client);
    if (state.style === null) {
        const style = new ClientStyle();
        const path = (): string => `Client#${host.label(stateOf(client).node)}.style`;
        bindStyle(style, {
            path,
            below: (name) => {
                pushStyles();
                return elementsOf(name, inherited(clientScope(client), name));
            },
            guard: () => {
                if (!stateOf(client).alive) {
                    throw new GnoblinError("not_found", path(), "the client is gone");
                }
            },
        });
        state.style = style;
    }
    return state.style;
}

/** The epoch the library was last given the styles at. */
let given = -1;

/** Hands the library what every style holds, when any changed. */
export function pushStyles(): void {
    const epoch = styleEpoch();
    if (epoch === given) {
        return;
    }
    given = epoch;
    for (const found of everyScope()) {
        give(found.scope, found.style.toJSON());
    }
    for (const client of ctx.clientRoot.list as Client[]) {
        const own = std.clientStyles.get(client)?.style;
        if (own !== null && own !== undefined && stateOf(client).alive) {
            give(clientScope(client), own.toJSON());
        }
    }
}

/** Resolves the effective style, retaining the last result after the client is gone. */
export function computedStyleOf(client: Client): ComputedStyle {
    const state = styleState(client);
    if (!stateOf(client).alive) {
        return state.computed ?? emptyComputed();
    }
    pushStyles();
    state.computed = computedOf(stateOf(client).node);
    return state.computed;
}

/** Reports style properties the renderer skipped for a client: what the library says it did not draw, and what the compositor did not. */
export function styleWarningsOf(client: Client): readonly string[] {
    pushStyles();
    return Object.freeze(notesOf(stateOf(client).node));
}

/** The scope styles of every collection a script asked the style of. */
function* everyScope(): Generator<ScopeStyle> {
    const scopes = [ctx.clientRoot, ...(std.workspaceRoot.list as Workspace[]).map((w) => wsState(w).clientsState)];
    for (const scope of [...scopes, ...std.styledSelections]) {
        const found = std.scopeStyles.get(scope);
        if (found !== undefined) {
            yield found;
        }
    }
}

/** Every style a script can have changed an array of in place. */
export function* everyStyle(): Generator<Style> {
    for (const found of everyScope()) {
        yield found.style;
    }
    for (const client of ctx.clientRoot.list as Client[]) {
        const own = std.clientStyles.get(client)?.style;
        if (own !== null && own !== undefined) {
            yield own;
        }
    }
}
