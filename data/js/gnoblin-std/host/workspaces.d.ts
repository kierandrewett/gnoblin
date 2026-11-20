import { Workspace } from "gnoblin-std/host/workspace";
/** Attaches a workspace to the public root; the library gives it any windows that are waiting for one. */
export declare function addWorkspace(workspace: Workspace): void;
/**
 * Takes the workspace out of `gnoblin.workspaces`. Its windows move to the neighbour before it (the one after it,
 * for the first), and that neighbour is shown where the removed one was; none is closed. The last workspace stays.
 */
export declare function removeWorkspace(workspace: Workspace): boolean;
/** Takes every workspace out of `gnoblin.workspaces`, closing no window. */
export declare function clearWorkspaces(): void;
/**
 * A script that declared no workspace gets one floating `desktop`, so there is always somewhere for a window to go. The
 * stock default's four workspaces are declared by `shells/default/shell.ts`; a user's `shell.ts` that declares none gets this one.
 */
export declare function ensureDefaultWorkspace(): void;
