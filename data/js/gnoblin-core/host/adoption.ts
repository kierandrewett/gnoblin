// Wraps compositor nodes as script objects and releases them when they disappear.

import { CREATE, host } from "gnoblin-core/host/bridge";
import { Client } from "gnoblin-core/host/client";
import { emitMember, holders, recompute } from "gnoblin-core/host/collection_events";
import type { CollectionState, Member } from "gnoblin-core/host/collection_state";
import { ctx } from "gnoblin-core/host/context";
import { queueFor } from "gnoblin-core/host/dispatch";
import { hooks } from "gnoblin-core/host/hooks";
import { freeze, stateOf } from "gnoblin-core/host/node_state";
import { Output, OUTPUT_KEYS, publishedOutput } from "gnoblin-core/host/output";
import type { OutputSeen } from "gnoblin-core/host/output";
import { disposeBindingsOf, raiseSignal } from "gnoblin-core/properties";
import type { NodeId } from "gnoblin-core/types";

/**
 * Wraps a node the first time it appears. Nodes added after startup are announced to collections and listeners.
 * Nodes already present at startup are not announced because the config attaches after they exist.
 */
export function adopt(node: NodeId, announce: boolean): void {
    const kind = host.kind(node);
    if (kind === "screen" && !ctx.outputsByNode.has(node)) {
        adoptScreen(node, announce);
        return;
    }
    if (kind === "window" && !ctx.clientsByNode.has(node)) {
        const client = new Client(CREATE, node);
        ctx.clientsByNode.set(node, client);
        ctx.clientRoot.list.push(client);
        raiseSignal(ctx.clientRoot, "items");
        if (announce) {
            queueFor(ctx.clientRoot, "onAdded", [client, ctx.clientRoot.self]);
            recompute(ctx.clientRoot, client);
            stateOf(client).readied = true;
            emitMember(client, holders(ctx.clientRoot, client), "onReady", [client]);
            hooks.clientArrived(client);
        } else {
            ctx.preexisting.add(client);
        }
    }
}

/** Wraps a screen node. Screens added after startup are announced; screens already present are not. */
function adoptScreen(node: NodeId, announce: boolean): void {
    const output = new Output(CREATE, node);
    ctx.outputsByNode.set(node, output);
    ctx.outputRoot.list.push(output);
    raiseSignal(ctx.outputRoot, "items");
    hooks.outputAdded(output);
    if (!announce) {
        return;
    }
    queueFor(ctx.outputRoot, "onAdded", [output, ctx.outputRoot.self]);
    recompute(ctx.outputRoot, output);
    emitMember(output, holders(ctx.outputRoot, output), "onConnected", [output]);
    hooks.outputAnnounced(output);
}

/** Wraps the nodes the compositor already had when the host booted: its screens first, then its windows. */
export function adoptExisting(): void {
    for (const node of host.screens()) {
        adopt(node, false);
    }
    for (const node of host.windows()) {
        adopt(node, false);
    }
}

/** Releases a destroyed screen: it leaves `gnoblin.outputs` and its listeners hear that it is gone. */
export function releaseScreen(output: Output): void {
    const held = leave(ctx.outputRoot, output, ctx.outputsByNode, OUTPUT_KEYS);
    hooks.outputLeft(output);
    emitMember(output, held, "onDisconnected", [output]);
    for (const state of held) {
        queueFor(state, "onRemoved", [output, state.self]);
    }
}

/** Queues events for changes to an output's published properties. */
export function outputChanged(output: Output): void {
    const seen = ctx.outputSeen.get(output) as OutputSeen;
    const now = publishedOutput(stateOf(output).node);
    const held = holders(ctx.outputRoot, output);
    let detached: boolean | null = null;
    if (now.detached !== seen.detached) {
        seen.detached = now.detached;
        detached = now.detached;
    }
    hooks.outputChange(output, detached);
    if (now.mode !== seen.mode) {
        seen.mode = now.mode;
        emitMember(output, held, "onModeChanged", [output, now.mode]);
    }
    if (now.scale !== seen.scale) {
        seen.scale = now.scale;
        emitMember(output, held, "onScaleChanged", [output, now.scale]);
    }
    if (now.x !== seen.x || now.y !== seen.y || now.width !== seen.width || now.height !== seen.height) {
        const { x, y, width, height } = now;
        Object.assign(seen, { x, y, width, height });
        emitMember(output, held, "onGeometryChanged", [output, { x, y, width, height }]);
    }
}

/** Removes a destroyed member from its collections after saving its final readable values. */
export function leave(
    top: CollectionState,
    member: Member,
    map: Map<NodeId, Member>,
    keys: string[],
): CollectionState[] {
    const node = stateOf(member).node;
    const held = holders(top, member);
    freeze(member, keys);
    map.delete(node);
    top.list.splice(top.list.indexOf(member), 1);
    raiseSignal(top, "items");
    disposeBindingsOf(member);
    for (const state of held) {
        state.members?.delete(member);
    }
    return held;
}
