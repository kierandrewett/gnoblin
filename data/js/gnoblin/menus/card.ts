// A menu card and its entry widgets. Submenus use their own cards, so the menu and widgets share this module.

import {
    EDGE,
    ITEM_H,
    LEFT,
    MARK,
    MENU_COLOURS,
    menuHeight,
    MIN_WIDTH,
    PAD,
    placeCard,
    SEPARATOR_H,
} from "gnoblin/menus/metrics";
import { menuKeyAction, MenuNavigator } from "gnoblin/menus/navigator";
import type { NavRow } from "gnoblin/menus/navigator";
import type { MenuClient, MenuEntry } from "gnoblin/menus/types";
import type { WindowMenus } from "gnoblin/menus/windows";
import { Box } from "gnoblin/ui/box";
import { uiScreen } from "gnoblin/ui/session";
import { Text } from "gnoblin/ui/text";
import type { UiKeyEvent, UiPointerEvent } from "gnoblin/ui/events";
import type { PaintOp } from "gnoblin/ui/paint";

/** Draws a check or submenu arrow from lines, without relying on a font glyph. */
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
    selected = false;
    /** The text widget that displays the label. */
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

    /** Returns the width needed for the label, marks, gaps and edges. */
    natural(): number {
        return this.labelText.metrics().width + 2 * EDGE + 2 * MARK + 4 * 4;
    }

    /** Updates row colours to match its enabled and selected state. */
    override refresh(): void {
        const ink = this.ink();
        this.fill = this.selected && this.enabled ? MENU_COLOURS.hot : "#00000000";
        this.labelText.color = ink;
        this.#check.color = ink;
        this.#arrow.color = ink;
    }

    /** Selects this row when the pointer enters it. */
    override onEnter(): void {
        const menu = this.#menu();
        if (menu !== null && this.enabled) {
            menu.nav.select(this);
        }
    }

    /** Selects and activates this row on a left-button press. */
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
    /** Visible menu rows in display order. */
    rows: MenuItem[] = [];
    /**
     * Where the card goes along x. The first card's is the anchor; a submenu's sits beside its parent.
     */
    placeX: (panel: MenuPanel) => number = () => 0;

    constructor(secure: boolean) {
        super({ direction: "column", radius: 10, fill: MENU_COLOURS.card, padding: PAD, gap: 0, secure });
    }

    /** Sizes the card to its widest row and updates its horizontal position. */
    override refresh(): void {
        let widest = MIN_WIDTH;
        for (const row of this.rows) {
            widest = Math.max(widest, row.natural() + 2 * PAD);
        }
        this.width = widest;
        this.x = this.placeX(this);
    }

    /** Keeps presses on the card padding from closing the menu. */
    override onPress(event: UiPointerEvent): void {
        event.stop();
    }
}

/** Builds a card from entries and their placement. */
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
 * A menu layer that takes every press outside its cards and holds keyboard focus. Closing it also closes its cards.
 */
export class WindowMenu extends Box {
    /** Client whose menu is open. */
    readonly client: MenuClient;
    /** Where the keyboard was before the menu took it: the node it goes back to. */
    readonly before: number | null;
    /** Keyboard selection across the main card and submenu. */
    readonly nav: MenuNavigator<MenuItem>;
    /** Main card containing the top-level entries. */
    readonly panel: MenuPanel;
    /** Open submenu card, or `null` when none is open. */
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

    /** Runs an entry, opening its submenu or closing the menu before running its action. */
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

    /** Opens `row`'s submenu to the right of the card, or to the left when there is no room. */
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

    /** Closes the submenu and restores selection to the main card. */
    closeSubmenu(): void {
        if (this.nav.depth !== 2) {
            return;
        }
        this.sub?.destroy();
        this.sub = null;
        this.nav.closeSubmenu();
    }

    /** Syncs row highlights with the keyboard selection. */
    override refresh(): void {
        for (const row of [...this.panel.rows, ...(this.sub?.rows ?? [])]) {
            row.selected = this.nav.isSelected(row);
        }
    }

    /** Closes the menu when a press lands outside its cards. */
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
