// The window menu's entry, window and shell types. These let it work with live objects or test doubles.

/**
 * Describes one menu entry. `run` runs after the menu closes; disabled entries are dimmed and skipped by keyboard
 * selection. `checked` shows a tick. `submenu` opens a list beside the card. `{ separator: true }` separates groups.
 */
export interface MenuEntry {
    /** The entry's displayed text. */
    label?: string;
    /** Runs after the menu closes, with the selected window. */
    run?: (client: MenuClient) => void;
    /** Whether the entry can be selected or run. Defaults to `true`. */
    enabled?: boolean;
    /** Whether the entry displays a tick. Defaults to `false`. */
    checked?: boolean;
    /** Entries opened beside this one instead of running it. */
    submenu?: readonly MenuEntry[] | ((client: MenuClient) => readonly MenuEntry[]);
    /** Marks a gap between groups of entries. */
    separator?: boolean;
}

/** The window fields and actions used by the menu. */
export interface MenuClient {
    /** Whether the window still exists. */
    readonly alive: boolean;
    /** Whether the window is fullscreen. */
    fullscreen: boolean;
    /** Whether the window is tiled. */
    readonly tiled: boolean;
    /** Whether the window's content is secure. */
    readonly secure?: boolean;
    /** The window's workspace, or `null` when it has none. */
    readonly workspace: MenuWorkspace | null;
    /** Whether the window is maximised. */
    maximised: boolean;
    /** Whether the window is minimised. */
    minimised: boolean;
    /** Closes the window. */
    close(): void;
    /**
     * Starts the keyboard move mode (`gnoblin/reposition`). A `Client` has it; for a client that does not, the entry
     * is shown off.
     */
    moveWithKeys?(): void;
    /** Starts the keyboard resize mode (`gnoblin/reposition`). Optional in the same way as `moveWithKeys`. */
    resizeWithKeys?(): void;
}

/** A workspace as shown in a menu. `active` marks the workspace the user is on. */
export interface MenuWorkspace {
    /** The workspace's identifier. */
    readonly id: string;
    /** The workspace's display name. */
    readonly name: string;
    /** Whether this is the workspace the user is on. */
    readonly active: boolean;
}

/** The shell operations and window lists used by the menu. */
export interface MenuWorld {
    /** The window whose compositor node number is `node`, or `undefined` when there is none. */
    clientOf(node: number): MenuClient | undefined;
    /** Every workspace, in the order the menu lists them. */
    workspaces(): readonly MenuWorkspace[];
    /** Puts `client` on `workspace`. */
    moveToWorkspace(client: MenuClient, workspace: MenuWorkspace): void;
    /** Every window, back to front: what "always on top" keeps the pinned ones over. */
    stack(): readonly MenuClient[];
    /** Raises `client` above the other windows. */
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
    /** Takes a snapped window out of its snap. The menu calls it when the window is snapped (see `snapped`). */
    unsnap?(client: MenuClient): void;
    /** Leaves fullscreen. Without it the `fullscreen` flag is written. */
    leaveFullscreen?(client: MenuClient): void;
}
