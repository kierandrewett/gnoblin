// Which style layers apply to a client: its own overrides, its workspace's, the collections' that hold it, and
// `where()` selections that style it, with the order they rank in. This is the resolution only (what `Client.style`
// and `computedStyle` read); `styles` writes the resolved values to the nodes.

import { GnoblinError } from "gnoblin/errors";
import { host } from "gnoblin/host/bridge";
import type { Client } from "gnoblin/host/client";
import { contains, QUERY_KEYS, rootOf } from "gnoblin/host/collection_state";
import type { CollectionState } from "gnoblin/host/collection_state";
import { ctx } from "gnoblin/host/context";
import { stateOf } from "gnoblin/host/node_state";
import type { Workspace } from "gnoblin/host/workspace";
import { wsOf, wsState } from "gnoblin/host/workspace_state";
import { bindStyle, ClientStyle, layeredList, resolveStyle } from "gnoblin/style";
import type { ComputedStyle, Style, StyleLayer } from "gnoblin/style";

/** The style a collection of clients carries, and where it ranks among the selections. */
export interface ScopeStyle {
    style: ClientStyle;
    /** The order the scope's style was first asked for in: the tie-break between selections of equal priority. */
    order: number;
}

/** What a client's style last resolved from and wrote to its node. */
export interface ClientStyleState {
    /** The client's own overrides, made the first time a script reads `client.style`. */
    style: ClientStyle | null;
    applied: boolean;
    /** The style epoch and the layers the last application was made for. */
    epoch: number;
    layers: string;
    computed: ComputedStyle | null;
    notes: string[];
    /** The effects last written, as JSON; null before the first write. */
    effects: string | null;
    /** The opacity last written, or the node's own when nothing was written yet. */
    opacity: number | null;
    /** The corner radius last written, or the node's own when nothing was written yet. */
    cornerRadius: number | null;
}

/** Lazily allocates the per-client style bookkeeping used by the frame pass. */
export function styleState(client: Client): ClientStyleState {
    let state = ctx.clientStyles.get(client);
    if (state === undefined) {
        state = {
            style: null,
            applied: false,
            epoch: -1,
            layers: "",
            computed: null,
            notes: [],
            effects: null,
            opacity: null,
            cornerRadius: null,
        };
        ctx.clientStyles.set(client, state);
    }
    return state;
}

/** Finds which workspace client collection owns a style scope. */
function workspaceOfScope(state: CollectionState): Workspace | undefined {
    return (ctx.workspaceRoot.list as Workspace[]).find((workspace) => wsState(workspace).clientsState === state);
}

/** `gnoblin.clients`, `Workspace#dev.clients`, and `.where({...})` after either: the front of a scope's error paths. */
function scopePath(state: CollectionState): string {
    if (state.parent !== null) {
        return `${scopePath(state.parent)}.where(${JSON.stringify(state.query)})`;
    }
    return state === ctx.clientRoot ? "gnoblin.clients" : `Workspace#${workspaceOfScope(state)?.id ?? "?"}.clients`;
}

/** The name `computedStyle.origin` gives a scope. */
function scopeOrigin(state: CollectionState): string {
    if (state.parent !== null) {
        return scopePath(state).replace(/^gnoblin\.clients\./, "");
    }
    return state === ctx.clientRoot ? "global" : `workspace:${workspaceOfScope(state)?.id ?? "?"}`;
}

/** The layers of the scopes above a collection's own: what a list it does not override is inherited from. */
function layersAbove(state: CollectionState): StyleLayer[] {
    const layers: StyleLayer[] = [];
    const global = ctx.scopeStyles.get(ctx.clientRoot);
    if (global !== undefined && state !== ctx.clientRoot) {
        layers.push({ origin: "global", style: global.style });
    }
    const root = rootOf(state);
    const scope = root === ctx.clientRoot ? undefined : ctx.scopeStyles.get(root);
    if (scope !== undefined && state !== root) {
        layers.push({ origin: scopeOrigin(root), style: scope.style });
    }
    return layers;
}

/** The scope of `state`, made when first asked for. */
export function scopeStyle(state: CollectionState): ScopeStyle {
    if (state.keys !== QUERY_KEYS.Client) {
        throw new TypeError("style: only a collection of clients has a style");
    }
    let scope = ctx.scopeStyles.get(state);
    if (scope === undefined) {
        const style = new ClientStyle();
        scope = { style, order: ctx.flags.styleOrder };
        ctx.flags.styleOrder += 1;
        ctx.scopeStyles.set(state, scope);
        if (state.parent !== null) {
            ctx.styledSelections.push(state);
        }
        bindStyle(style, {
            path: () => `${scopePath(state)}.style`,
            below: (name) => layeredList(layersAbove(state), name),
        });
    }
    return scope;
}

/** The scopes a client sits under, lowest precedence first: global, its workspace, then its `where()` selections. */
function scopeLayers(client: Client): StyleLayer[] {
    const layers: StyleLayer[] = [];
    const global = ctx.scopeStyles.get(ctx.clientRoot);
    if (global !== undefined) {
        layers.push({ origin: "global", style: global.style });
    }
    const own = wsOf(client);
    const workspace = own === null ? undefined : ctx.scopeStyles.get(own.clientsState);
    if (own !== null && workspace !== undefined) {
        layers.push({ origin: `workspace:${own.id}`, style: workspace.style });
    }
    const selected = ctx.styledSelections
        .filter((state) => contains(state, client))
        .sort(
            (a, b) =>
                (a.priority ?? 0) - (b.priority ?? 0) ||
                (ctx.scopeStyles.get(a) as ScopeStyle).order - (ctx.scopeStyles.get(b) as ScopeStyle).order,
        );
    for (const state of selected) {
        layers.push({ origin: scopeOrigin(state), style: (ctx.scopeStyles.get(state) as ScopeStyle).style });
    }
    return layers;
}

/** Creates and binds the client-owned style override layer. */
export function clientStyle(client: Client): ClientStyle {
    const state = styleState(client);
    if (state.style === null) {
        const style = new ClientStyle();
        const path = (): string => `Client#${host.label(stateOf(client).node)}.style`;
        bindStyle(style, {
            path,
            below: (name) => layeredList(scopeLayers(client), name),
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

/** `scopeLayers` and the client's own overrides on top. */
export function layersOf(client: Client): StyleLayer[] {
    const own = ctx.clientStyles.get(client)?.style;
    const layers = scopeLayers(client);
    return own === null || own === undefined ? layers : [...layers, { origin: "client", style: own }];
}

/** Resolves the effective style, retaining the last result after the client is gone. */
export function computedStyleOf(client: Client): ComputedStyle {
    if (!stateOf(client).alive) {
        return ctx.clientStyles.get(client)?.computed ?? resolveStyle([]);
    }
    return resolveStyle(layersOf(client));
}

/** Reports style properties the renderer skipped for a client. */
export function styleWarningsOf(client: Client): readonly string[] {
    const notes = (ctx.clientStyles.get(client)?.notes ?? []).map((note) => `not applied: style.${note}`);
    const skipped = (host.get(stateOf(client).node, "skipped") as unknown as string[] | undefined) ?? [];
    return Object.freeze([...notes, ...skipped.map((text) => `the compositor does not draw the effect ${text}`)]);
}

/** Every style a script can have changed an array of in place. */
export function* everyStyle(): Generator<Style> {
    const scopes = [ctx.clientRoot, ...(ctx.workspaceRoot.list as Workspace[]).map((w) => wsState(w).clientsState)];
    for (const scope of [...scopes, ...ctx.styledSelections]) {
        const found = ctx.scopeStyles.get(scope);
        if (found !== undefined) {
            yield found.style;
        }
    }
    for (const client of ctx.clientRoot.list as Client[]) {
        const own = ctx.clientStyles.get(client)?.style;
        if (own !== null && own !== undefined) {
            yield own;
        }
    }
}
