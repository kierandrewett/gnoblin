import type { MenuEntry } from "gnoblin-std/menus/types";
/** The height of a menu entry, in logical pixels. */
export declare const ITEM_H = 28;
/** The height of a separator, in logical pixels. */
export declare const SEPARATOR_H = 9;
/** The padding inside a menu card, in logical pixels. */
export declare const PAD = 4;
/** Horizontal space between an entry edge and its label, in logical pixels. */
export declare const EDGE = 8;
/** The width of a check or submenu mark, in logical pixels. */
export declare const MARK = 14;
/** The minimum card width, in logical pixels. */
export declare const MIN_WIDTH = 170;
/** The kernel code for the left mouse button. */
export declare const LEFT = 272;
/** The colours of the card, as the stock look draws it. */
export declare const MENU_COLOURS: {
    readonly card: "#1e1e2e";
    readonly hot: "#89b4fa";
    readonly text: "#cdd6f4";
    readonly hotText: "#11111b";
    readonly off: "#6c7086";
    readonly line: "#45475a";
};
/** The card height for `entries`, in logical pixels. */
export declare function menuHeight(entries: readonly MenuEntry[]): number;
/** Places a card at `anchor`, clamping its top-left corner inside `area`. */
export declare function placeCard(anchor: {
    x: number;
    y: number;
}, size: {
    width: number;
    height: number;
}, area: {
    width: number;
    height: number;
}): {
    x: number;
    y: number;
};
