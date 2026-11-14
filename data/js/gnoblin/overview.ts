// The stock overview: while it is on, every workspace is shown at full strength; while it is off, all but the active
// workspace are dimmed. A workspace is the script's, so what is dimmed is the opacity of its windows.
//
// It owns the on/off flag and the opacity writes. It does not draw thumbnails or move windows. Read
// `gnoblin/workspace` for what a workspace is.

/** What the overview needs of a workspace. A `Workspace` of the host module is one. */
export interface OverviewWorkspace {
    readonly active: boolean;
    readonly clients: Iterable<{ opacity: number }>;
}

/** How strong a window of a workspace that is not shown looks when the overview is off (0 is clear, 1 is full). */
export const DIMMED_OPACITY = 0.4;

/** The overview's switch. `toggle()` flips it and sets the opacity of every window of every workspace to match. */
export class Overview {
    #on = false;
    readonly #workspaces: () => Iterable<OverviewWorkspace>;

    /** `workspaces` is called on each toggle, so workspaces made later are covered. */
    constructor(workspaces: () => Iterable<OverviewWorkspace>) {
        this.#workspaces = workspaces;
    }

    /** Whether the overview is on. */
    get on(): boolean {
        return this.#on;
    }

    /** Turns the overview on or off, sets the strength of every window to match, and returns whether it is on. */
    toggle(): boolean {
        this.#on = !this.#on;
        for (const workspace of this.#workspaces()) {
            const opacity = this.#on || workspace.active ? 1 : DIMMED_OPACITY;
            for (const client of workspace.clients) {
                client.opacity = opacity;
            }
        }
        return this.#on;
    }
}

/** The host module's global, which is there before any stock module runs. */
declare const gnoblin: { readonly workspaces: Iterable<OverviewWorkspace> };

/** The shell's overview, over `gnoblin.workspaces`. */
export const overview = new Overview(() => gnoblin.workspaces);
