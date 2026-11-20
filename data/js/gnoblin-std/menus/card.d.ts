import { MenuNavigator } from "gnoblin-std/menus/navigator";
import type { NavRow } from "gnoblin-std/menus/navigator";
import type { MenuClient, MenuEntry } from "gnoblin-std/menus/types";
import type { WindowMenus } from "gnoblin-std/menus/windows";
import { Box } from "gnoblin-std/ui/box";
import { Text } from "gnoblin-std/ui/text";
import type { UiKeyEvent, UiPointerEvent } from "gnoblin-std/ui/events";
/**
 * One entry: a check column, the label, and a mark when it opens a submenu. `selected` is the entry the keyboard is on.
 * A disabled entry is shown dim and the selection skips it.
 */
export declare class MenuItem extends Box implements NavRow {
    #private;
    /** The source entry used to build this row. */
    readonly entry: MenuEntry;
    /** The displayed label, or an empty string when omitted. */
    readonly label: string;
    /** Whether this row can be selected or activated. */
    readonly enabled: boolean;
    /** Whether this row displays a check mark. */
    readonly checked: boolean;
    /** Whether this row opens a submenu. */
    readonly submenu: boolean;
    /** Whether this row is selected by the pointer or keyboard. */
    selected: boolean;
    /** The text widget that displays the label. */
    readonly labelText: Text;
    constructor(entry: MenuEntry);
    /** The colour of the label and the marks. */
    ink(): string;
    /** Returns the width needed for the label, marks, gaps and edges. */
    natural(): number;
    /** Updates row colours to match its enabled and selected state. */
    refresh(): void;
    /** Selects this row when the pointer enters it. */
    onEnter(): void;
    /** Selects and activates this row on a left-button press. */
    onPress(event: UiPointerEvent): void;
}
/** A column of entries on a card. It takes the presses on its padding, so a press there is not a click away. */
export declare class MenuPanel extends Box {
    /** Visible menu rows in display order. */
    rows: MenuItem[];
    /**
     * Where the card goes along x. The first card's is the anchor; a submenu's sits beside its parent.
     */
    placeX: (panel: MenuPanel) => number;
    constructor(secure: boolean);
    /** Sizes the card to its widest row and updates its horizontal position. */
    refresh(): void;
    /** Keeps presses on the card padding from closing the menu. */
    onPress(event: UiPointerEvent): void;
}
/**
 * A menu layer that takes every press outside its cards and holds keyboard focus. Closing it also closes its cards.
 */
export declare class WindowMenu extends Box {
    #private;
    /** Client whose menu is open. */
    readonly client: MenuClient;
    /** Where the keyboard was before the menu took it: the node it goes back to. */
    readonly before: number | null;
    /** Keyboard selection across the main card and submenu. */
    readonly nav: MenuNavigator<MenuItem>;
    /** Main card containing the top-level entries. */
    readonly panel: MenuPanel;
    /** Open submenu card, or `null` when none is open. */
    sub: MenuPanel | null;
    constructor(owner: WindowMenus, client: MenuClient, entries: readonly MenuEntry[], at: {
        x: number;
        y: number;
    }, before: number | null);
    /** Runs an entry, opening its submenu or closing the menu before running its action. */
    activate(row: MenuItem): void;
    /** Opens `row`'s submenu to the right of the card, or to the left when there is no room. */
    openSubmenu(row: MenuItem): void;
    /** Closes the submenu and restores selection to the main card. */
    closeSubmenu(): void;
    /** Syncs row highlights with the keyboard selection. */
    refresh(): void;
    /** Closes the menu when a press lands outside its cards. */
    onPress(event: UiPointerEvent): void;
    /** The keys while the menu is open. Every key is taken: a menu is modal. */
    onKey(event: UiKeyEvent): void;
}
