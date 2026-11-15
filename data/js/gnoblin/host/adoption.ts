// Adopting the compositor's nodes: a window becomes a `Client`, a screen an `Output` with a desk of its own, and
// the reverse when one goes away.

import { CREATE, host } from "gnoblin/host/bridge";
import { Client } from "gnoblin/host/client";
import { emitMember, holders, recompute } from "gnoblin/host/collection_events";
import type { CollectionState, Member } from "gnoblin/host/collection_state";
import { ctx } from "gnoblin/host/context";
import { fillDesks } from "gnoblin/host/desks";
import { queueFor } from "gnoblin/host/dispatch";
import { refitClient } from "gnoblin/host/fitting";
import { moveClient, placeDialog } from "gnoblin/host/membership";
import { freeze, stateOf } from "gnoblin/host/node_state";
import { Output, OUTPUT_KEYS, publishedOutput } from "gnoblin/host/output";
import type { OutputSeen } from "gnoblin/host/output";
import { focusedClient } from "gnoblin/host/seat";
import { areaOfDesk } from "gnoblin/host/work_area";
import type { Workspace } from "gnoblin/host/workspace";
import { wsState } from "gnoblin/host/workspace_state";
import { newWindowWorkspace } from "gnoblin/host/workspaces";
import { disposeBindingsOf, raiseSignal } from "gnoblin/properties";
import type { NodeId } from "gnoblin/types";

/**
 * Wraps a node the first time it is seen. A node that appears while the shell runs is announced: its
 * collections are told it was added and its listeners that it is ready. A node that was there at boot is
 * not an event, because the config attaches after it exists.
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
            // A window that appears joins the workspace the output under the pointer shows; a dialog joins its parent's
            // and is centred in it.
            if (ctx.flags.opened && !placeDialog(client)) {
                const target = newWindowWorkspace();
                if (target !== null) {
                    moveClient(client, target, true);
                }
            }
        } else {
            ctx.preexisting.add(client);
        }
    }
}

/**
 * Wraps a screen node the first time it is seen and gives it a desk: the output-less desk a tree kept (or a parked one
 * that waited for it), else a new one that shows the first workspace no desk shows. A screen that appears while the
 * shell runs is announced; one that was there at boot is not, because the config attaches after it exists.
 */
function adoptScreen(node: NodeId, announce: boolean): void {
    ctx.displayManager.changed();
    const output = new Output(CREATE, node);
    ctx.outputsByNode.set(node, output);
    ctx.outputRoot.list.push(output);
    raiseSignal(ctx.outputRoot, "items");
    // A mirror and an output that is off have no place in the desktop, so they get no desk: nothing is laid out there.
    const wasParked = (ctx.outputSeen.get(output) as OutputSeen).detached ? false : giveDesk(output);
    if (!announce) {
        return;
    }
    queueFor(ctx.outputRoot, "onAdded", [output, ctx.outputRoot.self]);
    recompute(ctx.outputRoot, output);
    emitMember(output, holders(ctx.outputRoot, output), "onConnected", [output]);
    const desk = ctx.deskByOutput.get(output);
    // The windows that waited hidden come back, and one takes the keyboard when nobody holds it.
    if (
        wasParked &&
        desk !== undefined &&
        desk.shown !== null &&
        host.input.seats().length > 0 &&
        focusedClient() === null
    ) {
        const first = (wsState(desk.shown).clientsState.list as Client[]).find((one) => one.alive && !one.minimised);
        first?.focus();
    }
}

/**
 * Gives an output a desk: the output-less desk a tree kept (or a parked one that waited for it), else a new one that
 * shows the first workspace no desk shows. Returns whether the desk was parked.
 */
function giveDesk(output: Output): boolean {
    let desk = ctx.desks.find((candidate) => candidate.output === null);
    if (desk === undefined) {
        desk = { output: null, shown: null, parked: false, spans: null, spannedBy: null };
        ctx.desks.push(desk);
    }
    const wasParked = desk.parked;
    desk.output = output;
    desk.parked = false;
    ctx.deskByOutput.set(output, desk);
    fillDesks();
    if (desk.shown !== null) {
        wsState(desk.shown).dirty = true;
    }
    return wasParked;
}

/**
 * A screen went away. Its windows move to the workspace another output shows (the retired shell's `Screen:on_destroyed`),
 * fitted to that output; with no other output they wait hidden on the parked desk for the next one, and the focus is
 * dropped, because a window with no output cannot hold it.
 */
export function releaseScreen(output: Output): void {
    ctx.displayManager.changed();
    const held = leave(ctx.outputRoot, output, ctx.outputsByNode, OUTPUT_KEYS);
    takeDesk(output);
    emitMember(output, held, "onDisconnected", [output]);
    for (const state of held) {
        queueFor(state, "onRemoved", [output, state.self]);
    }
}

/** Takes an output's desk away: its windows move to the workspace another output shows, or the desk parks. */
function takeDesk(output: Output): void {
    const desk = ctx.deskByOutput.get(output);
    if (desk !== undefined) {
        desk.output = null;
        const survivor = ctx.desks.find((candidate) => candidate !== desk && candidate.output !== null);
        const shown = desk.shown;
        if (survivor !== undefined) {
            ctx.desks.splice(ctx.desks.indexOf(desk), 1);
            const target = survivor.shown ?? (ctx.workspaceRoot.list[0] as Workspace | undefined) ?? null;
            if (shown !== null && target !== null && shown !== target) {
                const area = areaOfDesk(survivor);
                for (const client of [...wsState(shown).clientsState.list] as Client[]) {
                    if (client.parentWindow === null) {
                        moveClient(client, target, true);
                        if (area !== null) {
                            refitClient(client, area);
                        }
                    }
                }
            }
            if (shown !== null) {
                wsState(shown).dirty = true;
            }
        } else {
            desk.parked = true;
            const holder = host.input.seats().length > 0 ? focusedClient() : null;
            if (holder !== null && shown !== null && ctx.membership.get(holder) === shown) {
                ctx.input.primary.clearFocus();
            }
        }
    }
    for (const other of ctx.desks) {
        if (other.spannedBy === desk) {
            other.spannedBy = null;
        }
    }
    ctx.deskByOutput.delete(output);
}

/** Raises the events one output's changed properties imply, to the listeners that held it. */
export function outputChanged(output: Output): void {
    ctx.displayManager.changed();
    const seen = ctx.outputSeen.get(output) as OutputSeen;
    const now = publishedOutput(stateOf(output).node);
    const held = holders(ctx.outputRoot, output);
    if (now.detached !== seen.detached) {
        seen.detached = now.detached;
        if (now.detached) {
            takeDesk(output);
        } else {
            giveDesk(output);
        }
    }
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

/** Removes a destroyed member from its collections, after freezing its last values. */
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
