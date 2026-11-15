// The stock window menu, ported from data/lua/gnoblin/shell/menus.lua. It owns the menu card, its keys and submenus.
// The compositor sends a `menu` request for a window: a client's `show_window_menu`, the window-menu key, or a
// titlebar's right click. Each request ends in `windowMenu.show`. Its entries come from `windowMenu.items`; assign
// either to change it. `crates/gnoblin-js/tests/stock_ui.rs` drives the menu. A script starts it with
// `startWindowMenu()`.

import { WindowMenu } from "gnoblin/menus/card";
import type { MenuClient, MenuEntry, MenuWorld } from "gnoblin/menus/types";
import { onUiFrame, onUiRequest, uiFocus, uiFocused, uiLocked, uiPointer, uiScreen } from "gnoblin/ui";
import { isRequestHandled } from "gnoblin/window_state";

/**
 * What the window menu offers a window: Restore or Maximise, Minimise, Move, Resize, Always on top, Move to workspace
 * and Close. Move and Resize are the keyboard modes (`gnoblin/reposition`): they are shown off for a window that is not
 * floating and for a client that does not offer `moveWithKeys` and `resizeWithKeys`.
 */
export function defaultMenuItems(client: MenuClient, controller: WindowMenus, world: MenuWorld): MenuEntry[] {
    const floating = !(client.maximised || client.fullscreen || client.tiled);
    const restores = client.maximised || client.fullscreen || world.snapped?.(client) === true;
    const current = client.workspace;
    return [
        restores
            ? {
                  label: "Restore",
                  run: (w) => {
                      if (w.fullscreen) {
                          if (world.leaveFullscreen === undefined) {
                              w.fullscreen = false;
                          } else {
                              world.leaveFullscreen(w);
                          }
                      } else if (w.maximised) {
                          if (world.maximise === undefined) {
                              w.maximised = false;
                          } else {
                              world.maximise(w, false);
                          }
                      } else {
                          world.unsnap?.(w);
                      }
                  },
              }
            : {
                  label: "Maximise",
                  run: (w) => {
                      if (world.maximise === undefined) {
                          w.maximised = true;
                      } else {
                          world.maximise(w, true);
                      }
                  },
              },
        {
            label: "Minimise",
            run: (w) => {
                if (world.minimise === undefined) {
                    w.minimised = true;
                } else {
                    world.minimise(w);
                }
            },
        },
        { separator: true },
        { label: "Move", enabled: floating && client.moveWithKeys !== undefined, run: (w) => w.moveWithKeys?.() },
        { label: "Resize", enabled: floating && client.resizeWithKeys !== undefined, run: (w) => w.resizeWithKeys?.() },
        {
            label: "Always on top",
            checked: controller.isAbove(client),
            run: (w) => controller.setAbove(w, !controller.isAbove(w)),
        },
        {
            label: "Move to workspace",
            enabled: !client.fullscreen,
            submenu: () =>
                world.workspaces().map((workspace) => ({
                    label: workspace.name,
                    checked: workspace === current,
                    enabled: workspace !== current,
                    run: (w: MenuClient) => world.moveToWorkspace(w, workspace),
                })),
        },
        { separator: true },
        { label: "Close", run: (w) => w.close() },
    ];
}

/**
 * The window menu of the shell: it answers the compositor's menu requests, opens one menu at a time and keeps the
 * keyboard where it was. `items` and `show` are the two places to change it: assign either
 * (`windowMenu.items = (client) => [...]`) to change what the menu offers or to draw another.
 */
export class WindowMenus {
    #world: MenuWorld | null = null;
    #current: WindowMenu | null = null;
    readonly #above = new Set<MenuClient>();

    /** Whether the stock menu answers the compositor's requests. */
    get started(): boolean {
        return this.#world !== null;
    }

    /** The menu now open, or `null`. */
    get current(): WindowMenu | null {
        return this.#current;
    }

    /** Starts answering menu requests. A shell does this once, through `startWindowMenu`. */
    start(world: MenuWorld): void {
        this.#world = world;
        onUiRequest((request) => {
            if (request.request !== "menu" || isRequestHandled(request)) {
                return;
            }
            const client = world.clientOf(request.node);
            if (client !== undefined) {
                this.show(client, request.x, request.y);
            }
        });
        onUiFrame(() => this.#watch());
    }

    /** What the menu offers `client`: a list of entries. Assign it to change the menu. */
    items(client: MenuClient): MenuEntry[] {
        if (this.#world === null) {
            return [];
        }
        return defaultMenuItems(client, this, this.#world);
    }

    /**
     * Opens the window's menu with its corner at (x, y) in the compositor's coordinates, at the pointer when none is
     * given. Every request ends here; assign it to draw a different menu.
     */
    show(client: MenuClient, x?: number, y?: number): WindowMenu | null {
        const pointer = uiPointer() ?? { x: 0, y: 0 };
        return this.open(client, x ?? pointer.x, y ?? pointer.y);
    }

    /**
     * The stock menu: closes any other, and opens nothing when the window is not showing, the session is locked, there
     * are no entries or the tree has no screen. Returns the menu, or `null` when none opened.
     */
    open(client: MenuClient, x: number, y: number): WindowMenu | null {
        this.close();
        const world = this.#world;
        if (world === null || !client.alive || client.minimised || uiLocked() || uiScreen() === null) {
            return null;
        }
        if (client.workspace !== null && !client.workspace.active) {
            return null;
        }
        const entries = this.items(client);
        if (!Array.isArray(entries) || entries.length === 0) {
            return null;
        }
        const menu = new WindowMenu(this, client, entries, { x, y }, uiFocused());
        menu.mountTop();
        this.#current = menu;
        menu.focus();
        return menu;
    }

    /** Closes the menu, if one is open, and gives the keyboard back to where it was. Returns whether one was open. */
    close(): boolean {
        const menu = this.#current;
        if (menu === null) {
            return false;
        }
        this.#current = null;
        menu.destroy();
        // The node it came from may be gone: the compositor refuses a focus on a node it has not got, so ask and go on.
        try {
            uiFocus(menu.before);
        } catch {
            uiFocus(null);
        }
        return true;
    }

    /** Whether the window is pinned over the ones that are not. */
    isAbove(client: MenuClient): boolean {
        return this.#above.has(client);
    }

    /**
     * Pins `client` above the windows that are not pinned, or lets it go. Raising keeps pinned windows over the others.
     */
    setAbove(client: MenuClient, on: boolean): void {
        if (on) {
            this.#above.add(client);
            this.#world?.raise(client);
        } else {
            this.#above.delete(client);
        }
    }

    /** The menu closes when its window goes or the session locks, and a pinned window is kept over the others. */
    #watch(): void {
        const menu = this.#current;
        if (menu !== null && (!menu.client.alive || uiLocked())) {
            this.close();
        }
        if (this.#above.size > 0 && this.#world !== null) {
            this.#keepPinned(this.#world);
        }
    }

    /**
     * Raises the pinned windows when a window that is not pinned lies over one. A fullscreen window keeps the top over
     * the pinned ones.
     */
    #keepPinned(world: MenuWorld): void {
        for (const client of [...this.#above]) {
            if (!client.alive) {
                this.#above.delete(client);
            }
        }
        const stack = world.stack();
        const firstPinned = stack.findIndex((client) => this.#above.has(client));
        const loose = stack.some((client, at) => at > firstPinned && !this.#above.has(client) && !client.fullscreen);
        if (firstPinned < 0 || !loose) {
            return;
        }
        for (const client of stack.filter((candidate) => this.#above.has(candidate))) {
            world.raise(client);
        }
        for (const client of stack.filter((candidate) => candidate.fullscreen)) {
            world.raise(client);
        }
    }
}

/** The stock window menu, started by `startWindowMenu`. */
export const windowMenu = new WindowMenus();
