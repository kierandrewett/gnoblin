// What `gnoblin-std` does at the points where the object model of `gnoblin-core` changes: it gives a new window a
// workspace, an output a desk, and a closing window's workspace its place back. These are the hooks core defines
// (`gnoblin-core/host/hooks`); `gnoblin-std/host/install` hands them over.

import { host } from "gnoblin-core/host/bridge";
import type { Client } from "gnoblin-core/host/client";
import { holders } from "gnoblin-core/host/collection_events";
import { ctx } from "gnoblin-core/host/context";
import type { HostHooks } from "gnoblin-core/host/hooks";
import type { Output } from "gnoblin-core/host/output";
import { focusedClient } from "gnoblin-core/host/seat";
import { raiseSignal } from "gnoblin-core/properties";
import { isShown, fillDesks } from "gnoblin-std/host/desks";
import { refitClient } from "gnoblin-std/host/fitting";
import { dialogsOf, forgetArrival, moveClient, placeDialog } from "gnoblin-std/host/membership";
import { layout } from "gnoblin-std/host/layout_driver";
import { declareDecorations } from "gnoblin-std/host/services";
import { reconcileWorkspaces, snapshotWorkspaces } from "gnoblin-std/host/snapshot";
import { std } from "gnoblin-std/host/state";
import { areaOfDesk, workAreaOf } from "gnoblin-std/host/work_area";
import type { Workspace } from "gnoblin-std/host/workspace";
import { wsOf, wsState } from "gnoblin-std/host/workspace_state";
import { adoptOrphans, ensureDefaultWorkspace, newWindowWorkspace } from "gnoblin-std/host/workspaces";
import { lockRelease, runLockFrame } from "gnoblin-std/lock";
import { sceneForget, sceneNodePointer, sceneReleaseAll } from "gnoblin-std/scene/registry";
import { uiNodeGone, uiReleaseAll } from "gnoblin-std/ui/frame";
import { windowState } from "gnoblin-std/window_state";

/** The outputs whose desk was parked when they were added: they get a window focused when their announcement ends. */
const returned = new WeakSet<Output>();

/**
 * Gives an output a desk, reusing one without an output or creating one to show an available workspace.
 * Returns whether the reused desk was parked.
 */
function giveDesk(output: Output): boolean {
    let desk = std.desks.find((candidate) => candidate.output === null);
    if (desk === undefined) {
        desk = { output: null, shown: null, parked: false, spans: null, spannedBy: null };
        std.desks.push(desk);
    }
    const wasParked = desk.parked;
    desk.output = output;
    desk.parked = false;
    std.deskByOutput.set(output, desk);
    fillDesks();
    if (desk.shown !== null) {
        wsState(desk.shown).dirty = true;
    }
    return wasParked;
}

/** Takes an output's desk away: its windows move to another output's workspace, or the desk parks. */
function takeDesk(output: Output): void {
    const desk = std.deskByOutput.get(output);
    if (desk !== undefined) {
        desk.output = null;
        const survivor = std.desks.find((candidate) => candidate !== desk && candidate.output !== null);
        const shown = desk.shown;
        if (survivor !== undefined) {
            std.desks.splice(std.desks.indexOf(desk), 1);
            const target = survivor.shown ?? (std.workspaceRoot.list[0] as Workspace | undefined) ?? null;
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
            if (holder !== null && shown !== null && std.membership.get(holder) === shown) {
                ctx.input.primary.clearFocus();
            }
        }
    }
    for (const other of std.desks) {
        if (other.spannedBy === desk) {
            other.spannedBy = null;
        }
    }
    std.deskByOutput.delete(output);
}

/**
 * Hands the keyboard to the neighbour of a focused window that closed, as the retired strip did for its columns and
 * every layout now does: the window after it in the workspace, else the one before; a dialog hands it back to its
 * parent. Nothing moves while the workspace is not shown, a minimised window is skipped, and a window the compositor
 * has already focused is left alone. A layout that has its own `onClientRemoved` (a strip's columns are not in window
 * order) does it itself.
 */
function handOffFocus(workspace: Workspace, index: number, parent: Client | null): void {
    if (!isShown(workspace) || host.input.seats().length === 0 || focusedClient() !== null) {
        return;
    }
    const list = wsState(workspace).clientsState.list as Client[];
    const usable = (client: Client | undefined): client is Client =>
        client !== undefined && client.alive && !client.minimised;
    const next = [parent ?? undefined, list[index], list[index - 1]].find(usable);
    next?.focus();
}

/** The hooks of the desktop policy. */
export const lifecycle: Partial<HostHooks> = {
    // New windows join the workspace shown under the pointer. Dialogs join their parent's workspace and are centred.
    clientArrived(client) {
        if (ctx.flags.opened && !placeDialog(client)) {
            const target = newWindowWorkspace();
            if (target !== null) {
                moveClient(client, target, true);
            }
        }
    },

    clientRoots(client) {
        const own = wsOf(client);
        return own ? [own.clientsState] : [];
    },

    sharesStack: (a, b) => (std.membership.get(a) ?? null) === (std.membership.get(b) ?? null),

    clientChange(client, change) {
        if (change.decoration) {
            // The frame's room round the window came or went: the workspace's layout places it again.
            const own = wsOf(client);
            if (own) {
                own.dirty = true;
            }
        }
        if (change.moved !== null) {
            // A dialog moves with its parent.
            for (const dialog of dialogsOf(client)) {
                dialog.x += change.moved.dx;
                dialog.y += change.moved.dy;
            }
        }
        if (change.parent) {
            placeDialog(client);
        }
    },

    clientReleased(client, parent) {
        const own = wsOf(client);
        if (!own) {
            return;
        }
        const wasFocused = std.flags.lastFocus === client;
        const ownHeld = holders(own.clientsState, client);
        const workspace = std.membership.get(client) as Workspace;
        std.membership.delete(client);
        const index = own.clientsState.list.indexOf(client);
        own.clientsState.list.splice(index, 1);
        raiseSignal(own.clientsState, "items");
        own.dirty = true;
        if (wasFocused && typeof own.layout.onClientRemoved !== "function") {
            handOffFocus(workspace, index, parent);
        }
        forgetArrival(own, client);
        for (const state of ownHeld) {
            state.members?.delete(client);
        }
    },

    // Mirrors and disabled outputs have no desk, so the layout does not place windows on them.
    outputAdded(output) {
        std.displayManager.changed();
        if (ctx.outputSeen.get(output)?.detached !== true && giveDesk(output)) {
            returned.add(output);
        }
    },

    // Restore windows that waited hidden, then focus one if a seat exists and none has focus.
    outputAnnounced(output) {
        const wasParked = returned.delete(output);
        const desk = std.deskByOutput.get(output);
        if (
            wasParked &&
            desk !== undefined &&
            desk.shown !== null &&
            host.input.seats().length > 0 &&
            focusedClient() === null
        ) {
            const first = (wsState(desk.shown).clientsState.list as Client[]).find(
                (one) => one.alive && !one.minimised,
            );
            first?.focus();
        }
    },

    /**
     * When a screen disappears, its windows move to another output's workspace and are fitted there. With no other
     * output, they wait hidden on a parked desk, and focus is cleared because no output can hold it.
     */
    outputLeft(output) {
        std.displayManager.changed();
        takeDesk(output);
    },

    outputChange(output, detached) {
        std.displayManager.changed();
        if (detached === true) {
            takeDesk(output);
        } else if (detached === false) {
            giveDesk(output);
        }
    },

    nodeDestroyed: (node) => uiNodeGone(node),

    nodesDestroyed: (nodes) => sceneForget(nodes),

    nodePointer: (node, kind, point) => sceneNodePointer(node, kind, point),

    pointerEdge: (kind) => windowState.pointerEdge(kind),

    finishLoad: ensureDefaultWorkspace,

    opened() {
        adoptOrphans();
        // The declaration is written here, after the generation that ended took its own back: a reload stages the new
        // generation before it retires the old one, so a write made while staging would be undone by that retire.
        if (std.decorations !== null) {
            declareDecorations(true);
        }
    },

    layout,

    ui() {
        runLockFrame(
            (site, error) => ctx.reports.push({ site, error }),
            () => {
                try {
                    std.xrRuntime.frame();
                } catch (error) {
                    ctx.reports.push({ site: "XRRuntime.frame", error });
                }
            },
        );
    },

    inputDelivered() {
        try {
            std.focusPolicy?.frame();
        } catch (error) {
            ctx.reports.push({ site: "FocusPolicy.frame", error });
        }
    },

    release() {
        std.xrRuntime.release();
        sceneReleaseAll();
        lockRelease();
        uiReleaseAll();
        if (std.decorations !== null) {
            declareDecorations(false);
        }
    },

    snapshot: snapshotWorkspaces,

    reconcile: reconcileWorkspaces,

    workspaces: () => [...std.workspaceRoot.list],

    workArea: (client) => workAreaOf(client ?? null),

    layouts: () => std.workspaceRoot.list.map((workspace) => wsState(workspace as Workspace).layout),
};
