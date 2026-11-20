import { WindowActions } from "gnoblin-std/frame/actions";
import type { FrameClient, FrameHooks } from "gnoblin-std/frame/client";
import type { FrameFill } from "gnoblin-std/frame/decorations";
import { DragArea } from "gnoblin-std/frame/parts/drag_area";
import { TitleText } from "gnoblin-std/frame/parts/title_text";
import { WindowButton } from "gnoblin-std/frame/parts/window_button";
import type { WindowButtonName } from "gnoblin-std/frame/parts/window_button";
import { Box } from "gnoblin-std/ui/box";
/** The three window buttons, which a style can order, leave out or give other icons. */
export type TitlebarButton = WindowButtonName;
/** The titlebar's appearance and layout. Lengths are logical pixels. */
export interface TitlebarStyle {
    /** Bar height in logical pixels. */
    height: number;
    /** Space between the bar's edge and what it holds. */
    padding: number;
    /** Space between the drag area and the buttons, and between the buttons. */
    gap: number;
    /** Corner radius in logical pixels. */
    radius: number;
    /** The fill while the window has the focus. */
    focusedFill: string;
    /** Fill when the window is not focused. */
    unfocusedFill: string;
    /** Title text size in logical pixels. */
    titleSize: number;
    /** Title text colour. */
    titleColor: string;
    /** Title font family. */
    titleFont: string;
    /** The buttons, left to right. */
    buttons: readonly TitlebarButton[];
    /** Button icon size in logical pixels. */
    buttonSize: number;
    /** Button icon colour. */
    buttonColor: string;
    /** The icon of each button, by name from the icon theme. */
    icons: Readonly<Record<TitlebarButton, string>>;
}
/** The default titlebar appearance and button order. */
export declare const defaultTitlebarStyle: Readonly<TitlebarStyle>;
/**
 * The bar. Its width follows the client's, its fill follows the focus and its title the client's title, all read
 * once a frame in `refresh`. A press on a button does what the button says; a press elsewhere on the bar drags the
 * window, and a second press in quick succession, a middle click and a right click do what the seat's `double_click`,
 * `middle_click` and `right_click` say (`toggle-maximize`, `lower`, `menu`, `minimize` or `none`).
 *
 * The close button fades the window out first (`CLOSE_FADE_MS`) and then closes it; see `WindowActions`.
 *
 * Mounted on a window's node it lies above the window, its own size; put in a frame's `top` outlet it takes the
 * outlet's room, and the frame's actions are its own (`actions`).
 */
export declare class Titlebar extends DragArea {
    #private;
    /** Window whose title and state the bar displays. */
    readonly client: FrameClient;
    /** Mutable appearance applied during `refresh()`. */
    style: TitlebarStyle;
    /** Hooks used by the frame actions. */
    readonly hooks: FrameHooks;
    /** The part of the bar that holds the title: the bar's room that no button takes. */
    readonly dragArea: Box;
    /** Title text shown by the bar. */
    readonly title: TitleText;
    /** Buttons currently present in the bar, keyed by action name. */
    readonly buttons: Map<WindowButtonName, WindowButton>;
    constructor(client: FrameClient, style?: TitlebarStyle, hooks?: FrameHooks, actions?: WindowActions);
    /** Clock used to time double clicks and close fades, in milliseconds. */
    get now(): () => number;
    /** Sets the clock used to time double clicks and close fades. */
    set now(clock: () => number);
    /** Updates the bar's size, style, title, focus fill and visibility from its window. */
    refresh(): void;
}
/**
 * Adds a `Titlebar` to the top outlet of a server frame. Returns `false` for client-drawn frames, which have no server
 * frame. The outlet height follows the style.
 */
export declare function titlebarFill(style: TitlebarStyle): FrameFill;
