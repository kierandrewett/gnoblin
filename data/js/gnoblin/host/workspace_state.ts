// A script workspace's private state: its id, layout and gaps, its clients, and what its layout was last told.
// A workspace is the script's own object (the compositor does not know it), so this state is all there is.

import type { Client } from "gnoblin/host/client";
import type { CollectionState } from "gnoblin/host/collection_state";
import type { MutableCollection } from "gnoblin/host/collections";
import { ctx } from "gnoblin/host/context";
import type { Workspace } from "gnoblin/host/workspace";
import type { Gaps, Layout } from "gnoblin/layout";
import type { Rect } from "gnoblin/types";

/** Layout and membership state held privately for each script workspace. */
export interface WorkspaceState {
    /** Stable workspace id. */
    id: string;
    /** Display name. */
    name: string;
    /** Layout instance used to place the workspace's clients. */
    layout: Layout;
    /** Inner and outer gaps in logical pixels. */
    gaps: Gaps;
    /** Whether the workspace is in `gnoblin.workspaces`. */
    attached: boolean;
    /** Clients assigned to this workspace. */
    clients: MutableCollection<Client>;
    /** Internal membership state for the client collection. */
    clientsState: CollectionState;
    /** The clients that joined since the layout last heard of them, for `onClientAdded`. */
    arrived: Client[];
    /** Clients that left since the layout last heard of them. */
    departed: Client[];
    /** Whether the layout has to compute again the next time the workspace is shown. */
    dirty: boolean;
    /** The layout's own fields when the workspace was last looked at, to notice a change. */
    fields: Map<PropertyKey, unknown> | null;
    /** Gap values most recently observed by the layout. */
    gapsSeen: [number, number];
    /** The area the layout last computed for. */
    area: Rect | null;
    /** The focused client the layout last computed for: a change of focus is a reason to compute again. */
    focusSeen: Client | null;
    /** The fullscreen and maximised clients the layout last computed for: a change is a reason to compute again. */
    coverSeen: string;
}

/** Returns workspace state. Throws when the value is not a Workspace. */
export function wsState(workspace: unknown): WorkspaceState {
    const state = typeof workspace === "object" && workspace !== null ? ctx.wsStates.get(workspace) : undefined;
    if (!state) {
        throw new TypeError("not a Workspace");
    }
    return state;
}

/** Returns private workspace state for a client, or null when it has no workspace. */
export function wsOf(client: Client): WorkspaceState | null {
    const workspace = ctx.membership.get(client);
    return workspace ? wsState(workspace) : null;
}

/** Whether `value` is a `Workspace` (a brand check, so a module below `Workspace` can ask). */
export function isWorkspace(value: unknown): value is Workspace {
    return typeof value === "object" && value !== null && ctx.wsStates.has(value);
}
