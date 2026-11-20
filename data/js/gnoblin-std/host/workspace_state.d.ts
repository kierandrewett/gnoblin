import type { Client } from "gnoblin-core/host/client";
import type { CollectionState } from "gnoblin-core/host/collection_state";
import type { MutableCollection } from "gnoblin-core/host/collections";
import type { Collection as NativeCollection, Workspace as NativeWorkspace } from "gnoblin-std/generated/classes";
import type { LayoutBridge } from "gnoblin-std/host/layout_bridge";
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
export declare function wsState(workspace: unknown): WorkspaceState;
/** Gives the module the way to make the object of a workspace the library made itself (`Workspace.wrap`): a module below `Workspace` asks for it here. */
export declare function attachWorkspaceWrap(next: (native: NativeWorkspace) => Workspace): void;
/** The `Workspace` object of a workspace the library holds: the one for its handle. */
export declare function workspaceOf(native: NativeWorkspace): Workspace;
/** Returns private workspace state for a client, or null when it has no workspace. */
export declare function wsOf(client: Client): WorkspaceState | null;
/** Whether `value` is a `Workspace` (a brand check, so a module below `Workspace` can ask). */
export declare function isWorkspace(value: unknown): value is Workspace;
