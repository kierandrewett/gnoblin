// What the window menu needs of a window, a workspace and the shell (`MenuWorld`), and what an entry is. Types only,
// so the menu runs over the stock objects and over a test's fakes alike.

/**
 * One line of a menu: a `label`, and what it does. `run` runs after the menu has closed. `enabled: false` shows it dim
 * and the keyboard skips it. `checked` shows a tick. `submenu` is a list of entries (or a function that makes them)
 * that opens beside the card instead of running anything. `{ separator: true }` is a rule between groups.
 */
export interface MenuEntry {
    label?: string;
    run?: (client: MenuClient) => void;
    enabled?: boolean;
    checked?: boolean;
    submenu?: readonly MenuEntry[] | ((client: MenuClient) => readonly MenuEntry[]);
    separator?: boolean;
}

/** What the menu needs of a window. A `Client` of the host module is one. */
export interface MenuClient {
    readonly alive: boolean;
    fullscreen: boolean;
    readonly tiled: boolean;
    readonly secure?: boolean;
    readonly workspace: MenuWorkspace | null;
    maximised: boolean;
    minimised: boolean;
    close(): void;
    /**
     * Keyboard move and resize modes (`gnoblin/reposition`): a `Client` has them; for a client that does not, the two
     * entries are shown off.
     */
    moveWithKeys?(): void;
    resizeWithKeys?(): void;
}

/** One workspace as a menu entry shows it: `active` marks the workspace the user is on. */
export interface MenuWorkspace {
    readonly id: string;
    readonly name: string;
    readonly active: boolean;
}

/** What the menu needs of the shell around it. */
export interface MenuWorld {
    clientOf(node: number): MenuClient | undefined;
    workspaces(): readonly MenuWorkspace[];
    /** Puts `client` on `workspace`. */
    moveToWorkspace(client: MenuClient, workspace: MenuWorkspace): void;
    /** Every window, back to front: what "always on top" keeps the pinned ones over. */
    stack(): readonly MenuClient[];
    raise(client: MenuClient): void;
    /** Maximises or restores a window as the shell does it. Without it the `maximised` flag is written. */
    maximise?(client: MenuClient, on: boolean): void;
    /**
     * Minimises a window as the shell does it, which hands the keyboard focus on. Without it the `minimised` flag is
     * written.
     */
    minimise?(client: MenuClient): void;
    /** Whether the window sits where an edge snap put it, and the way back from there. */
    snapped?(client: MenuClient): boolean;
    unsnap?(client: MenuClient): void;
    /** Leaves fullscreen. Without it the `fullscreen` flag is written. */
    leaveFullscreen?(client: MenuClient): void;
}
