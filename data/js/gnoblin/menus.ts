// The stock window menu, ported from data/lua/gnoblin/shell/menus.lua. It owns the menu card, its keys and submenus.
// The compositor sends a `menu` request for a window: a client's `show_window_menu`, the window-menu key, or a
// titlebar's right click. Each request ends in `windowMenu.show`. Its entries come from `windowMenu.items`; assign
// either to change it. `crates/gnoblin-js/tests/stock_ui.rs` drives the menu. Turn it off with
// `defineConfig({ menus: false })`.

import { Box, Text, onUiFrame, onUiRequest, uiFocus, uiFocused, uiLocked, uiPointer, uiScreen } from "gnoblin/ui";
import type { PaintOp, UiKeyEvent, UiPointerEvent } from "gnoblin/ui";
import { isRequestHandled } from "gnoblin/window_state";

const ITEM_H = 28;
const SEPARATOR_H = 9;
const PAD = 4;
/** The space inside an entry, left and right of its label. */
const EDGE = 8;
/** The width of the check and submenu marks. */
const MARK = 14;
const MIN_WIDTH = 170;
const LEFT = 272;

export const MENU_COLOURS = {
    card: "#1e1e2e",
    hot: "#89b4fa",
    text: "#cdd6f4",
    hotText: "#11111b",
    off: "#6c7086",
    line: "#45475a",
} as const;

// ---- the entries -------------------------------------------------------------------------------------------------

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

/** The height the card for `entries` takes. */
export function menuHeight(entries: readonly MenuEntry[]): number {
    let height = 2 * PAD;
    for (const entry of entries) {
        height += entry.separator === true ? SEPARATOR_H : ITEM_H;
    }
    return height;
}

/** Where a card of `size` goes when asked for at `anchor`: its corner at the anchor, moved back inside `area`. */
export function placeCard(
    anchor: { x: number; y: number },
    size: { width: number; height: number },
    area: { width: number; height: number },
): { x: number; y: number } {
    return {
        x: Math.max(0, Math.min(anchor.x, area.width - size.width)),
        y: Math.max(0, Math.min(anchor.y, area.height - size.height)),
    };
}

// ---- keyboard navigation (no drawing) ----------------------------------------------------------------------------

/** What the navigator needs of a row. */
export interface NavRow {
    readonly label: string;
    readonly enabled: boolean;
    readonly submenu: boolean;
}

/**
 * Which row of a menu the keyboard is on, over the rows that can be run: a card of rows and, once one is opened, a
 * submenu's. Selection is kept per card, so closing the submenu leaves its parent row selected.
 */
export class MenuNavigator<R extends NavRow = NavRow> {
    readonly rows: readonly R[];
    #sub: readonly R[] = [];
    #depth: 1 | 2 = 1;
    #picked: [number, number] = [-1, -1];

    constructor(rows: readonly R[]) {
        this.rows = rows;
    }

    get depth(): 1 | 2 {
        return this.#depth;
    }

    get subRows(): readonly R[] {
        return this.#sub;
    }

    /** The rows the keyboard is in. */
    active(): readonly R[] {
        return this.#depth === 2 ? this.#sub : this.rows;
    }

    /** The place of the selection among the active rows, or -1. */
    get index(): number {
        return this.#picked[this.#depth - 1];
    }

    /** The row the keyboard is on, or null. */
    selected(): R | null {
        return this.active()[this.index] ?? null;
    }

    /** Whether `row` is the one picked in its card. */
    isSelected(row: R): boolean {
        return this.rows[this.#picked[0]] === row || this.#sub[this.#picked[1]] === row;
    }

    /** Puts the selection on `row`. The pointer going back to the first card closes the submenu it left. */
    select(row: R): void {
        if (this.#depth === 2 && !this.#sub.includes(row)) {
            this.closeSubmenu();
        }
        const at = this.active().indexOf(row);
        if (at >= 0) {
            this.#picked[this.#depth - 1] = at;
        }
    }

    /**
     * Moves the selection by `step` over the rows that can be run, wrapping at the ends. With nothing selected a step
     * forward lands on the first row and a step back on the last.
     */
    move(step: 1 | -1): void {
        const rows = this.active();
        const count = rows.length;
        let at = this.index;
        if (at < 0) {
            at = step > 0 ? -1 : count;
        }
        for (let tried = 0; tried < count; tried++) {
            at = (((at + step) % count) + count) % count;
            if (rows[at].enabled) {
                this.#picked[this.#depth - 1] = at;
                return;
            }
        }
    }

    home(): void {
        this.#picked[this.#depth - 1] = -1;
        this.move(1);
    }

    end(): void {
        this.#picked[this.#depth - 1] = -1;
        this.move(-1);
    }

    /** The first row after the selection, wrapping, whose label starts with `letter` (compared without case). */
    jump(letter: string): void {
        const rows = this.active();
        const count = rows.length;
        for (let step = 1; step <= count; step++) {
            const at = (this.index + step + count) % count;
            if (rows[at].enabled && rows[at].label.slice(0, 1).toLowerCase() === letter.toLowerCase()) {
                this.#picked[this.#depth - 1] = at;
                return;
            }
        }
    }

    /** Opens a submenu of `rows` and puts the selection on its first row that can be run. */
    openSubmenu(rows: readonly R[]): void {
        this.closeSubmenu();
        this.#sub = rows;
        this.#depth = 2;
        this.#picked[1] = -1;
        this.move(1);
    }

    closeSubmenu(): void {
        if (this.#depth !== 2) {
            return;
        }
        this.#sub = [];
        this.#depth = 1;
        this.#picked[1] = -1;
    }
}

/** What a key does to a menu. */
export type MenuKeyAction =
    | { do: "move"; step: 1 | -1 }
    | { do: "home" | "end" | "activate" | "right" | "left" | "escape" }
    | { do: "jump"; letter: string };

/** The action for a key of the keyboard while a menu is open, or `null` for a key the menu does not use. */
export function menuKeyAction(key: string, text: string): MenuKeyAction | null {
    switch (key) {
        case "Escape":
            return { do: "escape" };
        case "Down":
        case "Tab":
            return { do: "move", step: 1 };
        case "Up":
        case "ISO_Left_Tab":
            return { do: "move", step: -1 };
        case "Home":
            return { do: "home" };
        case "End":
            return { do: "end" };
        case "Return":
        case "KP_Enter":
        case "space":
            return { do: "activate" };
        case "Right":
            return { do: "right" };
        case "Left":
            return { do: "left" };
        default:
            return [...text].length === 1 && /\S/.test(text) ? { do: "jump", letter: text } : null;
    }
}

// ---- what the menu is drawn from ---------------------------------------------------------------------------------

/** A tick or a chevron, drawn from lines so it never depends on the font having the glyph. */
class MenuMark extends Box {
    kind: "" | "check" | "arrow" = "";
    color: string = MENU_COLOURS.text;

    constructor() {
        super({ width: MARK });
    }

    override draw(): PaintOp[] {
        const top = Math.floor((this.height - 10) / 2);
        const color = this.color;
        if (this.kind === "check") {
            return [
                { op: "line", x1: 2, y1: top + 5, x2: 5, y2: top + 9, width: 1.5, color },
                { op: "line", x1: 5, y1: top + 9, x2: 12, y2: top + 1, width: 1.5, color },
            ];
        }
        if (this.kind === "arrow") {
            return [
                { op: "line", x1: 4, y1: top, x2: 9, y2: top + 5, width: 1.5, color },
                { op: "line", x1: 9, y1: top + 5, x2: 4, y2: top + 10, width: 1.5, color },
            ];
        }
        return [];
    }
}

/** The rule between groups of entries. */
class MenuSeparator extends Box {
    constructor() {
        super({ height: SEPARATOR_H });
    }

    override draw(): PaintOp[] {
        // Before the card lays its entries out a separator is as wide as nothing: a rectangle is never negative.
        return [
            {
                op: "rect",
                x: EDGE,
                y: Math.floor(this.height / 2),
                width: Math.max(0, this.width - 2 * EDGE),
                height: 1,
                fill: MENU_COLOURS.line,
            },
        ];
    }
}

/**
 * One entry: a check column, the label, and a mark when it opens a submenu. `selected` is the entry the keyboard is on.
 * A disabled entry is shown dim and the selection skips it.
 */
export class MenuItem extends Box implements NavRow {
    readonly entry: MenuEntry;
    readonly label: string;
    readonly enabled: boolean;
    readonly checked: boolean;
    readonly submenu: boolean;
    selected = false;
    readonly labelText: Text;
    readonly #check = new MenuMark();
    readonly #arrow = new MenuMark();

    constructor(entry: MenuEntry) {
        super({ direction: "row", gap: 4, height: ITEM_H, radius: 6 });
        this.entry = entry;
        this.label = entry.label ?? "";
        this.enabled = entry.enabled !== false;
        this.checked = entry.checked === true;
        this.submenu = entry.submenu !== undefined;
        this.labelText = new Text({ text: this.label, size: 14, grow: 1 });
        this.#check.kind = this.checked ? "check" : "";
        this.#arrow.kind = this.submenu ? "arrow" : "";
        this.add(new Box({ width: EDGE }));
        this.add(this.#check);
        this.add(this.labelText);
        this.add(this.#arrow);
        this.add(new Box({ width: EDGE }));
    }

    /** The colour of the label and the marks. */
    ink(): string {
        if (!this.enabled) {
            return MENU_COLOURS.off;
        }
        return this.selected ? MENU_COLOURS.hotText : MENU_COLOURS.text;
    }

    /** How wide the entry wants to be: its label, the marks, the gaps and the edges. */
    natural(): number {
        return this.labelText.metrics().width + 2 * EDGE + 2 * MARK + 4 * 4;
    }

    override refresh(): void {
        const ink = this.ink();
        this.fill = this.selected && this.enabled ? MENU_COLOURS.hot : "#00000000";
        this.labelText.color = ink;
        this.#check.color = ink;
        this.#arrow.color = ink;
    }

    override onEnter(): void {
        const menu = this.#menu();
        if (menu !== null && this.enabled) {
            menu.nav.select(this);
        }
    }

    override onPress(event: UiPointerEvent): void {
        event.stop();
        const menu = this.#menu();
        if (menu !== null && this.enabled && event.button === LEFT) {
            menu.nav.select(this);
            menu.activate(this);
        }
    }

    #menu(): WindowMenu | null {
        for (let at = this.parent; at !== null; at = at.parent) {
            if (at instanceof WindowMenu) {
                return at;
            }
        }
        return null;
    }
}

/** A column of entries on a card. It takes the presses on its padding, so a press there is not a click away. */
export class MenuPanel extends Box {
    rows: MenuItem[] = [];
    /**
     * Where the card goes along x. The first card's is the anchor; a submenu's sits beside its parent.
     */
    placeX: (panel: MenuPanel) => number = () => 0;

    constructor(secure: boolean) {
        super({ direction: "column", radius: 10, fill: MENU_COLOURS.card, padding: PAD, gap: 0, secure });
    }

    override refresh(): void {
        let widest = MIN_WIDTH;
        for (const row of this.rows) {
            widest = Math.max(widest, row.natural() + 2 * PAD);
        }
        this.width = widest;
        this.x = this.placeX(this);
    }

    override onPress(event: UiPointerEvent): void {
        event.stop();
    }
}

/** The card for `entries`, with the extra properties `place` gives it, and the rows that can be selected, in order. */
function buildPanel(
    menu: WindowMenu,
    entries: readonly MenuEntry[],
    place: { y: number; x: (panel: MenuPanel) => number },
): MenuPanel {
    const panel = new MenuPanel(menu.client.secure === true);
    panel.height = menuHeight(entries);
    panel.y = place.y;
    panel.placeX = place.x;
    menu.add(panel);
    for (const entry of entries) {
        if (entry.separator === true) {
            panel.add(new MenuSeparator());
        } else {
            const row = new MenuItem(entry);
            panel.rows.push(row);
            panel.add(row);
        }
    }
    return panel;
}

/**
 * The whole menu: a transparent layer over the screen that takes every press, so one outside the cards is a click away,
 * and that holds the keyboard focus. The cards are its children, so closing it closes them.
 */
export class WindowMenu extends Box {
    readonly client: MenuClient;
    /** Where the keyboard was before the menu took it: the node it goes back to. */
    readonly before: number | null;
    readonly nav: MenuNavigator<MenuItem>;
    readonly panel: MenuPanel;
    sub: MenuPanel | null = null;
    readonly #owner: WindowMenus;

    constructor(
        owner: WindowMenus,
        client: MenuClient,
        entries: readonly MenuEntry[],
        at: { x: number; y: number },
        before: number | null,
    ) {
        super({ x: 0, y: 0, width: uiScreen()?.width ?? 0, height: uiScreen()?.height ?? 0 });
        this.#owner = owner;
        this.client = client;
        this.before = before;
        const height = menuHeight(entries);
        this.panel = buildPanel(this, entries, {
            y: placeCard(at, { width: 0, height }, this).y,
            x: (panel) => placeCard(at, panel, this).x,
        });
        this.nav = new MenuNavigator(this.panel.rows);
    }

    /** Runs a row. A submenu opens; anything else closes the menu first and then runs, so what it does starts clean. */
    activate(row: MenuItem): void {
        if (!row.enabled) {
            return;
        }
        if (row.submenu) {
            this.openSubmenu(row);
            return;
        }
        const { client } = this;
        const run = row.entry.run;
        this.#owner.close();
        run?.(client);
    }

    /** Opens the submenu of `row` to the right of the card, or to the left when there is no room. */
    openSubmenu(row: MenuItem): void {
        const spec = row.entry.submenu;
        const entries = typeof spec === "function" ? spec(this.client) : spec;
        if (entries === undefined || entries.length === 0) {
            return;
        }
        this.closeSubmenu();
        const { panel } = this;
        const at = row.y + panel.y;
        const sub = buildPanel(this, entries, {
            y: Math.max(0, Math.min(at - PAD, this.height - menuHeight(entries))),
            x: (card) => {
                const right = panel.x + panel.width - 2;
                return right + card.width > this.width ? Math.max(0, panel.x - card.width + 2) : right;
            },
        });
        this.sub = sub;
        this.nav.openSubmenu(sub.rows);
    }

    closeSubmenu(): void {
        if (this.nav.depth !== 2) {
            return;
        }
        this.sub?.destroy();
        this.sub = null;
        this.nav.closeSubmenu();
    }

    override refresh(): void {
        for (const row of [...this.panel.rows, ...(this.sub?.rows ?? [])]) {
            row.selected = this.nav.isSelected(row);
        }
    }

    override onPress(event: UiPointerEvent): void {
        event.stop();
        this.#owner.close();
    }

    /** The keys while the menu is open. Every key is taken: a menu is modal. */
    override onKey(event: UiKeyEvent): void {
        if (!event.pressed) {
            return;
        }
        const action = menuKeyAction(event.key, event.text);
        const { nav } = this;
        switch (action?.do) {
            case "escape":
                if (nav.depth === 2) {
                    this.closeSubmenu();
                } else {
                    this.#owner.close();
                }
                break;
            case "move":
                nav.move(action.step);
                break;
            case "home":
                nav.home();
                break;
            case "end":
                nav.end();
                break;
            case "activate": {
                const row = nav.selected();
                if (row !== null) {
                    this.activate(row);
                }
                break;
            }
            case "right": {
                const row = nav.selected();
                if (nav.depth === 1 && row !== null && row.submenu) {
                    this.openSubmenu(row);
                }
                break;
            }
            case "left":
                this.closeSubmenu();
                break;
            case "jump":
                nav.jump(action.letter);
                break;
            default:
        }
        event.stop();
    }
}

// ---- the default entries -----------------------------------------------------------------------------------------

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

// ---- the controller ----------------------------------------------------------------------------------------------

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

    /** Starts answering menu requests. A shell does this once; `defineConfig({ menus: false })` never does. */
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

/** The stock window menu. Started by `defineConfig` unless the config says `menus: false`. */
export const windowMenu = new WindowMenus();
