import type { WorkspaceSnapshot } from "gnoblin-core/host/contract";
/** Captures workspace membership and the workspace shown on each output. */
export declare function snapshotWorkspaces(): WorkspaceSnapshot;
/**
 * Restores window membership by workspace id. Unknown workspaces send their windows to the first workspace; windows
 * that remain on the same workspace are not marked as arrivals, so floating windows keep their positions.
 */
export declare function reconcileWorkspaces(previous: WorkspaceSnapshot): void;
