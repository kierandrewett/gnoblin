// The stock window menu handles compositor requests, builds entries and opens a menu card.

import { WindowMenu } from "gnoblin/menus/card";
import type { MenuClient, MenuEntry, MenuWorld } from "gnoblin/menus/types";
import { onUiFrame, onUiRequest } from "gnoblin/ui/frame";
import { uiFocus, uiFocused, uiLocked, uiPointer, uiScreen } from "gnoblin/ui/session";
import { isRequestHandled } from "gnoblin/window_state";

/**
 * Returns the stock entries for a window. Move and Resize are disabled for non-floating windows or clients without
 * keyboard move and resize support.
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
 * Handles compositor menu requests, opens one menu at a time and restores keyboard focus when it closes. Set `items`
 * to change the entries or `show` to draw a different menu.
 */
export class WindowMenus {
    #world: MenuWorld | null = null;
    #current: WindowMenu | null = null;
    readonly #above = new Set<MenuClient>();

    /** Whether this instance has started answering compositor requests. */
    get started(): boolean {
        return this.#world !== null;
    }

    /** The currently open menu, or `null`. */
    get current(): WindowMenu | null {
        return this.#current;
    }

    /** Starts handling menu requests. A shell calls this through `startWindowMenu`. */
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

    /** Returns the entries for `client`. Assign this method to change the menu. */
    items(client: MenuClient): MenuEntry[] {
        if (this.#world === null) {
            return [];
        }
        return defaultMenuItems(client, this, this.#world);
    }

    /**
     * Opens `client`'s menu at `(x, y)`, or at the pointer when coordinates are omitted. Assign this method to draw a
     * different menu.
     */
    show(client: MenuClient, x?: number, y?: number): WindowMenu | null {
        const pointer = uiPointer() ?? { x: 0, y: 0 };
        return this.open(client, x ?? pointer.x, y ?? pointer.y);
    }

    /** Closes any open menu, then opens the stock menu unless the window or session cannot show it or it is empty. */
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

    /** Closes the open menu and restores keyboard focus. Returns whether a menu was open. */
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

    /** Whether `client` is pinned above unpinned windows. */
    isAbove(client: MenuClient): boolean {
        return this.#above.has(client);
    }

    /** Pins `client` above unpinned windows, or removes the pin. Pinned windows stay above unpinned ones on raise. */
    setAbove(client: MenuClient, on: boolean): void {
        if (on) {
            this.#above.add(client);
            this.#world?.raise(client);
        } else {
            this.#above.delete(client);
        }
    }

    /** Closes the menu on window removal or session lock, and keeps pinned windows above unpinned windows. */
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
     * Raises pinned windows when an unpinned window would cover one. Fullscreen windows remain above pinned windows.
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

/** The stock window menu instance, started by `startWindowMenu`. */
export const windowMenu = new WindowMenus();
