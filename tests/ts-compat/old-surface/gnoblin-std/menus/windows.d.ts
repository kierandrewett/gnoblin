import { WindowMenu } from "gnoblin-std/menus/card";
import type { MenuClient, MenuEntry, MenuWorld } from "gnoblin-std/menus/types";
/**
 * Returns the stock entries for a window. Move and Resize are disabled for non-floating windows or clients without
 * keyboard move and resize support.
 */
export declare function defaultMenuItems(client: MenuClient, controller: WindowMenus, world: MenuWorld): MenuEntry[];
/**
 * Handles compositor menu requests, opens one menu at a time and restores keyboard focus when it closes. Set `items`
 * to change the entries or `show` to draw a different menu.
 */
export declare class WindowMenus {
    #private;
    /** Whether this instance has started answering compositor requests. */
    get started(): boolean;
    /** The currently open menu, or `null`. */
    get current(): WindowMenu | null;
    /** Starts handling menu requests. A shell calls this through `startWindowMenu`. */
    start(world: MenuWorld): void;
    /** Returns the entries for `client`. Assign this method to change the menu. */
    items(client: MenuClient): MenuEntry[];
    /**
     * Opens `client`'s menu at `(x, y)`, or at the pointer when coordinates are omitted. Assign this method to draw a
     * different menu.
     */
    show(client: MenuClient, x?: number, y?: number): WindowMenu | null;
    /** Closes any open menu, then opens the stock menu unless the window or session cannot show it or it is empty. */
    open(client: MenuClient, x: number, y: number): WindowMenu | null;
    /** Closes the open menu and restores keyboard focus. Returns whether a menu was open. */
    close(): boolean;
    /** Whether `client` is pinned above unpinned windows. */
    isAbove(client: MenuClient): boolean;
    /** Pins `client` above unpinned windows, or removes the pin. Pinned windows stay above unpinned ones on raise. */
    setAbove(client: MenuClient, on: boolean): void;
}
/** The stock window menu instance, started by `startWindowMenu`. */
export declare const windowMenu: WindowMenus;
