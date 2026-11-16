// Shared dimensions, colours and placement helpers for the stock window menu.

import type { MenuEntry } from "gnoblin-std/menus/types";

/** The height of a menu entry, in logical pixels. */
export const ITEM_H = 28;

/** The height of a separator, in logical pixels. */
export const SEPARATOR_H = 9;

/** The padding inside a menu card, in logical pixels. */
export const PAD = 4;

/** Horizontal space between an entry edge and its label, in logical pixels. */
export const EDGE = 8;

/** The width of a check or submenu mark, in logical pixels. */
export const MARK = 14;

/** The minimum card width, in logical pixels. */
export const MIN_WIDTH = 170;

/** The kernel code for the left mouse button. */
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

/** The card height for `entries`, in logical pixels. */
export function menuHeight(entries: readonly MenuEntry[]): number {
    let height = 2 * PAD;
    for (const entry of entries) {
        height += entry.separator === true ? SEPARATOR_H : ITEM_H;
    }
    return height;
}

/** Places a card at `anchor`, clamping its top-left corner inside `area`. */
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
