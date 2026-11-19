// What `gnoblin-std` does at the points where the object model of `gnoblin-core` changes. A new window's workspace, an output's
// desk, the keyboard when a window closes and a dialog that follows its parent are the library's: it watches the tree itself
// (`libgnoblin-std`'s events) and decides. What is left here is what only the engine knows: the collections a window is in
// for its listeners, the frame's own work, and what a reload keeps. These are the hooks core defines
// (`gnoblin-core/host/hooks`); `gnoblin-std/host/install` hands them over.

import { ctx } from "gnoblin-core/host/context";
import type { HostHooks } from "gnoblin-core/host/hooks";
import { stateOf } from "gnoblin-core/host/node_state";
import { clearInheritedFrames, layout } from "gnoblin-std/host/frame";
import { forgetClient } from "gnoblin-std/host/membership";
import { declareDecorations } from "gnoblin-std/host/services";
import { reconcileWorkspaces, snapshotWorkspaces } from "gnoblin-std/host/snapshot";
import { std } from "gnoblin-std/host/state";
import { workAreaOf } from "gnoblin-std/host/work_area";
import type { Workspace } from "gnoblin-std/host/workspace";
import { wsOf, wsState } from "gnoblin-std/host/workspace_state";
import { ensureDefaultWorkspace } from "gnoblin-std/host/workspaces";
import { lockRelease, runLockFrame } from "gnoblin-std/lock";
import { sceneForget, sceneNodePointer, sceneReleaseAll } from "gnoblin-std/scene/registry";
import { uiNodeGone, uiReleaseAll } from "gnoblin-std/ui/frame";
import { windowState } from "gnoblin-std/window_state";

/** The hooks of the desktop policy. */
export const lifecycle: Partial<HostHooks> = {
    clientRoots(client) {
        const own = wsOf(client);
        return own ? [own.clientsState] : [];
    },

    sharesStack: (a, b) => (std.membership.get(a) ?? null) === (std.membership.get(b) ?? null),

    // The library hears of a closing window with the compositor's word, a turn after the engine does: until then the window is
    // still on its workspace there, and a layout told of its leaving is given the object it knew.
    clientReleased(client) {
        std.closed.set(stateOf(client).node, { client, frame: std.frame });
        forgetClient(client);
    },

    nodeDestroyed: (node) => uiNodeGone(node),

    nodesDestroyed: (nodes) => sceneForget(nodes),

    nodePointer: (node, kind, point) => sceneNodePointer(node, kind, point),

    pointerEdge: (kind) => windowState.pointerEdge(kind),

    finishLoad() {
        clearInheritedFrames();
        ensureDefaultWorkspace();
    },

    opened() {
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

    // What an earlier generation left in the library has no bindings of this generation's.
    layouts: () =>
        std.workspaceRoot.list
            .filter((workspace) => !std.inherited.has(workspace as Workspace))
            .map((workspace) => wsState(workspace as Workspace).layout),
};
