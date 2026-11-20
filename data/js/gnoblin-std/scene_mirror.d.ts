type NodeId = number;
/** How a picture is fitted to the View's rectangle. */
export type ContentFit = "contain" | "cover" | "stretch";
/** One client of the mirrored workspace: its tree node and its rectangle in the compositor's logical pixels. */
export interface MirroredClient {
    /** Scene node id used as the client's source. */
    node: NodeId;
    /** Client position in workspace logical pixels. */
    x: number;
    /** Client position in workspace logical pixels. */
    y: number;
    /** Client width in logical pixels. */
    width: number;
    /** Client height in logical pixels. */
    height: number;
    /** Whether the source node is shown. */
    visible: boolean;
}
/** What the mirror reads of a workspace. Every read goes through the tracked getters, so it is followed. */
export interface WorkspaceSource {
    /** The clients on the workspace now, in stacking order. */
    clients(): MirroredClient[];
    /** The rectangle the workspace fills (its output), or null while no output shows it. */
    area(): {
        x: number;
        y: number;
        width: number;
        height: number;
    } | null;
}
/** The tree operations the mirror needs. The scene's backend provides them. */
export interface MirrorTree {
    /** Creates a scene node of class `cls`, or returns a refusal message. */
    create(cls: string): NodeId | string;
    /** Inserts `node` under `parent` at `index`, or returns a refusal message. */
    insert(node: NodeId, parent: NodeId, index: number): string | undefined;
    /** Destroys a node, or returns a refusal message. */
    destroy(node: NodeId): string | undefined;
    /** Reads a node property. */
    get(node: NodeId, key: string): unknown;
    /** Writes a node property, or returns a refusal message. */
    set(node: NodeId, key: string, value: number | boolean | number[]): string | undefined;
}
/** The scale and the offset that fit `area` into `size` under `fit`. A size with no area or no extent is left as is. */
export declare function fitScale(fit: ContentFit, area: {
    width: number;
    height: number;
}, size: {
    width: number;
    height: number;
}): {
    scale: [number, number, number];
    x: number;
    y: number;
};
/** Keeps the tree under one View node in step with one workspace. */
export declare class WorkspaceMirror {
    #private;
    /**
     * Starts mirroring `source` under `view`. `owner` is the script's View, whose death ends the mirror; `site` names
     * it in a report; `fit` is read on every run, and the caller raises the signal named `fit` on `owner` when it
     * changes.
     */
    constructor(tree: MirrorTree, source: WorkspaceSource, view: NodeId, owner: object, site: string, fit: () => ContentFit);
    /** Ends the mirror and removes the nodes it made. */
    stop(): void;
}
export {};
