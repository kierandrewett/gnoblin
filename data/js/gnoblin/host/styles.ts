// Applies resolved client styles to host nodes and reports effects the compositor cannot draw.

import { errorCodeOfRefusal, GnoblinError } from "gnoblin/errors";
import { host } from "gnoblin/host/bridge";
import type { Client } from "gnoblin/host/client";
import { ctx } from "gnoblin/host/context";
import { guard, say } from "gnoblin/host/dispatch";
import { stateOf } from "gnoblin/host/node_state";
import { everyStyle, layersOf, styleState } from "gnoblin/host/style_scopes";
import type { ClientStyleState } from "gnoblin/host/style_scopes";
import { syncStyleLists } from "gnoblin/style/lists";
import { mapStyle } from "gnoblin/style/map";
import { resolveStyle } from "gnoblin/style/resolve";
import { styleEpoch } from "gnoblin/style/state";
import type { StyleLayer } from "gnoblin/style/resolve";
import type { HostValue } from "gnoblin/types";

/**
 * Maximum effects supported on one node. A style can contain up to this many shadows and borders combined,
 * with a backdrop blur counting as one effect.
 */
const MAX_NODE_EFFECTS = 16;

/** Writes a client's resolved effects, opacity and corner radius to its node when they change. */
function applyClientStyle(client: Client, state: ClientStyleState, layers: StyleLayer[]): void {
    const node = stateOf(client).node;
    const label = `Client#${host.label(node)}.style`;
    const computed = resolveStyle(layers);
    state.computed = computed;
    const mapping = mapStyle(computed);
    state.notes = mapping.unapplied;
    for (const text of mapping.unapplied) {
        say(`not applied: style.${text}`);
    }
    const refused = (what: string, code: string | undefined | null): void => {
        if (code != null && code !== "unknown-node") {
            ctx.reports.push({
                site: label,
                error: new GnoblinError(errorCodeOfRefusal(code), label, `the host refused the ${what} (${code})`),
            });
        }
    };
    if (mapping.effects.length > MAX_NODE_EFFECTS) {
        const error = new RangeError(
            `${label}: a window carries at most ${MAX_NODE_EFFECTS} effects, the style makes ${mapping.effects.length}`,
        );
        state.notes = [...state.notes, error.message];
        ctx.reports.push({ site: label, error });
    } else {
        const json = JSON.stringify(mapping.effects);
        // A window an earlier generation styled keeps its effects: the first application clears them when the style
        // has none, which is how a line removed from the config takes its effect away on reload.
        const published = host.published(node, "effects") as unknown as unknown[] | undefined;
        const unchanged =
            state.effects === null
                ? mapping.effects.length === 0 && (published ?? []).length === 0
                : json === state.effects;
        if (!unchanged) {
            refused("effects", host.set(node, "effects", mapping.effects as unknown as HostValue));
        }
        state.effects = json;
    }
    const before = state.opacity ?? (host.published(node, "opacity") as number);
    if (computed.opacity !== before) {
        refused("opacity", host.set(node, "opacity", computed.opacity));
    }
    state.opacity = computed.opacity;
    const radiusBefore = state.cornerRadius ?? (host.published(node, "corner_radius") as number);
    if (mapping.cornerRadius !== radiusBefore) {
        refused("corner radius", host.set(node, "corner_radius", mapping.cornerRadius));
    }
    state.cornerRadius = mapping.cornerRadius;
}

/** Checks mutable style arrays, then applies resolved styles for clients whose style or inherited layers changed. */
export function applyStyles(): void {
    for (const style of everyStyle()) {
        for (const error of syncStyleLists(style)) {
            ctx.reports.push({ site: "gnoblin.style", error });
        }
    }
    const epoch = styleEpoch();
    for (const client of ctx.clientRoot.list as Client[]) {
        const state = styleState(client);
        const layers = layersOf(client);
        const key = layers.map((layer) => layer.origin).join("\n");
        if (state.applied && state.epoch === epoch && state.layers === key) {
            continue;
        }
        state.applied = true;
        state.epoch = epoch;
        state.layers = key;
        guard(`Client#${host.label(stateOf(client).node)}.style`, () => applyClientStyle(client, state, layers));
    }
}
