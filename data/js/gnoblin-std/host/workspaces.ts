// Adds and removes workspaces: the calls a script makes on `gnoblin.workspaces`, answered by the library, which keeps the list,
// gives a new workspace the windows that wait for one, and moves the windows of a removed one to a neighbour.

import { FAILED, throughLibrary } from "gnoblin-std/host/library";
import { syncMembership } from "gnoblin-std/host/membership";
import { std } from "gnoblin-std/host/state";
import { Workspace } from "gnoblin-std/host/workspace";
import { wsState } from "gnoblin-std/host/workspace_state";
import { Collection as NativeCollection } from "gnoblin-std/generated/classes";
import { CollectionKind } from "gnoblin-std/generated/constants";

/** The library's list of workspaces. */
function nativeList(): NativeCollection {
    const list = NativeCollection.root(CollectionKind.WORKSPACES);
    if (list === null) {
        throw new TypeError("the library has no list of workspaces");
    }
    return list;
}

/** Attaches a workspace to the public root; the library gives it any windows that are waiting for one. */
export function addWorkspace(workspace: Workspace): void {
    throughLibrary("gnoblin.workspaces.add", { [FAILED.busy]: "already_exists" }, () =>
        wsState(workspace).native.add(),
    );
    syncMembership();
}

/**
 * Takes the workspace out of `gnoblin.workspaces`. Its windows move to the neighbour before it (the one after it,
 * for the first), and that neighbour is shown where the removed one was; none is closed. The last workspace stays.
 */
export function removeWorkspace(workspace: Workspace): boolean {
    const removed = wsState(workspace).native.remove();
    syncMembership();
    return removed;
}

/** Takes every workspace out of `gnoblin.workspaces`, closing no window. */
export function clearWorkspaces(): void {
    nativeList().clear();
    syncMembership();
}

/**
 * A script that declared no workspace gets one floating `desktop`, so there is always somewhere for a window to go. The
 * stock default's four workspaces are declared by `data/js/starter/shell.ts`; a user's `shell.ts` that declares none gets this one.
 */
export function ensureDefaultWorkspace(): void {
    syncMembership();
    if (std.workspaceRoot.list.length > 0) {
        return;
    }
    addWorkspace(new Workspace({ id: "desktop" }));
}
