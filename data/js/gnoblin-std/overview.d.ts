/** What the overview needs of a workspace. A `Workspace` of the host module is one. */
export interface OverviewWorkspace {
    /** Whether this workspace is currently active. */
    readonly active: boolean;
    /** Windows whose opacity changes when the overview toggles. */
    readonly clients: Iterable<{
        opacity: number;
    }>;
}
/** How strong a window of a workspace that is not shown looks when the overview is off (0 is clear, 1 is full). */
export declare const DIMMED_OPACITY = 0.4;
/** The overview's switch. `toggle()` flips it and sets the opacity of every window of every workspace to match. */
export declare class Overview {
    #private;
    /** `workspaces` is called on each toggle, so workspaces made later are covered. */
    constructor(workspaces: () => Iterable<OverviewWorkspace>);
    /** Whether the overview is on. */
    get on(): boolean;
    /** Turns the overview on or off, sets the strength of every window to match, and returns whether it is on. */
    toggle(): boolean;
}
/** The shell's overview, over `gnoblin.workspaces`. */
export declare const overview: Overview;
