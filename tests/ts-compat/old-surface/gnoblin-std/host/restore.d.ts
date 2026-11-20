/**
 * Notes the workspaces as they are now; the function it returns puts them back. A layout a callback assigned (and so was not the
 * workspace's layout when this was called) keeps the field changes made to it.
 */
export declare function snapshotWorkspaceState(): () => void;
