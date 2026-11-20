import type { Client } from "gnoblin-core/host/client";
import type { Workspace } from "gnoblin-std/host/workspace";
/**
 * Makes every list of windows the library's. A window leaves its old workspace's list before it joins the new one's, as a
 * move always told them. A window the engine has not seen is not in the list yet, and the next call finds it.
 */
export declare function syncMembership(): void;
/** A window closed: it leaves its workspace's list at once, as the compositor's word of it reaches the engine first. */
export declare function forgetClient(client: Client): void;
/** Makes the moves that waited for the library to know their window. */
export declare function flushMoves(): void;
/** `client.moveToWorkspace(workspace)`: the retired shell's `Window:move_to`. */
export declare function moveClientTo(client: Client, workspace: Workspace): boolean;
/** `workspace.clients.add(client)`: the client leaves its workspace and joins this one. */
export declare function transferClient(workspace: Workspace, client: Client): void;
/** `workspace.clients.remove(client)`: the client has no workspace until it is given one. */
export declare function detachClient(workspace: Workspace, client: Client): void;
/** Removes every client from a workspace without closing the windows. */
export declare function clearClients(workspace: Workspace): void;
/** A workspace's clients traded places: its layout computes again, and one with a record of its own trades them there. */
export declare function swapClients(workspace: Workspace, a: Client, b: Client): void;
