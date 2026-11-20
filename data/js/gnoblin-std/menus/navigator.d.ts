/** The fields the navigator uses to select a row. */
export interface NavRow {
    /** Text shown for the row and used for letter-key navigation. */
    readonly label: string;
    /** Whether the row can be selected and activated. */
    readonly enabled: boolean;
    /** Whether the row opens a submenu. */
    readonly submenu: boolean;
}
/**
 * Tracks keyboard selection across a menu and its open submenu. Each card keeps its selection when the submenu closes.
 */
export declare class MenuNavigator<R extends NavRow = NavRow> {
    #private;
    /** Rows in the main card. */
    readonly rows: readonly R[];
    constructor(rows: readonly R[]);
    /** Whether selection is in the main card (1) or submenu (2). */
    get depth(): 1 | 2;
    /** Rows in the open submenu, or an empty list when it is closed. */
    get subRows(): readonly R[];
    /** The rows in the active card. */
    active(): readonly R[];
    /** The selected row's index in the active card, or -1. */
    get index(): number;
    /** The row the keyboard is on, or null. */
    selected(): R | null;
    /** Returns whether `row` is selected in either card. */
    isSelected(row: R): boolean;
    /** Selects `row`; selecting a row outside the submenu closes it. */
    select(row: R): void;
    /**
     * Moves the selection by `step` over the rows that can be run, wrapping at the ends. With nothing selected a step
     * forward lands on the first row and a step back on the last.
     */
    move(step: 1 | -1): void;
    /** Selects the first enabled row. */
    home(): void;
    /** Selects the last enabled row. */
    end(): void;
    /** Selects the next enabled row whose label starts with `letter`, ignoring case. */
    jump(letter: string): void;
    /** Opens a submenu and selects its first enabled row. */
    openSubmenu(rows: readonly R[]): void;
    /** Closes the submenu and restores the parent card's selection. */
    closeSubmenu(): void;
}
/** An action produced by a menu key. */
export type MenuKeyAction = {
    do: "move";
    step: 1 | -1;
} | {
    do: "home" | "end" | "activate" | "right" | "left" | "escape";
} | {
    do: "jump";
    letter: string;
};
/** Maps a key to its menu action, or returns `null` when the menu does not use it. */
export declare function menuKeyAction(key: string, text: string): MenuKeyAction | null;
