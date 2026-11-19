// A script workspace's private state: the library's handle on it, the layout object a script gave it and what that layout was
// last told, and the collection that follows its clients.
//
// The workspace itself (its windows, its place on an output, its gaps and its layout's placement of the windows) is the
// library's. What is here is what only the script's side has: the `Layout` object (a class a script extends, which the library
// reaches through `host/layout_bridge`), the live `gaps` object a script may write into, and the `MutableCollection` whose list
// `host/membership` keeps equal to the library's.

import type { Client } from "gnoblin-core/host/client";
import type { CollectionState } from "gnoblin-core/host/collection_state";
import type { MutableCollection } from "gnoblin-core/host/collections";
import type { Collection as NativeCollection, Workspace as NativeWorkspace } from "gnoblin-std/generated/classes";
import type { LayoutBridge } from "gnoblin-std/host/layout_bridge";
import { std } from "gnoblin-std/host/state";
import type { Workspace } from "gnoblin-std/host/workspace";
import type { Gaps, Layout } from "gnoblin-std/layout";

/** What the script's side keeps of one workspace. */
export interface WorkspaceState {
    /** The library's workspace. */
    readonly native: NativeWorkspace;
    /** Stable workspace id. */
    readonly id: string;
    /** Layout instance the library asks to place the workspace's clients. */
    layout: Layout;
    /** What the library asks the layout, and what it was told of the layout's own fields. */
    bridge: LayoutBridge;
    /** Inner and outer gaps in logical pixels: the object a script reads and may write into. */
    gaps: Gaps;
    /** The gaps the library was last given. */
    gapsSeen: [number, number];
    /** Whether the workspace is in `gnoblin.workspaces`, as the library last said. */
    attached: boolean;
    /** The library's collection of the workspace's clients. */
    readonly nativeClients: NativeCollection;
    /** Clients assigned to this workspace. */
    readonly clients: MutableCollection<Client>;
    /** Internal membership state for the client collection. */
    readonly clientsState: CollectionState;
}

/** Returns workspace state. Throws when the value is not a Workspace. */
export function wsState(workspace: unknown): WorkspaceState {
    const state = typeof workspace === "object" && workspace !== null ? std.wsStates.get(workspace) : undefined;
    if (!state) {
        throw new TypeError("not a Workspace");
    }
    return state;
}

let wrap: ((native: NativeWorkspace) => Workspace) | null = null;

/** Gives the module the way to make the object of a workspace the library made itself (`Workspace.wrap`): a module below `Workspace` asks for it here. */
export function attachWorkspaceWrap(next: (native: NativeWorkspace) => Workspace): void {
    wrap = next;
}

/** The `Workspace` object of a workspace the library holds: the one for its handle. */
export function workspaceOf(native: NativeWorkspace): Workspace {
    const known = std.byHandle.get(native.handle);
    if (known !== undefined) {
        return known;
    }
    if (wrap === null) {
        throw new TypeError("workspaces need `gnoblin-std/host/workspace`, which is not loaded");
    }
    return wrap(native);
}

/** Returns private workspace state for a client, or null when it has no workspace. */
export function wsOf(client: Client): WorkspaceState | null {
    const workspace = std.membership.get(client);
    return workspace ? wsState(workspace) : null;
}

/** Whether `value` is a `Workspace` (a brand check, so a module below `Workspace` can ask). */
export function isWorkspace(value: unknown): value is Workspace {
    return typeof value === "object" && value !== null && std.wsStates.has(value);
}
