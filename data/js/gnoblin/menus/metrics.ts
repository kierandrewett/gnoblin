// The look of the menu card: its sizes and colours, how tall a list of entries is, and where the card goes.

import type { MenuEntry } from "gnoblin/menus/types";

/** The height of an entry, in logical pixels. */
export const ITEM_H = 28;

/** The height of a separator. */
export const SEPARATOR_H = 9;

/** The padding inside the card. */
export const PAD = 4;

/** The space inside an entry, left and right of its label. */
export const EDGE = 8;

/** The width of the check and submenu marks. */
export const MARK = 14;

/** The narrowest a card is. */
export const MIN_WIDTH = 170;

/** The kernel number of the left mouse button. */
export const LEFT = 272;

/** The colours of the card, as the stock look draws it. */
export const MENU_COLOURS = {
    card: "#1e1e2e",
    hot: "#89b4fa",
    text: "#cdd6f4",
    hotText: "#11111b",
    off: "#6c7086",
    line: "#45475a",
} as const;

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
